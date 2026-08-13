//! Safe partial updates to `~/.anycode/config.json` from the dashboard.

pub use anycode_llm::config_file::{
    migrate_legacy_llm_section, patch_llm_config as patch_llm_config_inner, read_config_value,
    read_model_fallback, string_field, write_config_value, LlmConfigPatch as LlmConfigPatchInner,
};
pub use anycode_llm::{ModelFallbackConfig, ModelProfileFile, ModelsConfigFile};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LlmConfigPatchBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_credentials: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_agents: Option<HashMap<String, ModelProfileFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_agents_delete: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<ModelsConfigFile>,
}

impl From<&LlmConfigPatchBody> for LlmConfigPatchInner {
    fn from(body: &LlmConfigPatchBody) -> Self {
        let fallback = if body.fallback_provider.is_some()
            || body.fallback_model.is_some()
            || body.fallback_on.is_some()
        {
            Some(ModelFallbackConfig {
                provider: body.fallback_provider.clone(),
                model: body.fallback_model.clone(),
                on: body
                    .fallback_on
                    .as_deref()
                    .and_then(anycode_llm::FailoverTrigger::from_str_label)
                    .unwrap_or_default(),
            })
        } else {
            None
        };
        LlmConfigPatchInner {
            provider: body.provider.clone(),
            model: body.model.clone(),
            plan: body.plan.clone(),
            base_url: body.base_url.clone(),
            api_key: body.api_key.clone(),
            provider_credentials: body.provider_credentials.clone(),
            fallback,
            routing_agents: body.routing_agents.clone(),
            routing_agents_delete: body.routing_agents_delete.clone(),
            models: body.models.clone(),
            models_replace: false,
        }
    }
}

// Backward-compatible alias used by existing handlers.
pub type LlmConfigPatch = LlmConfigPatchBody;

pub fn patch_llm_config(patch: &LlmConfigPatchBody) -> Result<(PathBuf, Value)> {
    patch_llm_config_inner(None, &patch.into())
}

pub fn read_config_root() -> Result<(PathBuf, Value)> {
    read_config_value(None)
}

pub fn write_config_root(cfg: &Value) -> Result<PathBuf> {
    let (path, _) = read_config_value(None)?;
    write_config_value(&path, cfg)?;
    Ok(path)
}

// ---- P1.6: settings table mirror (dual-read window) ----
//
// The file stays the SSOT this window; the `settings` table mirrors top-level
// keys so dashboard consumers can read config through SQLite. P2.6 flips read
// priority and drops file writes. The original file is auto-backed up to
// `config.json.bak` before the first SQLite write so a migration bug can never
// lock the user out of their config.

/// Copy `config.json` to `config.json.bak` once (idempotent, no-op when the
/// backup already exists or the config file is absent).
pub fn backup_config_once(path: &std::path::Path) -> Result<Option<PathBuf>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bak = path.with_extension("json.bak");
    if bak.exists() {
        return Ok(None);
    }
    std::fs::copy(path, &bak)?;
    Ok(Some(bak))
}

/// Mirror the current config file into the `settings` table, backing the file
/// up first. Best-effort at call sites (log and continue on error).
pub async fn sync_settings_mirror(db: &crate::db::DashboardDb) -> Result<usize> {
    sync_settings_mirror_at(db, None).await
}

/// Path-injectable variant (tests; production passes `None`).
pub async fn sync_settings_mirror_at(
    db: &crate::db::DashboardDb,
    path: Option<&std::path::Path>,
) -> Result<usize> {
    let (path, cfg) = read_config_value(path)?;
    if let Some(bak) = backup_config_once(&path)? {
        tracing::info!(path = %bak.display(), "config.json backed up before first settings mirror write");
    }
    db.settings_sync_config(&cfg).await
}

/// Sync the mirror from an already-read config value (post-write refresh; the
/// one-time backup already ran on startup/first mirror write).
pub async fn settings_sync_only(db: &crate::db::DashboardDb, cfg: &Value) -> Result<usize> {
    db.settings_sync_config(cfg).await
}

/// Dual-read: serve the SQLite mirror when populated; otherwise read the file
/// and backfill the mirror (P0.6 playbook). The file remains authoritative —
/// the mirror only serves reads where a file read would be redundant.
pub async fn read_config_dual(db: &crate::db::DashboardDb) -> Result<Value> {
    read_config_dual_at(db, None).await
}

/// Path-injectable variant (tests; production passes `None`).
pub async fn read_config_dual_at(
    db: &crate::db::DashboardDb,
    path: Option<&std::path::Path>,
) -> Result<Value> {
    let mirrored = db.settings_all().await.unwrap_or(Value::Null);
    if mirrored.as_object().is_some_and(|o| !o.is_empty()) {
        return Ok(mirrored);
    }
    let (_, cfg) = read_config_value(path)?;
    if cfg.as_object().is_some_and(|o| !o.is_empty()) {
        let _ = sync_settings_mirror_at(db, path).await; // best-effort backfill
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_writes_flat_provider() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("HOME", dir.path());
        let (_, cfg) = patch_llm_config(&LlmConfigPatchBody {
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(
            cfg.get("provider").and_then(|v| v.as_str()),
            Some("anthropic")
        );
        assert!(cfg.get("llm").is_none());
    }

    #[test]
    fn backup_config_once_copies_then_noops() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        std::fs::write(&cfg_path, "{\"provider\":\"deepseek\"}").unwrap();
        let bak = backup_config_once(&cfg_path).unwrap().unwrap();
        assert!(bak.ends_with("config.json.bak"));
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "{\"provider\":\"deepseek\"}"
        );
        // Second call does not overwrite (config changed since).
        std::fs::write(&cfg_path, "{\"provider\":\"zai\"}").unwrap();
        assert!(backup_config_once(&cfg_path).unwrap().is_none());
        assert_eq!(
            std::fs::read_to_string(&bak).unwrap(),
            "{\"provider\":\"deepseek\"}"
        );
    }

    #[tokio::test]
    async fn read_config_dual_backfills_from_file_then_serves_mirror() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("config.json");
        std::fs::write(
            &cfg_path,
            "{\"provider\":\"deepseek\",\"model\":\"deepseek-v4-flash\"}",
        )
        .unwrap();
        let db = crate::db::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();
        // Mirror empty → file read + backfill (also creates config.json.bak).
        let cfg = read_config_dual_at(&db, Some(&cfg_path)).await.unwrap();
        assert_eq!(cfg["provider"], "deepseek");
        assert!(dir.path().join("config.json.bak").is_file());
        assert_eq!(
            db.settings_all().await.unwrap()["model"],
            "deepseek-v4-flash"
        );
        // File removed → mirror still serves.
        std::fs::remove_file(&cfg_path).unwrap();
        let cfg = read_config_dual_at(&db, Some(&cfg_path)).await.unwrap();
        assert_eq!(cfg["provider"], "deepseek");
    }
}
