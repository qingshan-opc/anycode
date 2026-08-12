//! 各 LLM provider 共享的 reqwest client 构建。
//!
//! - 非流式：`build_api_http_client()` —— connect timeout + 总超时（`API_TIMEOUT_MS`，默认 180s）。
//! - 流式（SSE）：`build_streaming_api_http_client()` —— **不设总超时**，长流不会被整体掐断；
//!   用 connect timeout + 读空闲超时（相邻两个 chunk 的最大间隔，`API_STREAM_IDLE_TIMEOUT_MS`，
//!   默认 120s）。半死连接在空闲超时后报错，由 provider 转成 `StreamEvent::Failed`。
//!
//! 之前 `openai.rs` / `zai.rs` 各自复制了一份，而 `anthropic.rs` / `github_copilot.rs`
//! 直接用裸 `Client::new()`（无超时， stalled 连接会无限挂起一轮对话）。统一到这里。

use reqwest::Client;
use std::time::Duration;

/// 单次非流式 HTTP 请求总超时；更长对话可设环境变量 `API_TIMEOUT_MS`。
pub(crate) const DEFAULT_API_TIMEOUT_MS: u64 = 180_000;
/// 建连阶段（TCP/TLS）超时，流式/非流式共用。
pub(crate) const DEFAULT_API_CONNECT_TIMEOUT_MS: u64 = 30_000;
/// 流式响应的读空闲超时：相邻两个 chunk（含响应头等待）之间的最大间隔。
/// 总时长不设限——只要流持续推进就不限时；卡死超过该间隔即判失败。
pub(crate) const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 120_000;

/// 环境变量解析：要求 >= 1000ms，否则回落默认值。抽出以便单测。
fn parse_timeout_ms(raw: Option<String>, default: u64) -> u64 {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 1_000)
        .unwrap_or(default)
}

pub(crate) fn configured_api_timeout_ms() -> u64 {
    parse_timeout_ms(std::env::var("API_TIMEOUT_MS").ok(), DEFAULT_API_TIMEOUT_MS)
}

pub(crate) fn configured_stream_idle_timeout_ms() -> u64 {
    parse_timeout_ms(
        std::env::var("API_STREAM_IDLE_TIMEOUT_MS").ok(),
        DEFAULT_STREAM_IDLE_TIMEOUT_MS,
    )
}

pub(crate) fn build_api_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_millis(DEFAULT_API_CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(configured_api_timeout_ms()))
        .build()
        .unwrap_or_else(|_| Client::new())
}

/// 流式（SSE）专用 client：无总超时；`read_timeout` 是每次 socket read 的超时，
/// 等价于"相邻 chunk 间隔上限"，长流持续推进不会被掐断，半死连接空闲超时后报错。
pub(crate) fn build_streaming_api_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_millis(DEFAULT_API_CONNECT_TIMEOUT_MS))
        .read_timeout(Duration::from_millis(configured_stream_idle_timeout_ms()))
        .build()
        .unwrap_or_else(|_| Client::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use tokio::io::{AsyncWriteExt, Interest};
    use tokio::net::TcpListener;

    #[test]
    fn parse_timeout_ms_honors_floor_and_fallback() {
        assert_eq!(parse_timeout_ms(None, 42), 42);
        assert_eq!(parse_timeout_ms(Some("".into()), 42), 42);
        assert_eq!(parse_timeout_ms(Some("abc".into()), 42), 42);
        assert_eq!(parse_timeout_ms(Some("999".into()), 42), 42);
        assert_eq!(parse_timeout_ms(Some("1000".into()), 42), 1000);
        assert_eq!(parse_timeout_ms(Some(" 5000 ".into()), 42), 5000);
    }

    /// 起一个最小 HTTP 服务端：按 `delays` 的节奏逐个吐出 chunk，然后保持连接。
    async fn spawn_chunked_server(delays: Vec<Duration>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            // 等到客户端真的发来请求再回包，避免竞态。
            socket.ready(Interest::READABLE).await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.try_read(&mut buf);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            for (i, delay) in delays.iter().enumerate() {
                if !delay.is_zero() {
                    tokio::time::sleep(*delay).await;
                }
                let payload = format!("data: chunk-{i}\n\n");
                let frame = format!("{:x}\r\n{payload}\r\n", payload.len());
                if socket.write_all(frame.as_bytes()).await.is_err() {
                    return;
                }
            }
            // 之后不再写数据，模拟挂起的长连接。
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        format!("http://{addr}/sse")
    }

    fn streaming_client_with_idle(idle: Duration) -> Client {
        Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(idle)
            .build()
            .unwrap()
    }

    /// NET-09 回归：流式 client 无总超时——总时长（900ms）远超 idle（300ms），
    /// 但只要每个 chunk 间隔低于 idle 就能完整读完。旧的 180s 总超时实现会把
    /// 超过总时限的流整体杀掉；这里验证"持续推进的长流不被掐断"的语义。
    #[tokio::test]
    async fn streaming_client_has_no_total_timeout_when_chunks_keep_coming() {
        let url = spawn_chunked_server(vec![
            Duration::from_millis(300),
            Duration::from_millis(300),
            Duration::from_millis(300),
        ])
        .await;
        let client = streaming_client_with_idle(Duration::from_millis(500));

        let started = std::time::Instant::now();
        let resp = client.get(&url).send().await.unwrap();
        let mut stream = resp.bytes_stream();
        let mut chunks = 0;
        while let Some(chunk) = stream.next().await {
            chunk.unwrap();
            chunks += 1;
            if chunks >= 3 {
                break;
            }
        }
        assert_eq!(chunks, 3);
        assert!(
            started.elapsed() >= Duration::from_millis(850),
            "test assumes ~900ms total stream time, got {:?}",
            started.elapsed()
        );
    }

    /// NET-09：流式 client 有读空闲超时——chunk 间隔超过 idle 即报错（半死连接不死等）。
    #[tokio::test]
    async fn streaming_client_errors_when_stream_goes_idle() {
        let url = spawn_chunked_server(vec![
            Duration::ZERO,
            Duration::from_millis(2_000), // 远超 idle=300ms
        ])
        .await;
        let client = streaming_client_with_idle(Duration::from_millis(300));

        let resp = client.get(&url).send().await.unwrap();
        let mut stream = resp.bytes_stream();
        let mut saw_timeout = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(_) => {}
                Err(e) => {
                    assert!(e.is_timeout(), "expected idle timeout error, got {e}");
                    saw_timeout = true;
                    break;
                }
            }
        }
        assert!(saw_timeout, "idle stream must surface a timeout error");
    }
}
