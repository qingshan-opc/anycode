//! Optional local embedding vectors for project knowledge (FastEmbed + SQLite sidecar).
//!
//! Enable with `anycode-tools/knowledge-embeddings` (pulls `anycode-memory/embedding-local`).
//!
//! 存储说明（DATA-02）：原为 sled sidecar（`vec.sled` 目录），现为单文件 `vec.db`
//! （`anycode_memory::SqliteVectorBackend` 的 `scored_entries` 表）。该 sidecar 是**纯派生
//! 缓存**——source of truth 是 dashboard `project_knowledge_chunks` 表（见
//! `dashboard/src/project_knowledge.rs::rebuild_vectors`，每次重建都全量重写本缓存并顺带导出
//! `chunks.jsonl`），因此不做 sled→sqlite 数据迁移：旧 `vec.sled` 目录下次重建时自然失效，
//! 可手动删除。

#[cfg(feature = "knowledge-embeddings")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "knowledge-embeddings")]
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// `scored_entries` 命名空间（每个 project 独立 db 文件，单命名空间即可）。
#[cfg(feature = "knowledge-embeddings")]
const KNOWLEDGE_NS: &str = "project-knowledge";

#[cfg(feature = "knowledge-embeddings")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredChunk {
    source_file: String,
    snippet: String,
}

#[derive(Debug, Clone)]
pub struct VectorHit {
    pub source_file: String,
    pub snippet: String,
    pub score: f32,
}

pub fn vectors_feature_enabled() -> bool {
    cfg!(feature = "knowledge-embeddings")
}

pub fn vector_store_path(project_id: &str) -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".anycode/knowledge").join(project_id).join("vec.db"))
}

pub fn vector_chunk_count(project_id: &str) -> usize {
    #[cfg(feature = "knowledge-embeddings")]
    {
        let Some(path) = vector_store_path(project_id) else {
            return 0;
        };
        if !path.is_file() {
            return 0;
        }
        // 该函数是 sync API（dashboard stats 诊断用）；sqlx 是异步的，
        // 在独立线程起临时 current_thread runtime 计数，避免阻塞调用方 runtime。
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .ok()?;
            rt.block_on(async move {
                let backend = anycode_memory::SqliteVectorBackend::new(&path).ok()?;
                backend.scored_count(KNOWLEDGE_NS).await.ok()
            })
        })
        .join()
        .ok()
        .flatten()
        .unwrap_or(0)
    }
    #[cfg(not(feature = "knowledge-embeddings"))]
    {
        let _ = project_id;
        0
    }
}

/// Merge BM25 and vector hits by source file + snippet prefix; hybrid score = 0.65*vec + 0.35*bm25 (normalized).
pub fn merge_hybrid_knowledge_hits<T>(
    bm25: Vec<T>,
    vectors: Vec<VectorHit>,
    limit: usize,
    source: impl Fn(&T) -> &str,
    snippet: impl Fn(&T) -> &str,
    score: impl Fn(&T) -> f32,
    map: impl Fn(String, String, f32) -> T,
) -> Vec<T> {
    if vectors.is_empty() {
        let mut out = bm25;
        out.truncate(limit);
        return out;
    }
    let max_bm25 = bm25.iter().map(&score).fold(0.0_f32, f32::max).max(1e-6);
    let max_vec = vectors
        .iter()
        .map(|h| h.score)
        .fold(0.0_f32, f32::max)
        .max(1e-6);

    let mut merged: std::collections::HashMap<String, (String, f32)> =
        std::collections::HashMap::new();
    for h in bm25 {
        let key = format!(
            "{}::{}",
            source(&h),
            snippet(&h).chars().take(64).collect::<String>()
        );
        let norm = score(&h) / max_bm25;
        merged.insert(key, (snippet(&h).to_string(), 0.35 * norm));
    }
    for v in vectors {
        let key = format!(
            "{}::{}",
            v.source_file,
            v.snippet.chars().take(64).collect::<String>()
        );
        let norm = v.score / max_vec;
        merged
            .entry(key)
            .and_modify(|(_, s)| *s += 0.65 * norm)
            .or_insert((v.snippet.clone(), 0.65 * norm));
    }
    let mut rows: Vec<(String, String, f32)> = merged
        .into_iter()
        .map(|(k, (snip, sc))| {
            let src = k.split("::").next().unwrap_or("").to_string();
            (src, snip, sc)
        })
        .collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    rows.truncate(limit);
    rows.into_iter()
        .map(|(src, snip, sc)| map(src, snip, sc))
        .collect()
}

