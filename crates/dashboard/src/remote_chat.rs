//! Home-device worker: stay connected to account-service and run prompts locally.

use crate::api::state::AppState;
use crate::schema::CreateSessionRequest;
use anycode_llm::{account_api_url, read_cloud_access_token};
use anycode_setup::read_cloud_session;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};

type WsWrite = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn spawn_remote_chat_worker(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(e) = run_once(state.clone()).await {
                tracing::debug!(error = %e, "remote chat worker idle/retry");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

fn ws_url(token: &str, device_id: &str) -> String {
    let http = account_api_url().trim_end_matches('/').to_string();
    let ws = if let Some(rest) = http.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = http.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("wss://{http}")
    };
    format!(
        "{ws}/api/v1/remote-chat/ws?device_id={}&access_token={}",
        encode_query(device_id),
        encode_query(token)
    )
}

async fn run_once(state: AppState) -> anyhow::Result<()> {
    let token = read_cloud_access_token().ok_or_else(|| anyhow::anyhow!("no cloud token"))?;
    let device_id = read_cloud_session()
        .and_then(|s| s.device_id)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("no device_id"))?;
    let url = ws_url(&token, &device_id);
    let (ws, _) = connect_async(&url).await?;
    let (write, mut read) = ws.split();
    tracing::info!(%device_id, "remote chat worker connected");

    let mut session_map: HashMap<String, String> = HashMap::new();
    let mut chat_rx = state.events.subscribe_chat();
    let write = Arc::new(tokio::sync::Mutex::new(write));
    let write_events = Arc::clone(&write);
    let write_ping = Arc::clone(&write);
    let map_for_fwd = Arc::new(tokio::sync::Mutex::new(HashMap::<String, String>::new()));
    let map_fwd = Arc::clone(&map_for_fwd);

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(25));
        loop {
            interval.tick().await;
            let mut w = write_ping.lock().await;
            if w.send(Message::Ping(Vec::new())).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        while let Ok(evt) = chat_rx.recv().await {
            let map = map_fwd.lock().await;
            let Some((conversation_id, _)) = map.iter().find(|(_, sid)| *sid == &evt.session_id)
            else {
                continue;
            };
            let msg = json!({
                "type": "event",
                "conversation_id": conversation_id,
                "kind": evt.kind,
                "payload": evt,
            });
            let mut w = write_events.lock().await;
            if w.send(Message::Text(msg.to_string())).await.is_err() {
                break;
            }
        }
    });

    while let Some(frame) = read.next().await {
        let Message::Text(text) = frame? else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "prompt" => {
                if let Err(e) =
                    handle_prompt(&state, &write, &map_for_fwd, &mut session_map, v).await
                {
                    tracing::warn!(error = %e, "remote prompt failed");
                }
            }
            "cancel" => {
                if let Some(sid) = v
                    .get("local_session_id")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                {
                    let _ = state.chat_runtime.cancel(sid).await;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn handle_prompt(
    state: &AppState,
    write: &Arc<tokio::sync::Mutex<WsWrite>>,
    map_for_fwd: &Arc<tokio::sync::Mutex<HashMap<String, String>>>,
    session_map: &mut HashMap<String, String>,
    v: Value,
) -> anyhow::Result<()> {
    let conversation_id = v
        .get("conversation_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing conversation_id"))?
        .to_string();
    let prompt = v
        .get("prompt")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if prompt.is_empty() {
        anyhow::bail!("empty prompt");
    }

    let projects = state.db.list_projects().await?;
    let project = projects
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no local project"))?;
    let root = std::path::PathBuf::from(&project.root_path);

    let existing = v
        .get("local_session_id")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let session = if let Some(sid) = existing {
        state
            .db
            .get_session(&sid)
            .await?
            .ok_or_else(|| anyhow::anyhow!("local session missing"))?
    } else {
        let title: String = prompt.chars().take(120).collect();
        state
            .db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "repl".into(),
                task_id: None,
                title,
                prompt_preview: Some(prompt.chars().take(240).collect()),
                agent_type: None,
                model: None,
                metadata_json: None,
            })
            .await?
    };

    session_map.insert(conversation_id.clone(), session.id.clone());
    map_for_fwd
        .lock()
        .await
        .insert(conversation_id.clone(), session.id.clone());

    {
        let mut w = write.lock().await;
        w.send(Message::Text(
            json!({
                "type": "bind",
                "conversation_id": conversation_id,
                "local_session_id": session.id,
                "project_name": project.name,
            })
            .to_string(),
        ))
        .await?;
        w.send(Message::Text(
            json!({
                "type": "status",
                "conversation_id": conversation_id,
                "status": "running",
            })
            .to_string(),
        ))
        .await?;
    }

    let result = crate::control::web_chat_dispatch::dispatch_web_chat_prompt(
        state,
        &project.id,
        &session.id,
        &root,
        None,
        &prompt,
        &prompt,
        None,
        None,
        None,
        false,
        "remote_chat",
        None,
    )
    .await;

    let status = if result.is_ok() { "idle" } else { "failed" };
    let mut w = write.lock().await;
    w.send(Message::Text(
        json!({
            "type": "status",
            "conversation_id": conversation_id,
            "status": status,
        })
        .to_string(),
    ))
    .await?;
    if let Err((_, msg)) = result {
        w.send(Message::Text(
            json!({
                "type": "event",
                "conversation_id": conversation_id,
                "kind": "session_error",
                "payload": { "text": msg },
            })
            .to_string(),
        ))
        .await?;
    }
    Ok(())
}
