//! Dashboard Web Skill App host (file IPC poll for present + brief).

use anycode_dashboard_ipc::approval_ipc::SESSION_ENV;
use anycode_dashboard_ipc::skill_app_ipc;
use anycode_tools::{
    SkillAppHost, SkillAppHostError, SkillAppPresentRequest, SkillAppPresentResponse,
};
use async_trait::async_trait;
use std::time::Duration;

const WEB_POLL_MS: u64 = 400;
const WEB_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct WorkbenchSkillAppHost;

impl WorkbenchSkillAppHost {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    fn session_id() -> Option<String> {
        anycode_core::current_dashboard_session_id()
            .or_else(|| std::env::var(SESSION_ENV).ok())
            .filter(|s| !s.is_empty())
    }

    fn user_turn_id() -> u32 {
        anycode_core::current_user_turn_id().unwrap_or(0)
    }

    async fn wait_brief(
        present_id: &str,
    ) -> Option<anycode_dashboard_ipc::skill_app_ipc::SkillAppBriefResponse> {
        let deadline = tokio::time::Instant::now() + WEB_TIMEOUT;
        let session_id = Self::session_id();
        loop {
            if let Some(resp) = skill_app_ipc::poll_response(present_id) {
                return Some(resp);
            }
            if skill_app_ipc::get_pending(present_id).is_none() {
                return None;
            }
            if session_id
                .as_deref()
                .is_some_and(anycode_dashboard_ipc::cancel_ipc::poll_cancel_requested)
            {
                skill_app_ipc::clear_pending(present_id);
                return None;
            }
            if tokio::time::Instant::now() >= deadline {
                skill_app_ipc::clear_pending(present_id);
                return None;
            }
            tokio::time::sleep(Duration::from_millis(WEB_POLL_MS)).await;
        }
    }
}

impl Default for WorkbenchSkillAppHost {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SkillAppHost for WorkbenchSkillAppHost {
    async fn present(
        &self,
        request: SkillAppPresentRequest,
    ) -> Result<SkillAppPresentResponse, SkillAppHostError> {
        let Some(sid) = Self::session_id() else {
            return Err(SkillAppHostError(
                "SkillAppPresent requires dashboard session".into(),
            ));
        };
        if !skill_app_ipc::web_skill_apps_enabled() {
            return Err(SkillAppHostError(
                "Web Skill Apps disabled (ANYCODE_DASHBOARD_WEB_SKILL_APP=0)".into(),
            ));
        }

        // Reuse an in-flight wait_brief present for the same session+skill so the
        // model calling SkillAppPresent does not nest a second studio.
        if request.wait_brief {
            if let Some(existing) = skill_app_ipc::find_wait_brief_pending(&sid, &request.skill_id)
            {
                tracing::info!(
                    target: "anycode_dashboard",
                    session_id = %sid,
                    present_id = %existing.present_id,
                    skill_id = %request.skill_id,
                    "SkillAppPresent reusing existing wait_brief present"
                );
                let resp = Self::wait_brief(&existing.present_id)
                    .await
                    .ok_or_else(|| {
                        SkillAppHostError("Skill App brief timed out or cancelled".into())
                    })?;
                return Ok(SkillAppPresentResponse {
                    present_id: resp.present_id,
                    brief: Some(resp.brief),
                });
            }
        }

        let present_id = skill_app_ipc::register_present(
            &sid,
            Self::user_turn_id(),
            &request.skill_id,
            &request.slot,
            request.wait_brief,
            request.push.clone(),
        )
        .map_err(|e| SkillAppHostError(e.to_string()))?;
        tracing::info!(
            target: "anycode_dashboard",
            session_id = %sid,
            present_id = %present_id,
            skill_id = %request.skill_id,
            wait_brief = request.wait_brief,
            "SkillAppPresent pending — open Skill App in Workbench"
        );
        if !request.wait_brief {
            return Ok(SkillAppPresentResponse {
                present_id,
                brief: None,
            });
        }
        let resp = Self::wait_brief(&present_id)
            .await
            .ok_or_else(|| SkillAppHostError("Skill App brief timed out or cancelled".into()))?;
        Ok(SkillAppPresentResponse {
            present_id: resp.present_id,
            brief: Some(resp.brief),
        })
    }

    async fn push(
        &self,
        skill_id: &str,
        _slot: &str,
        payload: serde_json::Value,
    ) -> Result<(), SkillAppHostError> {
        let Some(sid) = Self::session_id() else {
            return Err(SkillAppHostError(
                "SkillAppPush requires dashboard session".into(),
            ));
        };
        // Do not register_present — that re-opens a dismissed Skill App tab.
        // Progress is best-effort logged; the mini-app can read state on next open.
        tracing::debug!(
            target: "anycode_dashboard",
            session_id = %sid,
            skill_id,
            payload = %payload,
            "SkillAppPush acknowledged without re-presenting UI"
        );
        Ok(())
    }
}
