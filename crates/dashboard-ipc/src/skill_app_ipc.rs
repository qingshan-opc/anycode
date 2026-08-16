//! File-based Skill App present / brief wait between Agent and Workbench.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;
use uuid::Uuid;

type RegisterHook = Box<dyn Fn(&PendingSkillAppPresent) + Send + Sync>;
static REGISTER_HOOK: OnceLock<Mutex<Option<RegisterHook>>> = OnceLock::new();

pub fn set_register_hook(hook: RegisterHook) {
    let slot = REGISTER_HOOK.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = slot.lock() {
        *guard = Some(hook);
    }
}

fn invoke_register_hook(rec: &PendingSkillAppPresent) {
    let Some(slot) = REGISTER_HOOK.get() else {
        return;
    };
    if let Ok(guard) = slot.lock() {
        if let Some(hook) = guard.as_ref() {
            hook(rec);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingSkillAppPresent {
    pub present_id: String,
    pub session_id: String,
    #[serde(default)]
    pub user_turn_id: u32,
    pub skill_id: String,
    /// dock | conversation | project
    #[serde(default = "default_slot")]
    pub slot: String,
    /// When true, Agent waits for VisualBrief submission.
    #[serde(default)]
    pub wait_brief: bool,
    #[serde(default)]
    pub push: Option<serde_json::Value>,
    pub created_at: String,
    pub status: String,
}

fn default_slot() -> String {
    "dock".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAppBriefResponse {
    pub present_id: String,
    pub skill_id: String,
    pub brief: serde_json::Value,
    pub responded_at: String,
}

#[must_use]
pub fn web_skill_apps_enabled() -> bool {
    !matches!(
        std::env::var("ANYCODE_DASHBOARD_WEB_SKILL_APP").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    )
}

fn pending_dir() -> PathBuf {
    crate::cancel_ipc::dashboard_state_dir().join("skill-apps/pending")
}

fn response_dir() -> PathBuf {
    crate::cancel_ipc::dashboard_state_dir().join("skill-apps/responses")
}

pub fn register_present(
    session_id: &str,
    user_turn_id: u32,
    skill_id: &str,
    slot: &str,
    wait_brief: bool,
    push: Option<serde_json::Value>,
) -> Result<String> {
    if skill_id.trim().is_empty() {
        bail!("SkillAppPresent requires skill_id");
    }
    std::fs::create_dir_all(pending_dir())?;
    let present_id = format!("sa_{}", Uuid::new_v4().simple());
    let rec = PendingSkillAppPresent {
        present_id: present_id.clone(),
        session_id: session_id.to_string(),
        user_turn_id,
        skill_id: skill_id.to_string(),
        slot: slot.to_string(),
        wait_brief,
        push,
        created_at: chrono::Utc::now().to_rfc3339(),
        status: "pending".into(),
    };
    let path = pending_dir().join(format!("{present_id}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&rec)?)?;
    invoke_register_hook(&rec);
    Ok(present_id)
}

pub fn get_pending(present_id: &str) -> Option<PendingSkillAppPresent> {
    let path = pending_dir().join(format!("{present_id}.json"));
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn list_pending_for_session(
    session_id: Option<&str>,
    limit: usize,
) -> Vec<PendingSkillAppPresent> {
    let _ = sweep_stale(STALE_MAX_AGE_SECS);
    let dir = pending_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut rows: Vec<(std::time::SystemTime, PendingSkillAppPresent)> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let raw = std::fs::read_to_string(e.path()).ok()?;
            let rec: PendingSkillAppPresent = serde_json::from_str(&raw).ok()?;
            if rec.status != "pending" {
                return None;
            }
            if let Some(sid) = session_id {
                if rec.session_id != sid {
                    return None;
                }
            }
            Some((meta.modified().ok().unwrap_or(UNIX_EPOCH), rec))
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    rows.into_iter().take(limit).map(|(_, r)| r).collect()
}

pub fn clear_pending(present_id: &str) {
    let _ = std::fs::remove_file(pending_dir().join(format!("{present_id}.json")));
}

/// Existing wait_brief present for this session+skill, if any (reuse instead of nesting studios).
#[must_use]
pub fn find_wait_brief_pending(session_id: &str, skill_id: &str) -> Option<PendingSkillAppPresent> {
    list_pending_for_session(Some(session_id), 50)
        .into_iter()
        .find(|p| p.skill_id == skill_id && p.wait_brief)
}

pub fn submit_brief(present_id: &str, skill_id: &str, brief: serde_json::Value) -> Result<()> {
    let Some(pending) = get_pending(present_id) else {
        // Already consumed by the waiting chat turn — treat as success.
        return Ok(());
    };
    if pending.skill_id != skill_id {
        bail!("skill_id mismatch");
    }
    std::fs::create_dir_all(response_dir())?;
    let resp = SkillAppBriefResponse {
        present_id: present_id.to_string(),
        skill_id: skill_id.to_string(),
        brief,
        responded_at: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::write(
        response_dir().join(format!("{present_id}.json")),
        serde_json::to_string_pretty(&resp)?,
    )?;
    clear_pending(present_id);
    Ok(())
}

pub fn poll_response(present_id: &str) -> Option<SkillAppBriefResponse> {
    let path = response_dir().join(format!("{present_id}.json"));
    let raw = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(path);
    serde_json::from_str(&raw).ok()
}

const STALE_MAX_AGE_SECS: u64 = 60 * 60;

fn sweep_stale(max_age_secs: u64) -> usize {
    let dir = pending_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut n = 0;
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Ok(meta) = e.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let age = now
            .duration_since(modified)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if age > max_age_secs {
            let _ = std::fs::remove_file(path);
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util;
    use serde_json::json;
    use tempfile::tempdir;

    fn test_state(dir: &tempfile::TempDir) {
        std::env::set_var("ANYCODE_DASHBOARD_STATE_DIR", dir.path().join("dashboard"));
    }

    #[test]
    fn brief_roundtrip_unblocks_waiter() {
        let _guard = test_util::lock_state_dir_env();
        let dir = tempdir().unwrap();
        test_state(&dir);
        let id =
            register_present("sess_ppt", 0, "anycode-ppt", "conversation", true, None).unwrap();
        assert!(get_pending(&id).unwrap().wait_brief);
        submit_brief(&id, "anycode-ppt", json!({"family": "apple-keynote"})).unwrap();
        assert!(get_pending(&id).is_none());
        let resp = poll_response(&id).unwrap();
        assert_eq!(resp.brief["family"], "apple-keynote");
        submit_brief(&id, "anycode-ppt", json!({"family": "ignored"})).unwrap();
    }

    #[test]
    fn find_wait_brief_pending_reuses_same_session_skill() {
        let _guard = test_util::lock_state_dir_env();
        let dir = tempdir().unwrap();
        test_state(&dir);
        let id =
            register_present("sess_ppt", 0, "anycode-ppt", "conversation", true, None).unwrap();
        let _ =
            register_present("sess_ppt", 0, "anycode-ppt", "conversation", false, None).unwrap();
        let found = find_wait_brief_pending("sess_ppt", "anycode-ppt").unwrap();
        assert_eq!(found.present_id, id);
        assert!(found.wait_brief);
        assert!(find_wait_brief_pending("other", "anycode-ppt").is_none());
    }
}
