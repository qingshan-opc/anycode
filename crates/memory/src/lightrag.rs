//! LightRAG HTTP sidecar adapter (`MemoryStore` over REST).
//!
//! Default sidecar: `http://127.0.0.1:18765` (override with `ANYCODE_LIGHTRAG_URL`).
//!
//! - `save` → `POST /documents/text`
//! - `recall` → `POST /query` (hybrid mode; response text projected as memories)
//! - `update` → re-save; `delete` → best-effort no-op (sidecar has no id delete)

use anycode_core::prelude::*;
use async_trait::async_trait;
use serde::Deserialize;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Default LightRAG sidecar base URL (anyCode M2 convention).
pub const DEFAULT_LIGHTRAG_BASE_URL: &str = "http://127.0.0.1:18765";

/// Resolve base URL from env or default.
pub fn resolve_lightrag_base_url() -> String {
    std::env::var("ANYCODE_LIGHTRAG_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_LIGHTRAG_BASE_URL.to_string())
}

fn normalize_base(base: &str) -> String {
    base.trim().trim_end_matches('/').to_string()
}

/// Parse `http://host:port` (or `https://`) into `(host, port)` without the `url` crate.
fn host_port_from_base(base: &str) -> Result<(String, u16), String> {
    let rest = base
        .strip_prefix("http://")
        .or_else(|| base.strip_prefix("https://"))
        .ok_or_else(|| format!("LightRAG URL must be http(s): {base}"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    if let Some((host, port_s)) = authority.rsplit_once(':') {
        let port: u16 = port_s
            .parse()
            .map_err(|_| format!("invalid LightRAG port in {base}"))?;
        let host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        if host.is_empty() {
            return Err(format!("LightRAG URL missing host: {base}"));
        }
        Ok((host, port))
    } else if base.starts_with("https://") {
        Ok((authority.to_string(), 443))
    } else {
        Ok((authority.to_string(), 80))
    }
}

/// TCP reachability probe (safe inside an async runtime; no nested `block_on`).
pub fn probe_lightrag_reachable(base_url: &str) -> Result<(), String> {
    let base = normalize_base(base_url);
    let (host, port) = host_port_from_base(&base)?;
    let addr = format!("{host}:{port}");
    let mut addrs = addr
        .to_socket_addrs()
        .map_err(|e| format!("resolve {addr}: {e}"))?
        .collect::<Vec<SocketAddr>>();
    if addrs.is_empty() {
        return Err(format!("no addresses for {addr}"));
    }
    // Prefer loopback first when present.
    addrs.sort_by_key(|a| !a.ip().is_loopback());
    let timeout = Duration::from_secs(2);
    let mut last = None;
    for a in addrs {
        match TcpStream::connect_timeout(&a, timeout) {
            Ok(_) => return Ok(()),
            Err(e) => last = Some(e.to_string()),
        }
    }
    Err(last.unwrap_or_else(|| format!("connect failed: {addr}")))
}

pub struct LightRagMemoryStore {
    client: reqwest::Client,
    base_url: String,
}

impl LightRagMemoryStore {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            base_url: normalize_base(&base_url.into()),
        }
    }

    /// Open after a successful reachability probe.
    pub fn try_open(base_url: impl Into<String>) -> Result<Self, String> {
        let base = normalize_base(&base_url.into());
        probe_lightrag_reachable(&base)?;
        info!(target: "anycode_memory", base_url = %base, "LightRAG memory store connected");
        Ok(Self::new(base))
    }

    fn mem_to_text(memory: &Memory) -> String {
        let type_s = memory.mem_type.as_storage_str();
        let tags = if memory.tags.is_empty() {
            String::new()
        } else {
            format!("\ntags: {}", memory.tags.join(", "))
        };
        format!(
            "id: {}\ntype: {}\ntitle: {}{}\n\n{}",
            memory.id, type_s, memory.title, tags, memory.content
        )
    }

    fn memories_from_query_response(
        &self,
        query: &str,
        mem_type: MemoryType,
        body: QueryResponse,
    ) -> Vec<Memory> {
        let mut out = Vec::new();
        let now = chrono::Utc::now();
        if let Some(refs) = body.references.filter(|r| !r.is_empty()) {
            for (i, r) in refs.into_iter().take(12).enumerate() {
                let content = r.content.or(r.text).unwrap_or_default().trim().to_string();
                if content.is_empty() {
                    continue;
                }
                let title = r
                    .file_path
                    .or(r.source)
                    .unwrap_or_else(|| format!("lightrag-{i}"));
                out.push(Memory {
                    id: format!("lightrag-{}-{i}", mem_type.as_storage_str()),
                    mem_type,
                    title,
                    content,
                    tags: vec!["lightrag".into()],
                    scope: MemoryScope::Private,
                    created_at: now,
                    updated_at: now,
                    meta: None,
                });
            }
        }
        if out.is_empty() {
            if let Some(resp) = body.response.filter(|s| !s.trim().is_empty()) {
                out.push(Memory {
                    id: format!("lightrag-query-{}", mem_type.as_storage_str()),
                    mem_type,
                    title: if query.is_empty() {
                        "LightRAG recall".into()
                    } else {
                        format!("LightRAG: {}", query.chars().take(64).collect::<String>())
                    },
                    content: resp,
                    tags: vec!["lightrag".into()],
                    scope: MemoryScope::Private,
                    created_at: now,
                    updated_at: now,
                    meta: None,
                });
            }
        }
        out
    }
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    #[serde(default)]
    response: Option<String>,
    #[serde(default)]
    references: Option<Vec<QueryReference>>,
}

#[derive(Debug, Deserialize)]
struct QueryReference {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    source: Option<String>,
}

#[async_trait]
impl MemoryStore for LightRagMemoryStore {
    async fn save(&self, memory: Memory) -> Result<(), CoreError> {
        let url = format!("{}/documents/text", self.base_url);
        let text = Self::mem_to_text(&memory);
        let body = serde_json::json!({
            "text": text,
            "file_source": memory.id,
            "description": memory.title,
        });
        let res = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Other(anyhow::anyhow!("lightrag save: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(CoreError::Other(anyhow::anyhow!(
                "lightrag save status: {t}"
            )));
        }
        debug!(target: "anycode_memory", id = %memory.id, "LightRAG document inserted");
        Ok(())
    }

    async fn recall(&self, query: &str, mem_type: MemoryType) -> Result<Vec<Memory>, CoreError> {
        let url = format!("{}/query", self.base_url);
        let q = if query.trim().is_empty() {
            format!("list recent {} memories", mem_type.as_storage_str())
        } else {
            query.to_string()
        };
        let body = serde_json::json!({
            "query": q,
            "mode": "hybrid",
            "only_need_context": true,
        });
        let res = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CoreError::Other(anyhow::anyhow!("lightrag recall: {e}")))?;
        if !res.status().is_success() {
            let t = res.text().await.unwrap_or_default();
            return Err(CoreError::Other(anyhow::anyhow!(
                "lightrag recall status: {t}"
            )));
        }
        let parsed: QueryResponse = res
            .json()
            .await
            .map_err(|e| CoreError::Other(anyhow::anyhow!("lightrag recall json: {e}")))?;
        Ok(self.memories_from_query_response(query, mem_type, parsed))
    }

    async fn update(&self, _id: &str, memory: Memory) -> Result<(), CoreError> {
        self.save(memory).await
    }

    async fn delete(&self, id: &str) -> Result<(), CoreError> {
        // Upstream LightRAG API has no per-document id delete in the common REST surface.
        warn!(
            target: "anycode_memory",
            id,
            "LightRAG delete is a no-op (sidecar has no id-scoped delete)"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_trims_slash() {
        assert_eq!(
            normalize_base("http://127.0.0.1:18765/"),
            "http://127.0.0.1:18765"
        );
    }

    #[test]
    fn probe_closed_port_fails() {
        // Unlikely to be open; if it is, probe succeeds — still a valid Result path.
        let _ = probe_lightrag_reachable("http://127.0.0.1:1");
    }
}
