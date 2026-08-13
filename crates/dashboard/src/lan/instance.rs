//! Persistent LAN instance identity (`~/.anycode/lan/instance.json`).
//!
//! Storage is mid-migration to SQLite `kv_store` (namespace `"lan"`, roadmap
//! P0.6): reads prefer kv with the JSON file as fallback (backfilling kv on a
//! file hit), writes go to both for one version cycle.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::db::DashboardDb;

/// kv_store namespace for LAN identity/settings.
pub const LAN_KV_NAMESPACE: &str = "lan";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanInstance {
    pub instance_id: String,
    #[serde(default = "default_device_name")]
    pub device_name: String,
    #[serde(default = "chrono_now")]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanSettings {
    #[serde(default = "default_true")]
    pub discovery_enabled: bool,
    #[serde(default = "default_device_name")]
    pub display_name: String,
    #[serde(default = "default_lan_port")]
    pub lan_port: u16,
    #[serde(default = "default_max_bundle_mb")]
    pub max_bundle_mb: u64,
}

impl Default for LanSettings {
    fn default() -> Self {
        Self {
            discovery_enabled: true,
            display_name: default_device_name(),
            lan_port: default_lan_port(),
            max_bundle_mb: default_max_bundle_mb(),
        }
    }
}

impl LanSettings {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join("settings.json");
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, data_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(data_dir).context("create lan data dir")?;
        let path = data_dir.join("settings.json");
        let body = serde_json::to_string_pretty(self).context("serialize lan settings")?;
        std::fs::write(path, body).context("write lan settings")?;
        Ok(())
    }

    /// Dual-read: kv_store first, JSON file fallback. A file hit is backfilled
    /// into kv so the next boot is kv-served. Without a `db` this degrades to
    /// the file-only path.
    pub async fn load_dual(db: Option<&DashboardDb>, data_dir: &Path) -> Self {
        if let Some(db) = db {
            match db.kv_get::<LanSettings>(LAN_KV_NAMESPACE, "settings").await {
                Ok(Some(settings)) => return settings,
                Ok(None) => {
                    let from_file = Self::load(data_dir);
                    // Only backfill real user data — don't persist defaults a
                    // user never wrote.
                    if data_dir.join("settings.json").exists() {
                        let _ = db.kv_set(LAN_KV_NAMESPACE, "settings", &from_file).await;
                    }
                    return from_file;
                }
                Err(e) => {
                    tracing::warn!("lan settings kv read failed, using file: {e:#}");
                    return Self::load(data_dir);
                }
            }
        }
        Self::load(data_dir)
    }

    /// Write-through: file first (compatibility/export during the transition),
    /// then kv_store. If the kv half fails, drop any stale kv row so the
    /// fresher file wins on the next read instead of silently reverting the
    /// user's change.
    pub async fn save_dual(&self, db: Option<&DashboardDb>, data_dir: &Path) -> Result<()> {
        self.save(data_dir)?;
        if let Some(db) = db {
            if let Err(e) = db.kv_set(LAN_KV_NAMESPACE, "settings", self).await {
                tracing::warn!("lan settings kv write failed, clearing stale row: {e:#}");
                let _ = db.kv_delete(LAN_KV_NAMESPACE, "settings").await;
            }
        }
        Ok(())
    }
}

fn instance_path(data_dir: &Path) -> PathBuf {
    data_dir.join("instance.json")
}

pub fn load_or_create_instance(data_dir: &Path) -> LanInstance {
    let path = instance_path(data_dir);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(inst) = serde_json::from_str(&text) {
            return inst;
        }
    }
    let inst = LanInstance {
        instance_id: format!("lan_{}", Uuid::new_v4().simple()),
        device_name: default_device_name(),
        created_at: chrono_now(),
    };
    let _ = save_instance(data_dir, &inst);
    inst
}

pub fn save_instance(data_dir: &Path, inst: &LanInstance) -> Result<()> {
    std::fs::create_dir_all(data_dir).context("create lan data dir")?;
    let body = serde_json::to_string_pretty(inst).context("serialize lan instance")?;
    std::fs::write(instance_path(data_dir), body).context("write lan instance")?;
    Ok(())
}

