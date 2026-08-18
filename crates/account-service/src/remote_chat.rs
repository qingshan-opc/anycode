//! Same-user remote command plane: cloud stores conversations; execution stays on the home device.

use crate::api::{bearer_from_request, json_error, AppState, AuthContext};
use crate::db::AccountDb;
use crate::models::AuthUser;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

const PRESENCE_ONLINE_SECS: i64 = 90;

#[derive(Clone, Default)]
pub struct RemoteChatHub {
    devices: Arc<Mutex<HashMap<String, mpsc::UnboundedSender<Value>>>>,
}

impl RemoteChatHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn is_online(&self, device_id: &str) -> bool {
        self.devices.lock().await.contains_key(device_id)
    }

    pub async fn register(&self, device_id: String) -> mpsc::UnboundedReceiver<Value> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.devices.lock().await.insert(device_id, tx);
        rx
    }

    pub async fn unregister(&self, device_id: &str) {
        self.devices.lock().await.remove(device_id);
    }

    pub async fn send_to_device(&self, device_id: &str, msg: Value) -> bool {
        let map = self.devices.lock().await;
        map.get(device_id).is_some_and(|tx| tx.send(msg).is_ok())
    }
}

#[derive(Debug, Serialize)]
pub struct MineDevice {
    pub id: String,
    pub device_name: String,
    pub platform: String,
    pub last_seen_at: String,
    pub online: bool,
    pub this_device: bool,
}

#[derive(Debug, Serialize)]
pub struct CloudConversation {
    pub id: String,
    pub home_device_id: String,
    pub local_session_id: Option<String>,
    pub title: String,
    pub project_name: Option<String>,
    pub status: String,
    pub last_event_seq: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct CloudTurnEvent {
    pub seq: i64,
    pub kind: String,
    pub payload: Value,
    pub created_at: String,
}

fn row_conversation(r: &sqlx::mysql::MySqlRow) -> CloudConversation {
    let created: chrono::DateTime<Utc> = r.get("created_at");
    let updated: chrono::DateTime<Utc> = r.get("updated_at");
    CloudConversation {
        id: r.get("id"),
        home_device_id: r.get("home_device_id"),
        local_session_id: r.get("local_session_id"),
        title: r.get("title"),
        project_name: r.get("project_name"),
        status: r.get("status"),
        last_event_seq: r.get("last_event_seq"),
        created_at: created.to_rfc3339(),
        updated_at: updated.to_rfc3339(),
    }
}

pub async fn list_mine_devices(
    db: &AccountDb,
    user: &AuthUser,
    hub: &RemoteChatHub,
    this_device_id: Option<&str>,
) -> anyhow::Result<Vec<MineDevice>> {
    let cutoff = Utc::now() - Duration::seconds(PRESENCE_ONLINE_SECS);
    let rows = sqlx::query(
        r#"
        SELECT ld.id, ld.device_name, ld.platform, ld.last_seen_at, p.last_heartbeat_at
        FROM linked_devices ld
        LEFT JOIN a2a_agent_presence p ON p.device_id = ld.id
        WHERE ld.user_id = ? AND ld.revoked_at IS NULL
        ORDER BY ld.last_seen_at DESC
        "#,
    )
    .bind(&user.id)
    .fetch_all(db.pool())
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let id: String = r.get("id");
        let last: chrono::DateTime<Utc> = r.get("last_seen_at");
        let heartbeat_at: Option<chrono::DateTime<Utc>> = r.try_get("last_heartbeat_at").ok();
        let heartbeat_online = heartbeat_at.is_some_and(|at| at >= cutoff);
        let ws_online = hub.is_online(&id).await;
        out.push(MineDevice {
            id: id.clone(),
            device_name: r.get("device_name"),
            platform: r.get("platform"),
            last_seen_at: last.to_rfc3339(),
            online: ws_online || heartbeat_online,
            this_device: this_device_id == Some(id.as_str()),
        });
    }
    Ok(out)
}

