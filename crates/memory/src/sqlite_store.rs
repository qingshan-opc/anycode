//! 热层 SQLite 后端（sqlx / runtime-tokio）：替代已停维护的 sled（DATA-02）。
//!
//! 语义与 `SledMemoryStore` / `SledVectorBackend` 逐点对齐：
//! - `memories` 表 ≈ per-mem_type sled tree 的 JSON-per-key；recall 全量扫描 + Rust 子串过滤
//!   （title/content/tags）+ `KeywordRetrieval::rank`；`ORDER BY id` 逼近 sled 的字节序遍历。
//! - `embeddings` 表 ≈ 向量侧车 sled 默认 tree；search 余弦线性扫描、按分降序 `take(limit.max(1))`。
//!
//! 两者共用同一 `.db` 文件（两张表）；moka cache 行为与 sled 版一致（save 写入 / delete 失效）。
//!
//! 构造是同步且惰性的（`connect_lazy_with`，与 `SledMemoryStore::new` 人体工学一致）；
//! 建表与一次性 sled 迁移在首个异步操作时经 `tokio::sync::OnceCell` 执行。

use crate::retrieval::{KeywordRetrieval, MemoryRetrieval};
use crate::MemoryError;
use anycode_core::prelude::*;
use anycode_core::VectorMemoryBackend;
use async_trait::async_trait;
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

const CREATE_MEMORIES: &str = "CREATE TABLE IF NOT EXISTS memories (
    mem_type TEXT NOT NULL,
    id TEXT NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (mem_type, id)
)";

const CREATE_EMBEDDINGS: &str = "CREATE TABLE IF NOT EXISTS embeddings (
    mem_type INTEGER NOT NULL,
    id TEXT NOT NULL,
    payload BLOB NOT NULL,
    PRIMARY KEY (mem_type, id)
)";

fn core_err(e: impl std::fmt::Display) -> CoreError {
    CoreError::Other(anyhow::anyhow!(e.to_string()))
}

/// 与 `vector_sled.rs` 相同的落盘记录（迁移时按字节原样搬运）。
#[derive(Debug, Serialize, Deserialize)]
struct StoredEmb {
    id: String,
    #[allow(dead_code)]
    mem_type: MemoryType,
    vec: Vec<f32>,
}

/// 与 `vector_sled.rs` 完全一致的余弦实现。
fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return f32::NAN;
    }
    let mut dot = 0f32;
    let mut na = 0f32;
    let mut nb = 0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    let d = (na.sqrt() * nb.sqrt()).max(1e-12);
    dot / d
}