/// Dual-read instance identity: kv_store first, JSON file fallback (backfilled
/// into kv), created fresh when neither exists. The instance id must stay
/// stable across stores — whichever source wins is mirrored to the other.
pub async fn load_or_create_instance_dual(
    db: Option<&DashboardDb>,
    data_dir: &Path,
) -> LanInstance {
    if let Some(db) = db {
        match db.kv_get::<LanInstance>(LAN_KV_NAMESPACE, "instance").await {
            Ok(Some(inst)) => return inst,
            Ok(None) => {}
            Err(e) => tracing::warn!("lan instance kv read failed, using file: {e:#}"),
        }
        let inst = load_or_create_instance(data_dir);
        let _ = db.kv_set(LAN_KV_NAMESPACE, "instance", &inst).await;
        return inst;
    }
    load_or_create_instance(data_dir)
}

/// Write-through instance save (see [`LanSettings::save_dual`] for the
/// stale-row rationale).
pub async fn save_instance_dual(
    db: Option<&DashboardDb>,
    data_dir: &Path,
    inst: &LanInstance,
) -> Result<()> {
    save_instance(data_dir, inst)?;
    if let Some(db) = db {
        if let Err(e) = db.kv_set(LAN_KV_NAMESPACE, "instance", inst).await {
            tracing::warn!("lan instance kv write failed, clearing stale row: {e:#}");
            let _ = db.kv_delete(LAN_KV_NAMESPACE, "instance").await;
        }
    }
    Ok(())
}

fn default_device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "AnyCode".into())
}

fn default_lan_port() -> u16 {
    43181
}

fn default_max_bundle_mb() -> u64 {
    500
}

fn default_true() -> bool {
    true
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn test_db(dir: &Path) -> DashboardDb {
        DashboardDb::open(dir.join("t.db")).await.unwrap()
    }

    #[tokio::test]
    async fn settings_file_fallback_backfills_kv() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("lan");
        let db = test_db(dir.path()).await;

        // Seed only the legacy file.
        let seeded = LanSettings {
            display_name: "File Box".into(),
            lan_port: 43222,
            ..Default::default()
        };
        seeded.save(&data_dir).unwrap();

        // Dual-read serves the file value…
        let got = LanSettings::load_dual(Some(&db), &data_dir).await;
        assert_eq!(got.display_name, "File Box");
        assert_eq!(got.lan_port, 43222);
        // …and backfills kv.
        let kv: Option<LanSettings> = db.kv_get(LAN_KV_NAMESPACE, "settings").await.unwrap();
        assert_eq!(kv.unwrap().lan_port, 43222);

        // kv wins once populated: deleting the file must not lose settings.
        std::fs::remove_file(data_dir.join("settings.json")).unwrap();
        let got = LanSettings::load_dual(Some(&db), &data_dir).await;
        assert_eq!(got.display_name, "File Box");
    }

    #[tokio::test]
    async fn settings_save_dual_writes_both_stores() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("lan");
        let db = test_db(dir.path()).await;

        let settings = LanSettings {
            display_name: "Dual Box".into(),
            ..Default::default()
        };
        settings.save_dual(Some(&db), &data_dir).await.unwrap();

        let kv: Option<LanSettings> = db.kv_get(LAN_KV_NAMESPACE, "settings").await.unwrap();
        assert_eq!(kv.unwrap().display_name, "Dual Box");
        assert_eq!(
            LanSettings::load(&data_dir).display_name,
            "Dual Box",
            "file mirror must stay in sync during the transition window"
        );
    }

    #[tokio::test]
    async fn instance_identity_stable_across_stores() {
        let dir = tempdir().unwrap();
        let data_dir = dir.path().join("lan");
        let db = test_db(dir.path()).await;

        let first = load_or_create_instance_dual(Some(&db), &data_dir).await;
        assert!(data_dir.join("instance.json").exists());

        // kv is authoritative once populated — even with the file gone the
        // identity must not regenerate.
        std::fs::remove_file(data_dir.join("instance.json")).unwrap();
        let second = load_or_create_instance_dual(Some(&db), &data_dir).await;
        assert_eq!(first.instance_id, second.instance_id);

        // Fresh dir with no db → file-only path still works.
        let plain = load_or_create_instance_dual(None, &data_dir).await;
        assert!(plain.instance_id.starts_with("lan_"));
    }
}