async fn verify_device_owner(
    db: &AccountDb,
    user_id: &str,
    device_id: &str,
) -> anyhow::Result<bool> {
    crate::a2a::store::verify_device_owner(db, user_id, device_id).await
}

async fn create_conversation(
    db: &AccountDb,
    user_id: &str,
    home_device_id: &str,
    title: &str,
) -> anyhow::Result<CloudConversation> {
    let id = format!("cconv_{}", Uuid::new_v4());
    sqlx::query(
        r#"
        INSERT INTO cloud_conversations (id, user_id, home_device_id, title, status)
        VALUES (?, ?, ?, ?, 'idle')
        "#,
    )
    .bind(&id)
    .bind(user_id)
    .bind(home_device_id)
    .bind(title)
    .execute(db.pool())
    .await?;
    get_conversation(db, user_id, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("conversation missing after insert"))
}

async fn get_conversation(
    db: &AccountDb,
    user_id: &str,
    id: &str,
) -> anyhow::Result<Option<CloudConversation>> {
    let row = sqlx::query(
        r#"
        SELECT id, home_device_id, local_session_id, title, project_name, status,
               last_event_seq, created_at, updated_at
        FROM cloud_conversations WHERE id = ? AND user_id = ?
        "#,
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(db.pool())
    .await?;
    Ok(row.as_ref().map(row_conversation))
}

async fn list_conversations(
    db: &AccountDb,
    user_id: &str,
    home_device_id: &str,
) -> anyhow::Result<Vec<CloudConversation>> {
    let rows = sqlx::query(
        r#"
        SELECT id, home_device_id, local_session_id, title, project_name, status,
               last_event_seq, created_at, updated_at
        FROM cloud_conversations
        WHERE user_id = ? AND home_device_id = ?
        ORDER BY updated_at DESC
        LIMIT 200
        "#,
    )
    .bind(user_id)
    .bind(home_device_id)
    .fetch_all(db.pool())
    .await?;
    Ok(rows.iter().map(row_conversation).collect())
}

async fn append_event(
    db: &AccountDb,
    conversation_id: &str,
    kind: &str,
    payload: &Value,
) -> anyhow::Result<i64> {
    let mut tx = db.pool().begin().await?;
    let seq: i64 = sqlx::query_scalar(
        "SELECT last_event_seq FROM cloud_conversations WHERE id = ? FOR UPDATE",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await?;
    let next = seq + 1;
    sqlx::query(
        "INSERT INTO cloud_turn_events (conversation_id, seq, kind, payload_json) VALUES (?, ?, ?, ?)",
    )
    .bind(conversation_id)
    .bind(next)
    .bind(kind)
    .bind(payload.to_string())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE cloud_conversations SET last_event_seq = ?, updated_at = NOW(3) WHERE id = ?",
    )
    .bind(next)
    .bind(conversation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(next)
}

async fn list_events(
    db: &AccountDb,
    conversation_id: &str,
    after: i64,
) -> anyhow::Result<Vec<CloudTurnEvent>> {
    let rows = sqlx::query(
        r#"
        SELECT seq, kind, payload_json, created_at
        FROM cloud_turn_events
        WHERE conversation_id = ? AND seq > ?
        ORDER BY seq ASC
        LIMIT 500
        "#,
    )
    .bind(conversation_id)
    .bind(after)
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let created: chrono::DateTime<Utc> = r.get("created_at");
            let raw: String = r.get("payload_json");
            CloudTurnEvent {
                seq: r.get("seq"),
                kind: r.get("kind"),
                payload: serde_json::from_str(&raw).unwrap_or(Value::Null),
                created_at: created.to_rfc3339(),
            }
        })
        .collect())
}

#[derive(Deserialize)]
pub struct MineQ {
    pub device_id: Option<String>,
}

