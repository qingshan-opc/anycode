//! P1.6: flattened `config.json` mirror in the `settings` table.
//!
//! Dual-read window: the file is still the SSOT; the table mirrors top-level
//! keys so dashboard consumers can read config through SQLite, and P2.6 can
//! flip read priority + drop file writes without a data migration.

use super::*;

impl DashboardDb {
    /// Mirror top-level config.json keys into `settings`. Values keep their
    /// JSON shape (nested objects stored as JSON text, not recursed).
    pub async fn settings_sync_config(&self, cfg: &Value) -> Result<usize> {
        let Some(obj) = cfg.as_object() else {
            return Ok(0);
        };
        let mut tx = self.pool.begin().await?;
        for (key, value) in obj {
            sqlx::query(
                r#"INSERT INTO settings (key, value_json, updated_at)
                   VALUES (?, ?, datetime('now'))
                   ON CONFLICT(key) DO UPDATE SET
                     value_json = excluded.value_json,
                     updated_at = datetime('now')"#,
            )
            .bind(key)
            .bind(value.to_string())
            .execute(&mut *tx)
            .await?;
        }
        if obj.is_empty() {
            sqlx::query("DELETE FROM settings")
                .execute(&mut *tx)
                .await?;
        } else {
            let placeholders = vec!["?"; obj.len()].join(", ");
            let sql = format!("DELETE FROM settings WHERE key NOT IN ({placeholders})");
            let mut q = sqlx::query(&sql);
            for key in obj.keys() {
                q = q.bind(key);
            }
            q.execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(obj.len())
    }

    /// Reassemble the mirrored config object (mirror read side).
    pub async fn settings_all(&self) -> Result<Value> {
        let rows = sqlx::query("SELECT key, value_json FROM settings")
            .fetch_all(&self.pool)
            .await?;
        let mut map = serde_json::Map::new();
        for r in rows {
            let key: String = r.get("key");
            let raw: String = r.get("value_json");
            map.insert(
                key,
                serde_json::from_str(&raw).unwrap_or(Value::String(raw)),
            );
        }
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn settings_mirror_roundtrips_and_drops_stale_keys() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();
        let cfg = serde_json::json!({
            "provider": "deepseek",
            "model": "deepseek-v4-flash",
            "mcp": { "servers": [] }
        });
        assert_eq!(db.settings_sync_config(&cfg).await.unwrap(), 3);
        let mirrored = db.settings_all().await.unwrap();
        assert_eq!(mirrored["provider"], "deepseek");
        assert_eq!(mirrored["mcp"]["servers"], serde_json::json!([]));

        // Next sync drops removed keys and updates changed ones.
        let cfg2 = serde_json::json!({ "provider": "zai" });
        db.settings_sync_config(&cfg2).await.unwrap();
        let mirrored = db.settings_all().await.unwrap();
        assert_eq!(mirrored["provider"], "zai");
        assert!(mirrored.get("model").is_none());
        assert!(mirrored.get("mcp").is_none());
    }
}
