//! Generic namespaced JSON key-value landing zone (migration 019).
//!
//! This is the destination for loose `*.json` state files being migrated into
//! SQLite (roadmap P0.6/P1.6). Migration pattern ("dual-read playbook"):
//! read kv first, fall back to the legacy file, backfill kv on a file hit;
//! write through to both during the transition window. On a kv write failure
//! after a successful file write, delete the kv key (best effort) so the
//! fresher file wins on the next read instead of a stale kv row.

use super::DashboardDb;
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};

impl DashboardDb {
    /// Read a JSON value from `kv_store`. `None` when the key is absent or the
    /// stored payload no longer deserializes (a corrupt row must not brick the
    /// caller — it falls back to its legacy source and backfills).
    pub async fn kv_get<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<T>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT value_json FROM kv_store WHERE namespace = ? AND key = ?")
                .bind(namespace)
                .bind(key)
                .fetch_optional(self.pool())
                .await
                .context("read kv_store")?;
        match row {
            Some((json,)) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    /// Upsert a JSON value into `kv_store`.
    pub async fn kv_set<T: Serialize>(&self, namespace: &str, key: &str, value: &T) -> Result<()> {
        let json = serde_json::to_string(value).context("serialize kv value")?;
        sqlx::query(
            "INSERT INTO kv_store (namespace, key, value_json, updated_at) \
             VALUES (?, ?, ?, datetime('now')) \
             ON CONFLICT (namespace, key) DO UPDATE SET \
               value_json = excluded.value_json, updated_at = datetime('now')",
        )
        .bind(namespace)
        .bind(key)
        .bind(json)
        .execute(self.pool())
        .await
        .context("write kv_store")?;
        Ok(())
    }

    /// Remove a key. Used by the dual-write playbook to drop a stale kv row
    /// when the kv half of a write-through fails.
    pub async fn kv_delete(&self, namespace: &str, key: &str) -> Result<()> {
        sqlx::query("DELETE FROM kv_store WHERE namespace = ? AND key = ?")
            .bind(namespace)
            .bind(key)
            .execute(self.pool())
            .await
            .context("delete kv_store row")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    #[tokio::test]
    async fn kv_roundtrip_and_delete() {
        let dir = tempdir().unwrap();
        let db = super::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();

        let missing: Option<serde_json::Value> = db.kv_get("lan", "settings").await.unwrap();
        assert!(missing.is_none());

        db.kv_set("lan", "settings", &serde_json::json!({"port": 43181}))
            .await
            .unwrap();
        let got: Option<serde_json::Value> = db.kv_get("lan", "settings").await.unwrap();
        assert_eq!(got.unwrap()["port"], 43181);

        // Upsert overwrites.
        db.kv_set("lan", "settings", &serde_json::json!({"port": 50000}))
            .await
            .unwrap();
        let got: Option<serde_json::Value> = db.kv_get("lan", "settings").await.unwrap();
        assert_eq!(got.unwrap()["port"], 50000);

        // Namespaces isolate identical keys.
        db.kv_set("other", "settings", &serde_json::json!({"port": 1}))
            .await
            .unwrap();
        let got: Option<serde_json::Value> = db.kv_get("lan", "settings").await.unwrap();
        assert_eq!(got.unwrap()["port"], 50000);

        db.kv_delete("lan", "settings").await.unwrap();
        let gone: Option<serde_json::Value> = db.kv_get("lan", "settings").await.unwrap();
        assert!(gone.is_none());
    }
}