pub async fn devices_mine(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(q): Query<MineQ>,
) -> impl IntoResponse {
    match list_mine_devices(
        &state.db,
        &ctx.user,
        &state.remote_chat,
        q.device_id.as_deref(),
    )
    .await
    {
        Ok(devices) => Json(json!({ "devices": devices })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct ListConvQ {
    pub home_device_id: String,
}

pub async fn list_remote_conversations(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(q): Query<ListConvQ>,
) -> impl IntoResponse {
    match verify_device_owner(&state.db, &ctx.user.id, &q.home_device_id).await {
        Ok(true) => {}
        Ok(false) => {
            return json_error(StatusCode::FORBIDDEN, "device not linked").into_response();
        }
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response();
        }
    }
    match list_conversations(&state.db, &ctx.user.id, &q.home_device_id).await {
        Ok(conversations) => Json(json!({ "conversations": conversations })).into_response(),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub struct EventsQ {
    pub after: Option<i64>,
}

pub async fn get_remote_conversation(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Path(id): Path<String>,
    Query(q): Query<EventsQ>,
) -> impl IntoResponse {
    let conv = match get_conversation(&state.db, &ctx.user.id, &id).await {
        Ok(Some(c)) => c,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response();
        }
    };
    let events = match list_events(&state.db, &id, q.after.unwrap_or(0)).await {
        Ok(e) => e,
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response();
        }
    };
    Json(json!({ "conversation": conv, "events": events })).into_response()
}

#[derive(Deserialize)]
pub struct PromptBody {
    pub home_device_id: String,
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub title: Option<String>,
}

pub async fn post_remote_prompt(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<PromptBody>,
) -> impl IntoResponse {
    let prompt = body.prompt.trim();
    if prompt.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "prompt required").into_response();
    }
    match verify_device_owner(&state.db, &ctx.user.id, &body.home_device_id).await {
        Ok(true) => {}
        Ok(false) => {
            return json_error(StatusCode::FORBIDDEN, "device not linked").into_response();
        }
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response();
        }
    }
    if !state.remote_chat.is_online(&body.home_device_id).await {
        return json_error(StatusCode::CONFLICT, "device offline").into_response();
    }

    let conv = if let Some(id) = body
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        match get_conversation(&state.db, &ctx.user.id, id).await {
            Ok(Some(c)) if c.home_device_id == body.home_device_id => c,
            Ok(Some(_)) => {
                return json_error(StatusCode::FORBIDDEN, "device mismatch").into_response();
            }
            Ok(None) => return json_error(StatusCode::NOT_FOUND, "not found").into_response(),
            Err(e) => {
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
                    .into_response();
            }
        }
    } else {
        let title = body
            .title
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(prompt)
            .chars()
            .take(120)
            .collect::<String>();
        match create_conversation(&state.db, &ctx.user.id, &body.home_device_id, &title).await {
            Ok(c) => c,
            Err(e) => {
                return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
                    .into_response();
            }
        }
    };

    let _ = append_event(
        &state.db,
        &conv.id,
        "user_prompt",
        &json!({ "text": prompt }),
    )
    .await;
    let _ = sqlx::query("UPDATE cloud_conversations SET status = 'running' WHERE id = ?")
        .bind(&conv.id)
        .execute(state.db.pool())
        .await;

    let sent = state
        .remote_chat
        .send_to_device(
            &body.home_device_id,
            json!({
                "type": "prompt",
                "conversation_id": conv.id,
                "local_session_id": conv.local_session_id,
                "prompt": prompt,
            }),
        )
        .await;
    if !sent {
        return json_error(StatusCode::CONFLICT, "device offline").into_response();
    }
    Json(json!({ "ok": true, "conversation": conv })).into_response()
}

#[derive(Deserialize)]
pub struct CancelBody {
    pub conversation_id: String,
}

