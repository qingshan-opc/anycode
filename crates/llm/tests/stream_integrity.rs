//! 流式完整性集成测试：用裸 TCP mock SSE 服务器模拟
//! 中途截断 / 正常完成 / HTTP 错误三种形态，断言 provider 会区分
//! `StreamEvent::Failed` 与 `StreamEvent::Done`（此前一律静默发 Done，
//! 截断的 partial 回复会被 agent loop 当作完整回复接受）。

use anycode_core::prelude::*;
use anycode_llm::ZaiClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// 起一个一次性 HTTP 服务器：读完整请求后返回给定的 status/body。
async fn serve_once(status_line: &str, content_type: &str, body: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let status_line = status_line.to_string();
    let content_type = content_type.to_string();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // 读请求头
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            let n = sock.read(&mut chunk).await.unwrap();
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        // 按 Content-Length 读完 body（reqwest 要求服务端消费请求体）
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_lowercase();
        let content_len: usize = headers
            .lines()
            .find_map(|l| l.strip_prefix("content-length:"))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_len {
            let n = sock.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let resp = format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    });
    port
}

fn test_config(port: u16) -> ModelConfig {
    ModelConfig {
        provider: LLMProvider::OpenAI,
        model: "glm-5".into(),
        base_url: Some(format!("http://127.0.0.1:{port}/v4/chat/completions")),
        api_key: Some("test-key".into()),
        ..Default::default()
    }
}

fn user_message() -> Vec<Message> {
    vec![Message {
        id: uuid::Uuid::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Text("hi".into()),
        timestamp: chrono::Utc::now(),
        metadata: Default::default(),
    }]
}

async fn collect_events(mut rx: tokio::sync::mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    while let Some(ev) = rx.recv().await {
        out.push(ev);
    }
    out
}

#[tokio::test]
async fn truncated_stream_without_done_marker_reports_failed() {
    // 两个 delta 之后直接断连：没有 [DONE]，也没有 finish_reason。
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好\"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"，世界\"}}]}\r\n\r\n",
    )
    .to_string();
    let port = serve_once("200 OK", "text/event-stream", body).await;

    let client = ZaiClient::new("test-key".into(), None);
    let rx = client
        .chat_stream(user_message(), vec![], &test_config(port))
        .await
        .unwrap();
    let events = collect_events(rx).await;

    assert!(
        events
            .iter()
            .any(|e| matches!(e, StreamEvent::Delta(d) if d == "你好")),
        "partial deltas should still be delivered: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, StreamEvent::Failed(_))),
        "truncated stream must surface Failed: {events:?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, StreamEvent::Done)),
        "truncated stream must NOT emit Done: {events:?}"
    );
}

#[tokio::test]
async fn complete_stream_with_done_marker_emits_done_not_failed() {
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\r\n\r\n",
        "data: [DONE]\r\n\r\n",
    )
    .to_string();
    let port = serve_once("200 OK", "text/event-stream", body).await;

    let client = ZaiClient::new("test-key".into(), None);
    let rx = client
        .chat_stream(user_message(), vec![], &test_config(port))
        .await
        .unwrap();
    let events = collect_events(rx).await;

    assert!(events.iter().any(|e| matches!(e, StreamEvent::Done)));
    assert!(!events.iter().any(|e| matches!(e, StreamEvent::Failed(_))));
}

#[tokio::test]
async fn http_error_on_stream_open_reports_failed() {
    // 400 不可重试（非 429/5xx），单次尝试即失败，测试不会卡在退避上。
    let port = serve_once("400 Bad Request", "application/json", "{}".into()).await;

    let client = ZaiClient::new("test-key".into(), None);
    let rx = client
        .chat_stream(user_message(), vec![], &test_config(port))
        .await
        .unwrap();
    let events = collect_events(rx).await;

    assert!(
        events.iter().any(|e| matches!(e, StreamEvent::Failed(_))),
        "HTTP error must surface Failed: {events:?}"
    );
    assert!(!events.iter().any(|e| matches!(e, StreamEvent::Done)));
}
