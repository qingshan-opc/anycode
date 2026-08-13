//! P1.8: persisted transcript diagrams backing the `/diagram/{id}` page.
//!
//! Ids are deterministic (`dgm_` + sha256(kind, source) prefix): rendering the
//! same diagram twice yields the same URL and `INSERT OR IGNORE` keeps storage
//! flat, so the UI can persist-on-first-render without a lookup round trip.

use super::*;
use sha2::{Digest, Sha256};

/// Diagram kinds the share page knows how to render.
pub const DIAGRAM_KINDS: [&str; 3] = ["mermaid", "mindmap", "math"];

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagramRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub kind: String,
    pub source: String,
    pub title: Option<String>,
    pub created_at: String,
}

#[must_use]
pub fn diagram_id(kind: &str, source: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\n");
    hasher.update(source.as_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(4 + 24);
    id.push_str("dgm_");
    for byte in &digest[..12] {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

impl DashboardDb {
    /// Persist a diagram (idempotent by content hash). Returns the stable id.
    pub async fn diagrams_save(
        &self,
        session_id: Option<&str>,
        kind: &str,
        source: &str,
        title: Option<&str>,
    ) -> Result<String> {
        anyhow::ensure!(
            DIAGRAM_KINDS.contains(&kind),
            "unsupported diagram kind: {kind}"
        );
        anyhow::ensure!(!source.trim().is_empty(), "diagram source is empty");
        anyhow::ensure!(source.len() <= 256 * 1024, "diagram source too large");
        let id = diagram_id(kind, source);
        sqlx::query(
            r#"INSERT OR IGNORE INTO diagrams (id, session_id, kind, source, title)
               VALUES (?, ?, ?, ?, ?)"#,
        )
        .bind(&id)
        .bind(session_id)
        .bind(kind)
        .bind(source)
        .bind(title)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    pub async fn diagrams_get(&self, id: &str) -> Result<Option<DiagramRecord>> {
        let row = sqlx::query(
            "SELECT id, session_id, kind, source, title, created_at FROM diagrams WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| DiagramRecord {
            id: r.get("id"),
            session_id: r.get("session_id"),
            kind: r.get("kind"),
            source: r.get("source"),
            title: r.get("title"),
            created_at: r.get("created_at"),
        }))
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn diagrams_save_is_idempotent_and_kind_gated() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();
        let id1 = db
            .diagrams_save(Some("sess-1"), "mermaid", "graph TD; A-->B", Some("flow"))
            .await
            .unwrap();
        assert!(id1.starts_with("dgm_"));
        // Same content → same id, no error, no duplicate.
        let id2 = db
            .diagrams_save(Some("sess-1"), "mermaid", "graph TD; A-->B", Some("flow"))
            .await
            .unwrap();
        assert_eq!(id1, id2);
        let rec = db.diagrams_get(&id1).await.unwrap().unwrap();
        assert_eq!(rec.kind, "mermaid");
        assert_eq!(rec.title.as_deref(), Some("flow"));
        assert_eq!(rec.session_id.as_deref(), Some("sess-1"));
        // Unknown kind rejected.
        assert!(db
            .diagrams_save(None, "graphviz", "digraph {}", None)
            .await
            .is_err());
        // Missing id → None.
        assert!(db.diagrams_get("dgm_nope").await.unwrap().is_none());
    }
}