fn lazy_pool(db_path: &std::path::Path) -> SqlitePool {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(std::time::Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(4)
        .connect_lazy_with(opts)
}

async fn create_schema(pool: &SqlitePool) -> Result<(), MemoryError> {
    sqlx::query(CREATE_MEMORIES).execute(pool).await?;
    sqlx::query(CREATE_EMBEDDINGS).execute(pool).await?;
    Ok(())
}

/// 由 `*.hot.db`（或 `*.db`）库路径推导旧 sled 目录：`<base>.sled`（热层）与
/// `<base>.vec.sled`（向量侧车）。命名约定见 `bootstrap::memory_setup`。
#[cfg(feature = "sled-migrate")]
fn legacy_sled_dirs(db_path: &std::path::Path) -> Option<(PathBuf, PathBuf)> {
    let s = db_path.to_str()?;
    let base = s
        .strip_suffix(".hot.db")
        .or_else(|| s.strip_suffix(".db"))?;
    Some((
        PathBuf::from(format!("{base}.sled")),
        PathBuf::from(format!("{base}.vec.sled")),
    ))
}

/// 一次性迁移：sqlite 库为空且旧 sled 目录存在时，把热层/向量数据原样搬入。
/// sled 仅在此函数内使用；幂等——搬过后表非空，二次启动直接跳过。
#[cfg(feature = "sled-migrate")]
async fn migrate_from_sled_if_empty(
    pool: &SqlitePool,
    db_path: &std::path::Path,
) -> Result<(), MemoryError> {
    let Some((mem_dir, vec_dir)) = legacy_sled_dirs(db_path) else {
        return Ok(());
    };
    let mem_empty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memories")
        .fetch_one(pool)
        .await?;
    let emb_empty: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM embeddings")
        .fetch_one(pool)
        .await?;

    // sled 是同步阻塞 API：先在 blocking 线程把行全部读出，再异步写入 sqlite。
    let mem_rows: Vec<(String, String, Vec<u8>)> = if mem_empty == 0 && mem_dir.is_dir() {
        let dir = mem_dir.clone();
        tokio::task::spawn_blocking(
            move || -> Result<Vec<(String, String, Vec<u8>)>, MemoryError> {
                let db = sled::open(&dir)?;
                let mut out = Vec::new();
                for t in MemoryType::ALL {
                    let tree = db.open_tree(t.as_storage_str())?;
                    for item in tree.iter() {
                        let (k, v) = item?;
                        let Ok(id) = std::str::from_utf8(&k) else {
                            debug!(
                                "sled-migrate: skip non-utf8 key in tree {}",
                                t.as_storage_str()
                            );
                            continue;
                        };
                        out.push((t.as_storage_str().to_string(), id.to_string(), v.to_vec()));
                    }
                }
                Ok(out)
            },
        )
        .await
        .map_err(|e| MemoryError::Migration(e.to_string()))??
    } else {
        Vec::new()
    };

    let emb_rows: Vec<(u8, String, Vec<u8>)> = if emb_empty == 0 && vec_dir.is_dir() {
        let dir = vec_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<Vec<(u8, String, Vec<u8>)>, MemoryError> {
            let db = sled::open(&dir)?;
            let mut out = Vec::new();
            for item in db.iter() {
                let (k, v) = item?;
                if k.is_empty() {
                    continue;
                }
                let Ok(id) = std::str::from_utf8(&k[1..]) else {
                    debug!("sled-migrate: skip non-utf8 vector key");
                    continue;
                };
                out.push((k[0], id.to_string(), v.to_vec()));
            }
            Ok(out)
        })
        .await
        .map_err(|e| MemoryError::Migration(e.to_string()))??
    } else {
        Vec::new()
    };

    if mem_rows.is_empty() && emb_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    for (mem_type, id, payload) in &mem_rows {
        sqlx::query("INSERT OR REPLACE INTO memories (mem_type, id, payload) VALUES (?1, ?2, ?3)")
            .bind(mem_type)
            .bind(id)
            .bind(payload)
            .execute(&mut *tx)
            .await?;
    }
    for (mem_type, id, payload) in &emb_rows {
        sqlx::query(
            "INSERT OR REPLACE INTO embeddings (mem_type, id, payload) VALUES (?1, ?2, ?3)",
        )
        .bind(*mem_type as i64)
        .bind(id)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    tracing::info!(
        target: "anycode_memory",
        memories = mem_rows.len(),
        embeddings = emb_rows.len(),
        "migrated legacy sled hot store into sqlite"
    );
    Ok(())
}

// ============================================================================
// Sqlite Memory Store（语义对齐 SledMemoryStore）
// ============================================================================

pub struct SqliteMemoryStore {
    pool: SqlitePool,
    #[cfg(feature = "sled-migrate")]
    db_path: PathBuf,
    init: tokio::sync::OnceCell<()>,
    cache: Arc<Cache<String, Memory>>,
}

impl SqliteMemoryStore {
    /// 同步惰性打开（与 `SledMemoryStore::new` 对齐）：首次异步操作时建表/迁移。
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let db_path: PathBuf = db_path.into();
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let pool = lazy_pool(&db_path);
        let cache = Cache::builder()
            .max_capacity(1000)
            .time_to_live(std::time::Duration::from_secs(600))
            .build();
        Ok(Self {
            pool,
            #[cfg(feature = "sled-migrate")]
            db_path,
            init: tokio::sync::OnceCell::new(),
            cache: Arc::new(cache),
        })
    }

    async fn ensure_init(&self) -> Result<(), MemoryError> {
        let pool = self.pool.clone();
        #[cfg(feature = "sled-migrate")]
        let db_path = self.db_path.clone();
        self.init
            .get_or_try_init(|| async move {
                create_schema(&pool).await?;
                #[cfg(feature = "sled-migrate")]
                migrate_from_sled_if_empty(&pool, &db_path).await?;
                Ok::<(), MemoryError>(())
            })
            .await?;
        Ok(())
    }

    /// 按键读取单条记忆（热层）。语义对齐 `SledMemoryStore::get_by_id`。
    pub async fn get_by_id(
        &self,
        id: &str,
        mem_type: &MemoryType,
    ) -> Result<Option<Memory>, MemoryError> {
        self.ensure_init().await?;
        let row = sqlx::query("SELECT payload FROM memories WHERE mem_type = ?1 AND id = ?2")
            .bind(mem_type.as_storage_str())
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let payload: Vec<u8> = row.try_get(0)?;
        Ok(Some(serde_json::from_slice(&payload)?))
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn save(&self, memory: Memory) -> Result<(), CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        let payload = serde_json::to_vec(&memory)?;
        sqlx::query("INSERT OR REPLACE INTO memories (mem_type, id, payload) VALUES (?1, ?2, ?3)")
            .bind(memory.mem_type.as_storage_str())
            .bind(&memory.id)
            .bind(payload)
            .execute(&self.pool)
            .await
            .map_err(core_err)?;

        self.cache.insert(memory.id.clone(), memory).await;
        Ok(())
    }

    async fn recall(&self, query: &str, mem_type: MemoryType) -> Result<Vec<Memory>, CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        let rows = sqlx::query("SELECT payload FROM memories WHERE mem_type = ?1 ORDER BY id")
            .bind(mem_type.as_storage_str())
            .fetch_all(&self.pool)
            .await
            .map_err(core_err)?;

        let mut memories = Vec::new();
        for row in rows {
            let payload: Vec<u8> = row.try_get(0).map_err(core_err)?;
            match serde_json::from_slice::<Memory>(&payload) {
                Ok(memory) => {
                    if memory.content.contains(query)
                        || memory.title.contains(query)
                        || memory.tags.iter().any(|t| t.contains(query))
                    {
                        memories.push(memory);
                    }
                }
                Err(e) => {
                    debug!("Failed to deserialize memory: {:?}", e);
                }
            }
        }

        Ok(KeywordRetrieval.rank(query, memories))
    }

    async fn update(&self, _id: &str, memory: Memory) -> Result<(), CoreError> {
        self.save(memory).await
    }

    async fn delete(&self, id: &str) -> Result<(), CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        for mem_type in MemoryType::ALL {
            let res = sqlx::query("DELETE FROM memories WHERE mem_type = ?1 AND id = ?2")
                .bind(mem_type.as_storage_str())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(core_err)?;
            if res.rows_affected() > 0 {
                self.cache.invalidate(&id.to_string()).await;
                return Ok(());
            }
        }
        Ok(())
    }
}

