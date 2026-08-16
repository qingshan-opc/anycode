//! Memory store / pipeline construction for runtime bootstrap.

use anycode_config::Config;

use anycode_core::prelude::*;
use anycode_core::{EmbeddingProvider, MemoryPipeline, VectorMemoryBackend};
#[cfg(feature = "embedding-local")]
use anycode_memory::FastEmbedEmbeddingProvider;
use anycode_memory::{
    resolve_lightrag_base_url, FileMemoryStore, HybridMemoryStore, LightRagMemoryStore,
    NoopVectorBackend, OpenAiCompatibleEmbeddingProvider, RootReturnMemoryPipeline,
    SqliteVectorBackend,
};
use async_trait::async_trait;

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// How this process attaches to configured memory (hot store is single-file sqlite; sled is retired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryAttachMode {
    /// Channel bridges: open hybrid/pipeline hot store when configured.
    Exclusive,
    /// Local REPL/run: same Markdown tree as Exclusive; use `file` when config is hybrid/pipeline.
    Shared,
}

impl MemoryAttachMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exclusive => "exclusive",
            Self::Shared => "shared",
        }
    }
}

/// Effective store backend after attach policy (`config.memory.backend` may stay `hybrid`).
pub fn effective_memory_backend(config: &Config, attach: MemoryAttachMode) -> &str {
    match attach {
        MemoryAttachMode::Exclusive => config.memory.backend.as_str(),
        MemoryAttachMode::Shared => match config.memory.backend.as_str() {
            "hybrid" | "pipeline" | "layered" | "guigen" => "file",
            other => other,
        },
    }
}

fn resolve_memory_attach(requested: MemoryAttachMode) -> MemoryAttachMode {
    match std::env::var("ANYCODE_MEMORY_ATTACH")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("exclusive") => MemoryAttachMode::Exclusive,
        Some("shared") => MemoryAttachMode::Shared,
        _ => requested,
    }
}

struct NoopMemoryStore;

#[async_trait]
impl MemoryStore for NoopMemoryStore {
    async fn save(&self, _memory: Memory) -> Result<(), CoreError> {
        Ok(())
    }

    async fn recall(&self, _query: &str, _mem_type: MemoryType) -> Result<Vec<Memory>, CoreError> {
        Ok(vec![])
    }

    async fn update(&self, _id: &str, _memory: Memory) -> Result<(), CoreError> {
        Ok(())
    }

    async fn delete(&self, _id: &str) -> Result<(), CoreError> {
        Ok(())
    }
}

/// 旧 sled 热层目录（已停维护，仅供诊断/一次性迁移定位）。
pub fn memory_sled_path_for_diagnostics(file_memory_root: &Path) -> PathBuf {
    sibling_sled_path(file_memory_root)
}

fn sibling_sled_path(file_memory_root: &Path) -> PathBuf {
    let name = file_memory_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("memory");
    let parent = file_memory_root.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{}.sled", name))
}

/// 热层 sqlite 单文件（`hybrid` backend）：原 `<name>.sled` 目录 → 同目录下 `<name>.hot.db`。
fn sibling_hot_db_path(file_memory_root: &Path) -> PathBuf {
    let name = file_memory_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("memory");
    let parent = file_memory_root.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{}.hot.db", name))
}

/// 热层 sqlite 单文件（归根通道 `pipeline` backend）：原 `<name>.pipeline.sled` 目录 →
/// 同目录下 `<name>.pipeline.hot.db`；向量侧车与热层共用该文件的两张表（原 `.pipeline.vec.sled`）。
fn sibling_pipeline_hot_db_path(file_memory_root: &Path) -> PathBuf {
    let name = file_memory_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("memory");
    let parent = file_memory_root.parent().unwrap_or_else(|| Path::new("."));
    parent.join(format!("{}.pipeline.hot.db", name))
}

fn sibling_pipeline_buffer_wal_path(pipeline_hot_db: &Path) -> PathBuf {
    let s = pipeline_hot_db.to_string_lossy();
    if let Some(base) = s.strip_suffix(".hot.db") {
        // 与 sled 时代一致：`<name>.pipeline.buffer.wal`，存量 WAL 可继续重放。
        PathBuf::from(format!("{base}.buffer.wal"))
    } else {
        pipeline_hot_db.with_extension("buffer.wal")
    }
}