#[cfg(not(feature = "knowledge-embeddings"))]
pub async fn rebuild_project_vectors(
    _project_id: &str,
    _chunks: &[(String, String, String)],
) -> Result<usize> {
    Ok(0)
}

#[cfg(not(feature = "knowledge-embeddings"))]
pub async fn search_project_vectors(
    _project_id: &str,
    _query: &str,
    _limit: usize,
) -> Result<Vec<VectorHit>> {
    Ok(vec![])
}

#[cfg(feature = "knowledge-embeddings")]
pub async fn rebuild_project_vectors(
    project_id: &str,
    chunks: &[(String, String, String)],
) -> Result<usize> {
    use anycode_core::EmbeddingProvider;
    use anycode_memory::FastEmbedEmbeddingProvider;

    let Some(path) = vector_store_path(project_id) else {
        return Ok(0);
    };
    // 与 sled 版一致：rebuild 即全量重写（旧库文件连同 WAL/SHM 删除后重建）。
    if path.exists() {
        std::fs::remove_file(&path).with_context(|| format!("clear {}", path.display()))?;
    }
    for ext in ["wal", "shm"] {
        let side = path.with_extension(format!("db-{ext}"));
        if side.exists() {
            let _ = std::fs::remove_file(&side);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let backend = anycode_memory::SqliteVectorBackend::new(&path)
        .with_context(|| format!("open sqlite {}", path.display()))?;
    let embedder =
        FastEmbedEmbeddingProvider::try_new(None, None).context("init FastEmbed embedder")?;
    let mut count = 0usize;
    for (id, source_file, content) in chunks {
        let text = content.trim();
        if text.is_empty() {
            continue;
        }
        let vec = embedder.embed_one(text).await.context("embed chunk")?;
        let snippet: String = text.chars().take(400).collect();
        let rec = StoredChunk {
            source_file: source_file.clone(),
            snippet,
        };
        let bytes = serde_json::to_vec(&rec).context("serialize chunk meta")?;
        backend
            .scored_upsert(KNOWLEDGE_NS, id, &vec, &bytes)
            .await
            .with_context(|| format!("sqlite upsert {id}"))?;
        count += 1;
    }
    Ok(count)
}

#[cfg(feature = "knowledge-embeddings")]
pub async fn search_project_vectors(
    project_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<VectorHit>> {
    use anycode_core::EmbeddingProvider;
    use anycode_memory::FastEmbedEmbeddingProvider;

    let q = query.trim();
    if q.is_empty() || limit == 0 {
        return Ok(vec![]);
    }
    let Some(path) = vector_store_path(project_id) else {
        return Ok(vec![]);
    };
    if !path.is_file() {
        return Ok(vec![]);
    }
    let backend = anycode_memory::SqliteVectorBackend::new(&path)
        .with_context(|| format!("open sqlite {}", path.display()))?;
    let embedder =
        FastEmbedEmbeddingProvider::try_new(None, None).context("init FastEmbed embedder")?;
    let q_vec = embedder.embed_one(q).await.context("embed query")?;
    // 与 sled 版一致：余弦 > 0.05、按分降序、truncate(limit)。
    let rows = backend
        .scored_search(KNOWLEDGE_NS, &q_vec, 0.05, limit)
        .await
        .context("vector scan")?;
    let mut hits = Vec::with_capacity(rows.len());
    for (_, payload, score) in rows {
        let Ok(rec) = serde_json::from_slice::<StoredChunk>(&payload) else {
            continue;
        };
        hits.push(VectorHit {
            source_file: rec.source_file,
            snippet: rec.snippet,
            score,
        });
    }
    Ok(hits)
}

#[cfg(all(test, feature = "knowledge-embeddings"))]
mod tests {
    use super::*;

    #[test]
    fn merge_prefers_higher_combined_score() {
        #[derive(Clone)]
        struct Hit {
            src: String,
            snip: String,
            score: f32,
        }
        let bm25 = vec![Hit {
            src: "a.md".into(),
            snip: "hello world".into(),
            score: 2.0,
        }];
        let vec = vec![VectorHit {
            source_file: "b.md".into(),
            snippet: "other doc".into(),
            score: 0.9,
        }];
        let merged = merge_hybrid_knowledge_hits(
            bm25,
            vec,
            2,
            |h| h.src.as_str(),
            |h| h.snip.as_str(),
            |h| h.score,
            |src, snip, score| Hit { src, snip, score },
        );
        assert_eq!(merged.len(), 2);
        assert!(merged[0].score >= merged[1].score);
    }
}
