//! 各 LLM provider 共享的 reqwest client 构建：连接/总超时统一由 `API_TIMEOUT_MS` 调节。
//!
//! 之前 `openai.rs` / `zai.rs` 各自复制了一份，而 `anthropic.rs` / `github_copilot.rs`
//! 直接用裸 `Client::new()`（无超时， stalled 连接会无限挂起一轮对话）。统一到这里。

use reqwest::Client;
use std::time::Duration;

/// 单次 HTTP 请求总超时（含流式读 body）；更长对话可设环境变量 `API_TIMEOUT_MS`。
pub(crate) const DEFAULT_API_TIMEOUT_MS: u64 = 180_000;
/// 建连阶段（TCP/TLS）超时，与总超时独立。
pub(crate) const DEFAULT_API_CONNECT_TIMEOUT_MS: u64 = 30_000;

pub(crate) fn configured_api_timeout_ms() -> u64 {
    std::env::var("API_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 1_000)
        .unwrap_or(DEFAULT_API_TIMEOUT_MS)
}

pub(crate) fn build_api_http_client() -> Client {
    Client::builder()
        .connect_timeout(Duration::from_millis(DEFAULT_API_CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(configured_api_timeout_ms()))
        .build()
        .unwrap_or_else(|_| Client::new())
}