// ============================================================================
// Sqlite Vector Backend（语义对齐 SledVectorBackend）
// ============================================================================

pub struct SqliteVectorBackend {
    pool: SqlitePool,
    init: tokio::sync::OnceCell<()>,
}

impl SqliteVectorBackend {
    /// 与热层共用同一 `.db` 文件（`embeddings` 表）。
    pub fn new(db_path: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let db_path: PathBuf = db_path.into();
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        Ok(Self {
            pool: lazy_pool(&db_path),
            init: tokio::sync::OnceCell::new(),
        })
    }

    async fn ensure_init(&self) -> Result<(), MemoryError> {
        let pool = self.pool.clone();
        self.init
            .get_or_try_init(|| async move { create_schema(&pool).await })
            .await?;
        Ok(())
    }
}

#[async_trait]
impl VectorMemoryBackend for SqliteVectorBackend {
    async fn upsert(
        &self,
        id: &str,
        embedding: &[f32],
        mem_type: MemoryType,
    ) -> Result<(), CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        let rec = StoredEmb {
            id: id.to_string(),
            mem_type,
            vec: embedding.to_vec(),
        };
        let payload = serde_json::to_vec(&rec)?;
        sqlx::query(
            "INSERT OR REPLACE INTO embeddings (mem_type, id, payload) VALUES (?1, ?2, ?3)",
        )
        .bind(mem_type.discriminant() as i64)
        .bind(id)
        .bind(payload)
        .execute(&self.pool)
        .await
        .map_err(core_err)?;
        Ok(())
    }

    async fn search(
        &self,
        query_embedding: &[f32],
        mem_type: MemoryType,
        limit: usize,
    ) -> Result<Vec<String>, CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        let rows = sqlx::query("SELECT payload FROM embeddings WHERE mem_type = ?1 ORDER BY id")
            .bind(mem_type.discriminant() as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(core_err)?;

        let mut scored: Vec<(f32, String)> = Vec::new();
        for row in rows {
            let payload: Vec<u8> = row.try_get(0).map_err(core_err)?;
            let rec: StoredEmb = serde_json::from_slice(&payload)?;
            let s = cosine(query_embedding, &rec.vec);
            if s.is_finite() {
                scored.push((s, rec.id));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(scored
            .into_iter()
            .take(limit.max(1))
            .map(|(_, id)| id)
            .collect())
    }

    async fn remove(&self, id: &str, mem_type: MemoryType) -> Result<(), CoreError> {
        self.ensure_init().await.map_err(core_err)?;
        sqlx::query("DELETE FROM embeddings WHERE mem_type = ?1 AND id = ?2")
            .bind(mem_type.discriminant() as i64)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(core_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "anycode-sqlite-test-{}-{}.hot.db",
            name,
            uuid::Uuid::new_v4()
        ))
    }

    fn mem(id: &str, title: &str, content: &str, tags: &[&str], mem_type: MemoryType) -> Memory {
        Memory {
            id: id.to_string(),
            mem_type,
            title: title.to_string(),
            content: content.to_string(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            scope: MemoryScope::Project,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            meta: None,
        }
    }

    #[tokio::test]
    async fn save_recall_filters_and_ranks_like_sled() {
        let db = temp_db("recall");
        let store = SqliteMemoryStore::new(&db).unwrap();
        store
            .save(mem(
                "a",
                "misc",
                "contains alpha in content",
                &[],
                MemoryType::Project,
            ))
            .await
            .unwrap();
        store
            .save(mem("b", "alpha title", "other", &[], MemoryType::Project))
            .await
            .unwrap();
        store
            .save(mem("c", "misc2", "other", &["alpha"], MemoryType::Project))
            .await
            .unwrap();
        store
            .save(mem("d", "alpha user", "alpha", &[], MemoryType::User))
            .await
            .unwrap();

        let out = store.recall("alpha", MemoryType::Project).await.unwrap();
        let ids: Vec<&str> = out.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["b", "a", "c"], "title > content > tags");

        let none = store
            .recall("zzz-no-match", MemoryType::Project)
            .await
            .unwrap();
        assert!(none.is_empty());
        // mem_type 隔离
        let user = store.recall("alpha", MemoryType::User).await.unwrap();
        assert_eq!(user.len(), 1);
        assert_eq!(user[0].id, "d");
    }

    #[tokio::test]
    async fn save_overwrites_and_get_by_id_roundtrip() {
        let db = temp_db("overwrite");
        let store = SqliteMemoryStore::new(&db).unwrap();
        store
            .save(mem("x", "v1", "body_v1", &[], MemoryType::Project))
            .await
            .unwrap();
        store
            .save(mem("x", "v2", "body_v2", &[], MemoryType::Project))
            .await
            .unwrap();
        let got = store
            .get_by_id("x", &MemoryType::Project)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.title, "v2");
        assert!(got.content.contains("body_v2"));
    }

    #[tokio::test]
    async fn update_delegates_to_save_and_delete_is_idempotent() {
        let db = temp_db("update");
        let store = SqliteMemoryStore::new(&db).unwrap();
        store
            .save(mem("u", "t", "before_token", &[], MemoryType::Project))
            .await
            .unwrap();
        // update 忽略传入 id（与 sled 版一致），按 memory.id 覆盖
        store
            .update(
                "ignored",
                mem("u", "t2", "after_token", &[], MemoryType::Project),
            )
            .await
            .unwrap();
        let got = store
            .get_by_id("u", &MemoryType::Project)
            .await
            .unwrap()
            .unwrap();
        assert!(got.content.contains("after_token"));

        store.delete("u").await.unwrap();
        assert!(store
            .get_by_id("u", &MemoryType::Project)
            .await
            .unwrap()
            .is_none());
        // sled 语义：删除不存在的 id 也是 Ok
        store.delete("u").await.unwrap();
        store.delete("never-existed").await.unwrap();
    }

    #[tokio::test]
    async fn vector_upsert_search_remove_like_sled() {
        let db = temp_db("vec");
        let vec = SqliteVectorBackend::new(&db).unwrap();
        vec.upsert("p1", &[1.0, 0.0, 0.0], MemoryType::Project)
            .await
            .unwrap();
        vec.upsert("p2", &[0.9, 0.1, 0.0], MemoryType::Project)
            .await
            .unwrap();
        vec.upsert("p3", &[0.0, 1.0, 0.0], MemoryType::Project)
            .await
            .unwrap();
        vec.upsert("u1", &[1.0, 0.0, 0.0], MemoryType::User)
            .await
            .unwrap();

        let hits = vec
            .search(&[1.0, 0.0, 0.0], MemoryType::Project, 10)
            .await
            .unwrap();
        assert_eq!(hits, vec!["p1", "p2", "p3"], "余弦降序 + mem_type 隔离");

        // take(limit.max(1))：limit=0 仍返回 1 条
        let one = vec
            .search(&[1.0, 0.0, 0.0], MemoryType::Project, 0)
            .await
            .unwrap();
        assert_eq!(one.len(), 1);

        // 覆盖同 key
        vec.upsert("p3", &[1.0, 0.0, 0.0], MemoryType::Project)
            .await
            .unwrap();
        let hits = vec
            .search(&[1.0, 0.0, 0.0], MemoryType::Project, 1)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);

        vec.remove("p1", MemoryType::Project).await.unwrap();
        let hits = vec
            .search(&[1.0, 0.0, 0.0], MemoryType::Project, 10)
            .await
            .unwrap();
        assert!(!hits.contains(&"p1".to_string()));
        // 维度不匹配（cosine NaN）被跳过
        vec.upsert("bad", &[1.0], MemoryType::Project)
            .await
            .unwrap();
        let hits = vec
            .search(&[1.0, 0.0, 0.0], MemoryType::Project, 10)
            .await
            .unwrap();
        assert!(!hits.contains(&"bad".to_string()));
    }

    #[cfg(feature = "sled-migrate")]
    #[tokio::test]
    async fn migrates_legacy_sled_once() {
        let base =
            std::env::temp_dir().join(format!("anycode-migrate-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&base).unwrap();
        let db_path = base.join("memory.pipeline.hot.db");
        let mem_sled = base.join("memory.pipeline.sled");
        let vec_sled = base.join("memory.pipeline.vec.sled");

        // 造旧 sled 数据
        {
            let legacy = crate::SledMemoryStore::new(&mem_sled).unwrap();
            MemoryStore::save(
                &legacy,
                mem(
                    "m1",
                    "legacy alpha title",
                    "legacy body",
                    &[],
                    MemoryType::Project,
                ),
            )
            .await
            .unwrap();
            let legacy_vec = crate::vector_sled::SledVectorBackend::new(&vec_sled).unwrap();
            legacy_vec
                .upsert("m1", &[1.0, 0.0], MemoryType::Project)
                .await
                .unwrap();
        }

        // 首次打开：迁移发生
        let store = SqliteMemoryStore::new(&db_path).unwrap();
        let out = store.recall("legacy", MemoryType::Project).await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "m1");
        let vec = SqliteVectorBackend::new(&db_path).unwrap();
        let hits = vec
            .search(&[1.0, 0.0], MemoryType::Project, 10)
            .await
            .unwrap();
        assert_eq!(hits, vec!["m1".to_string()]);

        // 二次启动：不重复迁移（写入新数据后重开，旧 sled 仍有 m1 但不覆盖/不重复）
        store
            .save(mem("m2", "new", "new body", &[], MemoryType::Project))
            .await
            .unwrap();
        drop(store);
        let store2 = SqliteMemoryStore::new(&db_path).unwrap();
        let out = store2.recall("", MemoryType::Project).await.unwrap();
        assert_eq!(out.len(), 2, "无重复迁移");
        let got = store2
            .get_by_id("m1", &MemoryType::Project)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(got.title, "legacy alpha title");

        let _ = std::fs::remove_dir_all(&base);
    }
}