fn open_file_memory_store(path: PathBuf) -> anyhow::Result<FileMemoryStore> {
    FileMemoryStore::new(path).map_err(|e| anyhow::anyhow!("file memory store: {e}"))
}

/// Open LightRAG HTTP store, or fall back to file on sidecar unreachable.
fn open_lightrag_or_file(
    memory_path: PathBuf,
) -> anyhow::Result<(Arc<dyn MemoryStore>, Option<Arc<dyn MemoryPipeline>>)> {
    let base = resolve_lightrag_base_url();
    match LightRagMemoryStore::try_open(&base) {
        Ok(store) => Ok((Arc::new(store), None)),
        Err(e) => {
            tracing::warn!(
                target: "anycode_bootstrap",
                base_url = %base,
                error = %e,
                "memory.backend=lightrag sidecar unreachable; falling back to FileMemoryStore"
            );
            let store = open_file_memory_store(memory_path)?;
            Ok((Arc::new(store), None))
        }
    }
}

/// `plugin:<id>` slot — stub falls back to file until a real plugin registry lands.
fn open_plugin_memory_or_file(
    plugin_id: &str,
    memory_path: PathBuf,
) -> anyhow::Result<(Arc<dyn MemoryStore>, Option<Arc<dyn MemoryPipeline>>)> {
    tracing::warn!(
        target: "anycode_bootstrap",
        plugin_id,
        "memory.backend=plugin:{plugin_id} is a stub; falling back to FileMemoryStore"
    );
    let store = open_file_memory_store(memory_path)?;
    Ok((Arc::new(store), None))
}