pub async fn post_remote_cancel(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Json(body): Json<CancelBody>,
) -> impl IntoResponse {
    let conv = match get_conversation(&state.db, &ctx.user.id, &body.conversation_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return json_error(StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response();
        }
    };
    let sent = state
        .remote_chat
        .send_to_device(
            &conv.home_device_id,
            json!({
                "type": "cancel",
                "conversation_id": conv.id,
                "local_session_id": conv.local_session_id,
            }),
        )
        .await;
    if !sent {
        return json_error(StatusCode::CONFLICT, "device offline").into_response();
    }
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
pub struct WsQ {
    pub device_id: Option<String>,
    pub access_token: Option<String>,
}

pub async fn remote_chat_ws(
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
    Query(q): Query<WsQ>,
    req: axum::extract::Request,
) -> impl IntoResponse {
    let token = q.access_token.clone().or_else(|| bearer_from_request(&req));
    let Some(token) = token.filter(|t| !t.trim().is_empty()) else {
        return json_error(StatusCode::UNAUTHORIZED, "missing token").into_response();
    };
    let user = match crate::store::resolve_session(&state.db, &token).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            return json_error(StatusCode::UNAUTHORIZED, "invalid credentials").into_response()
        }
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response()
        }
    };
    let Some(device_id) = q.device_id.filter(|s| !s.trim().is_empty()) else {
        return json_error(StatusCode::BAD_REQUEST, "device_id required").into_response();
    };
    match verify_device_owner(&state.db, &user.id, &device_id).await {
        Ok(true) => {}
        Ok(false) => return json_error(StatusCode::FORBIDDEN, "device not linked").into_response(),
        Err(e) => {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response()
        }
    }
    let hub = state.remote_chat.clone();
    let db = state.db.clone();
    let user_id = user.id.clone();
    ws.on_upgrade(move |socket| device_socket(socket, hub, db, user_id, device_id))
        .into_response()
}

async fn device_socket(
    mut socket: WebSocket,
    hub: RemoteChatHub,
    db: AccountDb,
    user_id: String,
    device_id: String,
) {
    let mut rx = hub.register(device_id.clone()).await;
    let _ = sqlx::query("UPDATE linked_devices SET last_seen_at = NOW(3) WHERE id = ?")
        .bind(&device_id)
        .execute(db.pool())
        .await;
    loop {
        tokio::select! {
            incoming = rx.recv() => {
                match incoming {
                    Some(msg) => {
                        if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }
            frame = socket.recv() => {
                match frame {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                            handle_device_frame(&db, &user_id, &device_id, v).await;
                        }
                    }
                    Some(Ok(Message::Ping(p))) => {
                        let _ = socket.send(Message::Pong(p)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
    hub.unregister(&device_id).await;
}

async fn handle_device_frame(db: &AccountDb, user_id: &str, device_id: &str, v: Value) {
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    let conversation_id = v
        .get("conversation_id")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    if conversation_id.is_empty() {
        return;
    }
    let Ok(Some(conv)) = get_conversation(db, user_id, conversation_id).await else {
        return;
    };
    if conv.home_device_id != device_id {
        return;
    }
    match typ {
        "bind" => {
            if let Some(local) = v.get("local_session_id").and_then(|x| x.as_str()) {
                let _ = sqlx::query(
                    "UPDATE cloud_conversations SET local_session_id = ?, project_name = COALESCE(?, project_name) WHERE id = ?",
                )
                .bind(local)
                .bind(v.get("project_name").and_then(|x| x.as_str()))
                .bind(conversation_id)
                .execute(db.pool())
                .await;
            }
        }
        "status" => {
            if let Some(status) = v.get("status").and_then(|x| x.as_str()) {
                let _ = sqlx::query("UPDATE cloud_conversations SET status = ? WHERE id = ?")
                    .bind(status)
                    .bind(conversation_id)
                    .execute(db.pool())
                    .await;
            }
        }
        "event" => {
            let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("event");
            let payload = v.get("payload").cloned().unwrap_or(Value::Null);
            let _ = append_event(db, conversation_id, kind, &payload).await;
        }
        _ => {}
    }
}
