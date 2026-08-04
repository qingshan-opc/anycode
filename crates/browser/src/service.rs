//! Shared browser session registry for agents and workbench.

use crate::error::{BrowserError, BrowserResult};
use crate::session_actor::SessionActorHandle;
use crate::types::{
    BrowserHitTestResult, BrowserScreenshot, BrowserSessionInfo, BrowserSnapshot, BrowserState,
    BrowserTabInfo, BrowserViewport, LockHolder, ScreencastFrame, ViewportSpec,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

struct LiveSession {
    info: BrowserSessionInfo,
    actor: SessionActorHandle,
    screencast: Option<broadcast::Sender<ScreencastFrame>>,
}

#[derive(Clone, Default)]
pub struct BrowserService {
    inner: Arc<RwLock<HashMap<String, LiveSession>>>,
}

impl BrowserService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shared() -> Arc<Self> {
        use std::sync::OnceLock;
        static INSTANCE: OnceLock<Arc<BrowserService>> = OnceLock::new();
        INSTANCE
            .get_or_init(|| Arc::new(BrowserService::new()))
            .clone()
    }

    async fn actor(&self, session_id: &str) -> BrowserResult<SessionActorHandle> {
        let guard = self.inner.read().await;
        guard
            .get(session_id)
            .map(|s| s.actor.clone())
            .ok_or_else(|| BrowserError::SessionNotFound(session_id.to_string()))
    }

    pub async fn create_session(
        &self,
        project_id: &str,
        conversation_id: Option<&str>,
        bind_key: Option<&str>,
        viewport: Option<ViewportSpec>,
    ) -> BrowserResult<BrowserSessionInfo> {
        let session_id = bind_key
            .map(str::to_string)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        {
            let guard = self.inner.read().await;
            if let Some(existing) = guard.get(&session_id) {
                if let Some(vp) = viewport {
                    let actor = existing.actor.clone();
                    drop(guard);
                    let _ = actor
                        .set_viewport(vp.width, vp.height, vp.device_scale_factor)
                        .await;
                }
                let guard = self.inner.read().await;
                if let Some(existing) = guard.get(&session_id) {
                    return Ok(existing.info.clone());
                }
                return Err(BrowserError::SessionNotFound(session_id));
            }
        }
        // Panel open + MCP mirror (or React Strict Mode) can race; spawn is
        // expensive — only one Chromium per bind key may win the insert.
        let actor = SessionActorHandle::spawn(viewport).await?;
        let info = BrowserSessionInfo {
            session_id: session_id.clone(),
            project_id: project_id.to_string(),
            conversation_id: conversation_id.map(str::to_string),
        };
        {
            let mut guard = self.inner.write().await;
            if let Some(existing) = guard.get(&session_id) {
                let kept = existing.info.clone();
                let existing_actor = existing.actor.clone();
                drop(guard);
                actor.shutdown().await;
                if let Some(vp) = viewport {
                    let _ = existing_actor
                        .set_viewport(vp.width, vp.height, vp.device_scale_factor)
                        .await;
                }
                return Ok(kept);
            }
            guard.insert(
                session_id,
                LiveSession {
                    info: info.clone(),
                    actor,
                    screencast: None,
                },
            );
        }
        Ok(info)
    }

    pub async fn set_viewport(
        &self,
        session_id: &str,
        width: u32,
        height: u32,
        device_scale_factor: f64,
    ) -> BrowserResult<BrowserViewport> {
        self.actor(session_id)
            .await?
            .set_viewport(width, height, device_scale_factor)
            .await
    }

    pub async fn close_session(&self, session_id: &str) -> BrowserResult<()> {
        let mut guard = self.inner.write().await;
        if let Some(session) = guard.remove(session_id) {
            session.actor.shutdown().await;
        }
        Ok(())
    }

    pub async fn list_tabs(&self, session_id: &str) -> BrowserResult<Vec<BrowserTabInfo>> {
        self.actor(session_id).await?.list_tabs().await
    }

    pub async fn new_tab(&self, session_id: &str) -> BrowserResult<String> {
        self.actor(session_id).await?.new_tab().await
    }

    pub async fn close_tab(&self, session_id: &str, tab_id: &str) -> BrowserResult<()> {
        self.actor(session_id).await?.close_tab(tab_id).await
    }

    pub async fn select_tab(&self, session_id: &str, tab_id: &str) -> BrowserResult<()> {
        self.actor(session_id).await?.select_tab(tab_id).await
    }

    pub async fn navigate(&self, session_id: &str, url: &str) -> BrowserResult<BrowserState> {
        self.agent_lock(session_id).await?;
        self.actor(session_id).await?.navigate(url).await
    }

    pub async fn console_and_network(
        &self,
        session_id: &str,
        limit: usize,
    ) -> BrowserResult<serde_json::Value> {
        self.agent_lock(session_id).await?;
        self.actor(session_id)
            .await?
            .console_and_network(limit)
            .await
    }

    pub async fn navigate_user(&self, session_id: &str, url: &str) -> BrowserResult<BrowserState> {
        self.actor(session_id).await?.navigate(url).await
    }

    pub async fn state(&self, session_id: &str) -> BrowserResult<BrowserState> {
        self.actor(session_id).await?.state().await
    }

    pub async fn snapshot(
        &self,
        session_id: &str,
        root_ref: Option<&str>,
    ) -> BrowserResult<BrowserSnapshot> {
        self.agent_lock(session_id).await?;
        self.actor(session_id).await?.snapshot(root_ref).await
    }

    pub async fn screenshot(&self, session_id: &str) -> BrowserResult<BrowserScreenshot> {
        self.actor(session_id).await?.screenshot().await
    }

    /// Hit-test at viewport CSS pixels (screencast device coordinates).
    pub async fn hit_test(
        &self,
        session_id: &str,
        x: f64,
        y: f64,
    ) -> BrowserResult<Option<BrowserHitTestResult>> {
        self.actor(session_id).await?.hit_test(x, y).await
    }

    pub async fn set_design_mode(&self, session_id: &str, enabled: bool) -> BrowserResult<()> {
        self.actor(session_id).await?.set_design_mode(enabled).await
    }

    pub async fn poll_design_inspect(
        &self,
        session_id: &str,
    ) -> BrowserResult<crate::types::BrowserDesignInspectState> {
        self.actor(session_id).await?.poll_design_inspect().await
    }

    pub async fn click(&self, session_id: &str, ref_id: &str) -> BrowserResult<()> {
        self.agent_lock(session_id).await?;
        self.actor(session_id).await?.click(ref_id).await
    }

    pub async fn type_text(
        &self,
        session_id: &str,
        ref_id: &str,
        text: &str,
        submit: bool,
    ) -> BrowserResult<()> {
        self.agent_lock(session_id).await?;
        self.actor(session_id)
            .await?
            .type_text(ref_id, text, submit)
            .await
    }

    pub async fn press_key(&self, session_id: &str, key: &str) -> BrowserResult<()> {
        self.agent_lock(session_id).await?;
        self.actor(session_id).await?.press_key(key).await
    }

    pub async fn scroll(
        &self,
        session_id: &str,
        direction: &str,
        amount: i32,
    ) -> BrowserResult<()> {
        self.agent_lock(session_id).await?;
        self.actor(session_id)
            .await?
            .scroll(direction, amount)
            .await
    }

    pub async fn cdp(&self, session_id: &str, method: &str, params: Value) -> BrowserResult<Value> {
        self.agent_lock(session_id).await?;
        self.actor(session_id).await?.cdp(method, params).await
    }

    pub async fn set_lock(&self, session_id: &str, lock: LockHolder) -> BrowserResult<LockHolder> {
        self.actor(session_id).await?.set_lock(lock).await
    }

    pub async fn user_unlock(&self, session_id: &str) -> BrowserResult<LockHolder> {
        self.set_lock(session_id, LockHolder::User).await
    }

    async fn agent_lock(&self, session_id: &str) -> BrowserResult<()> {
        let lock = self.set_lock(session_id, LockHolder::Agent).await?;
        if lock == LockHolder::User {
            return Err(BrowserError::Locked("user".into()));
        }
        Ok(())
    }

    pub async fn subscribe_screencast(
        &self,
        session_id: &str,
    ) -> BrowserResult<broadcast::Receiver<ScreencastFrame>> {
        // When CEF Alloy embed published a CDP port, the live NSView is the
        // preview — skip JPEG screencast. Kill-switch ANYCODE_CEF_EMBED=0 clears
        // the port so this path uses screencast instead.
        if std::env::var("ANYCODE_CEF_CDP_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .is_some_and(|p| p > 0)
        {
            let (tx, rx) = broadcast::channel(1);
            let _ = tx;
            return Ok(rx);
        }
        let mut guard = self.inner.write().await;
        let session = guard
            .get_mut(session_id)
            .ok_or_else(|| BrowserError::SessionNotFound(session_id.to_string()))?;
        if session.screencast.is_none() {
            let (tx, _) = broadcast::channel(16);
            session.actor.subscribe_screencast(tx.clone()).await;
            session.screencast = Some(tx);
        }
        Ok(session.screencast.as_ref().unwrap().subscribe())
    }

    /// Resolve session id for agent tools from env or explicit input.
    pub fn resolve_agent_session_id(explicit: Option<&str>) -> Option<String> {
        if let Some(s) = explicit.filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
        std::env::var("ANYCODE_BROWSER_SESSION_ID")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("ANYCODE_DASHBOARD_SESSION_ID")
                    .ok()
                    .filter(|s| !s.is_empty())
            })
    }
}