pub fn build_memory_layer(
    config: &Config,
    attach: MemoryAttachMode,
) -> anyhow::Result<(Arc<dyn MemoryStore>, Option<Arc<dyn MemoryPipeline>>)> {
    let attach = resolve_memory_attach(attach);
    let backend = effective_memory_backend(config, attach);
    if let Some(plugin_id) = backend.strip_prefix("plugin:") {
        let id = plugin_id.trim();
        if id.is_empty() {
            anyhow::bail!("unsupported memory backend: plugin: (missing id)");
        }
        return open_plugin_memory_or_file(id, config.memory.path.clone());
    }
    match backend {
        "noop" => Ok((Arc::new(NoopMemoryStore), None)),
        "file" => {
            let store = open_file_memory_store(config.memory.path.clone())?;
            Ok((Arc::new(store), None))
        }
        "lightrag" => open_lightrag_or_file(config.memory.path.clone()),
        "hybrid" => {
            let hot_db = sibling_hot_db_path(&config.memory.path);
            let store = HybridMemoryStore::new(hot_db, config.memory.path.clone())
                .map_err(|e| anyhow::anyhow!("hybrid memory store: {e}"))?;
            Ok((Arc::new(store), None))
        }
        "pipeline" => {
            let hot_db = sibling_pipeline_hot_db_path(&config.memory.path);
            let buffer_wal = if config.memory.pipeline.buffer_wal_enabled {
                Some(sibling_pipeline_buffer_wal_path(&hot_db))
            } else {
                None
            };
            let legacy = if config.memory.pipeline.merge_legacy_file_recall {
                Some(Arc::new(open_file_memory_store(
                    config.memory.path.clone(),
                )?))
            } else {
                None
            };
            let (vector, embedding): (
                Arc<dyn VectorMemoryBackend>,
                Option<Arc<dyn EmbeddingProvider>>,
            ) = if config.memory.pipeline.embedding_enabled {
                // 向量侧车与热层共用同一 hot.db（embeddings 表），原 .pipeline.vec.sled 由迁移吸收。
                let v = Arc::new(
                    SqliteVectorBackend::new(hot_db.clone())
                        .map_err(|e| anyhow::anyhow!("pipeline vector sqlite: {}", e))?,
                ) as Arc<dyn VectorMemoryBackend>;
                if config.memory.embedding_provider == "local" {
                    #[cfg(feature = "embedding-local")]
                    {
                        if std::env::var_os("HF_ENDPOINT").is_none() {
                            if let Some(ref ep) = config.memory.embedding_hf_endpoint {
                                let t = ep.trim();
                                if !t.is_empty() {
                                    std::env::set_var("HF_ENDPOINT", t);
                                }
                            }
                        }
                        let emb = FastEmbedEmbeddingProvider::try_new(
                            config.memory.embedding_local_cache_dir.clone(),
                            config.memory.embedding_local_model.clone(),
                        )
                        .map_err(|e| anyhow::anyhow!("local embedding init: {}", e))?;
                        (v, Some(Arc::new(emb) as Arc<dyn EmbeddingProvider>))
                    }
                    #[cfg(not(feature = "embedding-local"))]
                    {
                        anyhow::bail!(
                            "memory.pipeline.embedding_provider is \"local\" but this build lacks the `embedding-local` feature. Rebuild with embedding-local enabled on anycode-bootstrap / anycode-dashboard"
                        );
                    }
                } else {
                    let registry_settings =
                        anycode_llm::read_config_value(None)
                            .ok()
                            .and_then(|(_, cfg_json)| {
                                let reg =
                                    anycode_llm::ResolvedModelRegistry::from_config(&cfg_json);
                                reg.active_item(anycode_llm::ModelCapability::Embedding)
                                    .map(|item| {
                                        (
                                            reg.resolve_model(item),
                                            reg.resolve_base_url(item).unwrap_or_else(|| {
                                                "https://api.openai.com/v1".to_string()
                                            }),
                                            reg.resolve_api_key(item),
                                        )
                                    })
                            });
                    let base_url = config
                        .memory
                        .embedding_base_url
                        .clone()
                        .or_else(|| registry_settings.as_ref().map(|(_, u, _)| u.clone()))
                        .or_else(|| config.llm.base_url.clone())
                        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
                    let model = config
                        .memory
                        .embedding_model
                        .clone()
                        .or_else(|| registry_settings.as_ref().map(|(m, _, _)| m.clone()))
                        .unwrap_or_else(|| "text-embedding-3-small".to_string());
                    let key = registry_settings
                        .as_ref()
                        .and_then(|(_, _, k)| k.clone())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| config.llm.api_key.trim().to_string());
                    if key.is_empty() {
                        tracing::warn!(
                            target: "anycode_cli",
                            "memory.pipeline.embedding_enabled (http) but llm api_key empty; embeddings disabled"
                        );
                        (
                            Arc::new(NoopVectorBackend) as Arc<dyn VectorMemoryBackend>,
                            None,
                        )
                    } else {
                        let emb =
                            Arc::new(OpenAiCompatibleEmbeddingProvider::new(base_url, key, model));
                        (v, Some(emb as Arc<dyn EmbeddingProvider>))
                    }
                }
            } else {
                (
                    Arc::new(NoopVectorBackend) as Arc<dyn VectorMemoryBackend>,
                    None,
                )
            };
            let pipe = Arc::new(RootReturnMemoryPipeline::open(
                config.memory.pipeline.clone(),
                hot_db,
                buffer_wal,
                legacy,
                vector,
                embedding,
            )?);
            let pipeline_iface: Arc<dyn MemoryPipeline> = pipe.clone();
            let store_iface: Arc<dyn MemoryStore> = pipe;
            Ok((store_iface, Some(pipeline_iface)))
        }
        other => anyhow::bail!("unsupported memory backend: {other}"),
    }
}

#[cfg(test)]
fn effective_memory_backend_for_test(configured: &str, attach: MemoryAttachMode) -> &str {
    match attach {
        MemoryAttachMode::Exclusive => configured,
        MemoryAttachMode::Shared => match configured {
            "hybrid" | "pipeline" | "layered" | "guigen" => "file",
            other => other,
        },
    }
}

#[cfg(test)]
mod memory_attach_tests {
    use super::MemoryAttachMode;

    #[test]
    fn shared_attach_maps_sled_backends_to_file() {
        for b in ["hybrid", "pipeline", "layered", "guigen"] {
            assert_eq!(
                super::effective_memory_backend_for_test(b, MemoryAttachMode::Shared),
                "file"
            );
            assert_eq!(
                super::effective_memory_backend_for_test(b, MemoryAttachMode::Exclusive),
                b
            );
        }
        assert_eq!(
            super::effective_memory_backend_for_test("noop", MemoryAttachMode::Shared),
            "noop"
        );
    }
}
