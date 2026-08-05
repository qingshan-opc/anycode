//! Per-session CDP actor — one Tokio task owns Browser + pages.

use crate::ax_tree::fetch_ax_tree_yaml;
use crate::chromium::resolve_chromium_executable;
use crate::design_inspect;
use crate::error::{BrowserError, BrowserResult};
use crate::policy::{cdp_method_allowed, validate_navigation_url};
use crate::snapshot::snapshot_script;
use crate::types::{
    BrowserDesignInspectState, BrowserHitTestResult, BrowserScreenshot, BrowserSnapshot,
    BrowserState, BrowserTabInfo, BrowserViewport, LockHolder, ScreencastFrame, ScreencastMetadata,
    ViewportSpec,
};
use crate::viewport::resolve_viewport_spec;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::{
    ClearDeviceMetricsOverrideParams, SetDeviceMetricsOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::page::{
    CaptureScreenshotFormat, EventScreencastFrame, ScreencastFrameAckParams, StartScreencastFormat,
    StartScreencastParams, StopScreencastParams,
};
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::Page;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub struct SessionActorHandle {
    inner: Arc<SessionActorInner>,
}

struct SessionActorInner {
    cmd_tx: mpsc::Sender<ActorCmd>,
    lock: Arc<Mutex<LockHolder>>,
}

enum ActorCmd {
    /// Liveness probe: true when the CDP connection is alive AND at least
    /// one page target exists. False means the chromiumoxide handler task is
    /// gone (websocket dropped / CEF browser restarted) or no page is left.
    Ping(oneshot::Sender<bool>),
    ListTabs(oneshot::Sender<BrowserResult<Vec<BrowserTabInfo>>>),
    NewTab(oneshot::Sender<BrowserResult<String>>),
    CloseTab {
        tab_id: String,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    SelectTab {
        tab_id: String,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    Navigate {
        url: String,
        respond: oneshot::Sender<BrowserResult<BrowserState>>,
    },
    Console {
        limit: usize,
        respond: oneshot::Sender<BrowserResult<Value>>,
    },
    State(oneshot::Sender<BrowserResult<BrowserState>>),
    Snapshot {
        root_ref: Option<String>,
        respond: oneshot::Sender<BrowserResult<BrowserSnapshot>>,
    },
    Screenshot(oneshot::Sender<BrowserResult<BrowserScreenshot>>),
    HitTest {
        x: f64,
        y: f64,
        respond: oneshot::Sender<BrowserResult<Option<BrowserHitTestResult>>>,
    },
    SetDesignMode {
        enabled: bool,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    PollDesignInspect(oneshot::Sender<BrowserResult<BrowserDesignInspectState>>),
    Click {
        ref_id: String,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    TypeText {
        ref_id: String,
        text: String,
        submit: bool,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    PressKey {
        key: String,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    Scroll {
        direction: String,
        amount: i32,
        respond: oneshot::Sender<BrowserResult<()>>,
    },
    Cdp {
        method: String,
        params: Value,
        respond: oneshot::Sender<BrowserResult<Value>>,
    },
    SetLock {
        lock: LockHolder,
        respond: oneshot::Sender<BrowserResult<LockHolder>>,
    },
    SetViewport {
        width: u32,
        height: u32,
        device_scale_factor: f64,
        respond: oneshot::Sender<BrowserResult<BrowserViewport>>,
    },
    SubscribeScreencast(broadcast::Sender<ScreencastFrame>),
    Shutdown,
}

struct TabMeta {
    page: Page,
    url: String,
    title: String,
}

/// When the desktop CEF host is running, Agent Browser* tools attach to the same
/// Chromium instance via remote debugging instead of launching headless Chrome.
fn cef_cdp_port() -> Option<u16> {
    let raw = std::env::var("ANYCODE_CEF_CDP_PORT").ok()?;
    let port: u16 = raw.parse().ok()?;
    (port > 0).then_some(port)
}

impl SessionActorHandle {
    pub async fn spawn(requested_viewport: Option<ViewportSpec>) -> BrowserResult<Self> {
        if let Some(port) = cef_cdp_port() {
            // Desktop CEF is showing the live preview — never fall back to an
            // invisible headless Chrome or Agent ops diverge from the panel.
            return Self::spawn_attach_cdp(port, requested_viewport)
                .await
                .map_err(|e| {
                    BrowserError::Other(anyhow::anyhow!(
                        "CEF CDP attach failed on port {port}: {e}. Open the Browser panel and retry."
                    ))
                });
        }
        Self::spawn_headless(requested_viewport).await
    }

    async fn spawn_attach_cdp(
        port: u16,
        requested_viewport: Option<ViewportSpec>,
    ) -> BrowserResult<Self> {
        let vp = resolve_viewport_spec(requested_viewport);
        let viewport_w = vp.width;
        let viewport_h = vp.height;
        let url = format!("http://127.0.0.1:{port}");
        let (browser, mut handler) = Browser::connect(url)
            .await
            .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?;
        tokio::spawn(async move {
            while handler.next().await.is_some() {}
            tracing::warn!(
                target: "anycode_browser",
                "CEF CDP websocket closed — attached actor is now a zombie until respawn"
            );
        });

        let first_page = {
            let mut existing = Vec::new();
            // The desktop panel creates the first CEF page when the Browser
            // rail opens (cef_browser_show). That happens in parallel with the
            // agent's first Browser call, so give the frontend a few seconds to
            // publish a page before giving up — never fall back to headless.
            for _ in 0..120 {
                existing = browser
                    .pages()
                    .await
                    .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?;
                if !existing.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            let page = existing.into_iter().next().ok_or_else(|| {
                BrowserError::Other(anyhow::anyhow!(
                    "CEF CDP connected but no pages yet (port {port}). \
                     Open the Browser panel (right rail) and retry the Browser tool."
                ))
            })?;
            // Alloy already sizes to the NSView. Forcing screen-sized device
            // metrics here makes the page layout huge vs the panel (quarter /
            // scroll / select breakage). Clear any override and let CEF win.
            let _ = page.execute(ClearDeviceMetricsOverrideParams {}).await;
            page
        };

        let first_id = Uuid::new_v4().to_string();
        let mut tabs = HashMap::new();
        tabs.insert(
            first_id.clone(),
            TabMeta {
                page: first_page,
                url: "about:blank".into(),
                title: String::new(),
            },
        );

        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let lock = Arc::new(Mutex::new(LockHolder::Idle));
        tokio::spawn(session_loop(
            cmd_rx,
            browser,
            tabs,
            first_id,
            lock.clone(),
            viewport_w,
            viewport_h,
        ));

        Ok(Self {
            inner: Arc::new(SessionActorInner { cmd_tx, lock }),
        })
    }

    async fn spawn_headless(requested_viewport: Option<ViewportSpec>) -> BrowserResult<Self> {
        let chrome = resolve_chromium_executable()
            .ok_or_else(|| BrowserError::Unavailable(crate::chromium::chromium_doctor_message()))?;

        let user_data = std::env::temp_dir().join(format!("anycode-browser-{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&user_data);

        let vp = resolve_viewport_spec(requested_viewport);
        let viewport_w = vp.width;
        let viewport_h = vp.height;
        let device_scale = vp.device_scale_factor;

        // Match window_size and Emulation.SetDeviceMetricsOverride — chromiumoxide
        // defaults viewport to 800×600, which letterboxes / mismatches screencast.
        // device_scale_factor > 1 captures Retina-sharp screencast bitmaps.
        let config = BrowserConfig::builder()
            .chrome_executable(chrome)
            .user_data_dir(&user_data)
            .arg("--headless=new")
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .window_size(viewport_w, viewport_h)
            .viewport(Viewport {
                width: viewport_w,
                height: viewport_h,
                device_scale_factor: Some(device_scale),
                emulating_mobile: false,
                is_landscape: viewport_w >= viewport_h,
                has_touch: false,
            })
            .build()
            .map_err(|e| BrowserError::Other(anyhow::anyhow!("{e}")))?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?;

        tokio::spawn(async move {
            while handler.next().await.is_some() {}
            tracing::warn!(
                target: "anycode_browser",
                "headless Chrome CDP websocket closed — actor is now a zombie until respawn"
            );
        });

        // Chromium already opens a default blank target on launch. Calling
        // `new_page` again creates a second tab — reuse existing pages first.
        let first_page = {
            let mut existing = Vec::new();
            for _ in 0..20 {
                existing = browser
                    .pages()
                    .await
                    .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?;
                if !existing.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            let mut iter = existing.into_iter();
            let page = if let Some(page) = iter.next() {
                page
            } else {
                browser
                    .new_page("about:blank")
                    .await
                    .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?
            };
            // Drop any extra blank targets so the local window shows one tab.
            for extra in iter {
                let _ = extra.close().await;
            }
            page
        };

        let first_id = Uuid::new_v4().to_string();
        let mut tabs = HashMap::new();
        tabs.insert(
            first_id.clone(),
            TabMeta {
                page: first_page,
                url: "about:blank".into(),
                title: String::new(),
            },
        );

        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let lock = Arc::new(Mutex::new(LockHolder::Idle));

        tokio::spawn(session_loop(
            cmd_rx,
            browser,
            tabs,
            first_id,
            lock.clone(),
            viewport_w,
            viewport_h,
        ));

        Ok(Self {
            inner: Arc::new(SessionActorInner { cmd_tx, lock }),
        })
    }

    async fn send<R>(
        &self,
        build: impl FnOnce(oneshot::Sender<BrowserResult<R>>) -> ActorCmd,
    ) -> BrowserResult<R> {
        let (tx, rx) = oneshot::channel();
        self.inner
            .cmd_tx
            .send(build(tx))
            .await
            .map_err(|_| BrowserError::Unavailable("browser session closed".into()))?;
        rx.await
            .map_err(|_| BrowserError::Unavailable("browser session dropped".into()))?
    }

    pub async fn list_tabs(&self) -> BrowserResult<Vec<BrowserTabInfo>> {
        self.send(ActorCmd::ListTabs).await
    }

    pub async fn new_tab(&self) -> BrowserResult<String> {
        self.send(ActorCmd::NewTab).await
    }

    pub async fn close_tab(&self, tab_id: &str) -> BrowserResult<()> {
        self.send(|r| ActorCmd::CloseTab {
            tab_id: tab_id.into(),
            respond: r,
        })
        .await
    }

    pub async fn select_tab(&self, tab_id: &str) -> BrowserResult<()> {
        self.send(|r| ActorCmd::SelectTab {
            tab_id: tab_id.into(),
            respond: r,
        })
        .await
    }

    pub async fn navigate(&self, url: &str) -> BrowserResult<BrowserState> {
        self.send(|r| ActorCmd::Navigate {
            url: url.into(),
            respond: r,
        })
        .await
    }

    pub async fn console_and_network(&self, limit: usize) -> BrowserResult<Value> {
        self.send(|r| ActorCmd::Console { limit, respond: r }).await
    }

    pub async fn state(&self) -> BrowserResult<BrowserState> {
        self.send(ActorCmd::State).await
    }

    pub async fn snapshot(&self, root_ref: Option<&str>) -> BrowserResult<BrowserSnapshot> {
        self.send(|r| ActorCmd::Snapshot {
            root_ref: root_ref.map(str::to_string),
            respond: r,
        })
        .await
    }

    pub async fn screenshot(&self) -> BrowserResult<BrowserScreenshot> {
        self.send(ActorCmd::Screenshot).await
    }

    pub async fn hit_test(&self, x: f64, y: f64) -> BrowserResult<Option<BrowserHitTestResult>> {
        self.send(|respond| ActorCmd::HitTest { x, y, respond })
            .await
    }

    pub async fn set_design_mode(&self, enabled: bool) -> BrowserResult<()> {
        self.send(|respond| ActorCmd::SetDesignMode { enabled, respond })
            .await
    }

    pub async fn poll_design_inspect(&self) -> BrowserResult<BrowserDesignInspectState> {
        self.send(ActorCmd::PollDesignInspect).await
    }

    pub async fn click(&self, ref_id: &str) -> BrowserResult<()> {
        self.send(|r| ActorCmd::Click {
            ref_id: ref_id.into(),
            respond: r,
        })
        .await
    }

    pub async fn type_text(&self, ref_id: &str, text: &str, submit: bool) -> BrowserResult<()> {
        self.send(|r| ActorCmd::TypeText {
            ref_id: ref_id.into(),
            text: text.into(),
            submit,
            respond: r,
        })
        .await
    }

    pub async fn press_key(&self, key: &str) -> BrowserResult<()> {
        self.send(|r| ActorCmd::PressKey {
            key: key.into(),
            respond: r,
        })
        .await
    }

    pub async fn scroll(&self, direction: &str, amount: i32) -> BrowserResult<()> {
        self.send(|r| ActorCmd::Scroll {
            direction: direction.into(),
            amount,
            respond: r,
        })
        .await
    }

    pub async fn cdp(&self, method: &str, params: Value) -> BrowserResult<Value> {
        self.send(|r| ActorCmd::Cdp {
            method: method.into(),
            params,
            respond: r,
        })
        .await
    }

    pub async fn set_lock(&self, lock: LockHolder) -> BrowserResult<LockHolder> {
        self.send(|r| ActorCmd::SetLock { lock, respond: r }).await
    }

    pub async fn set_viewport(
        &self,
        width: u32,
        height: u32,
        device_scale_factor: f64,
    ) -> BrowserResult<BrowserViewport> {
        self.send(|r| ActorCmd::SetViewport {
            width,
            height,
            device_scale_factor,
            respond: r,
        })
        .await
    }

    pub fn lock_holder(&self) -> Arc<Mutex<LockHolder>> {
        self.inner.lock.clone()
    }

    pub async fn subscribe_screencast(&self, tx: broadcast::Sender<ScreencastFrame>) {
        let _ = self
            .inner
            .cmd_tx
            .send(ActorCmd::SubscribeScreencast(tx))
            .await;
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.cmd_tx.send(ActorCmd::Shutdown).await;
    }

    /// Cheap liveness probe through the CDP connection. Returns false when
    /// the actor task died, the chromiumoxide handler task ended (websocket
    /// dropped, CEF browser process restarted), or no page targets remain —
    /// in all those cases every subsequent command fails instantly with
    /// "send failed because receiver is gone" and the caller should respawn.
    pub async fn ping(&self) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.inner.cmd_tx.send(ActorCmd::Ping(tx)).await.is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }
}

async fn session_loop(
    mut cmd_rx: mpsc::Receiver<ActorCmd>,
    browser: Browser,
    mut tabs: HashMap<String, TabMeta>,
    mut active_tab: String,
    lock: Arc<Mutex<LockHolder>>,
    mut viewport_w: u32,
    mut viewport_h: u32,
) {
    let mut screencast_tx: Option<broadcast::Sender<ScreencastFrame>> = None;
    let mut screencast_task: Option<tokio::task::JoinHandle<()>> = None;
    let mut design_mode = false;

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            ActorCmd::Ping(respond) => {
                let ok = match browser.version().await {
                    Ok(_) => browser
                        .pages()
                        .await
                        .map(|p| !p.is_empty())
                        .unwrap_or(false),
                    Err(_) => false,
                };
                let _ = respond.send(ok);
            }
            ActorCmd::ListTabs(respond) => {
                let list = tabs
                    .iter()
                    .map(|(id, t)| BrowserTabInfo {
                        tab_id: id.clone(),
                        url: t.url.clone(),
                        title: t.title.clone(),
                        active: id == &active_tab,
                    })
                    .collect();
                let _ = respond.send(Ok(list));
            }
            ActorCmd::NewTab(respond) => {
                let result = match browser.new_page("about:blank").await {
                    Ok(page) => {
                        let id = Uuid::new_v4().to_string();
                        tabs.insert(
                            id.clone(),
                            TabMeta {
                                page,
                                url: "about:blank".into(),
                                title: String::new(),
                            },
                        );
                        active_tab = id.clone();
                        Ok(id)
                    }
                    Err(e) => Err(BrowserError::Other(e.into())),
                };
                let _ = respond.send(result);
                restart_screencast_if_needed(
                    &screencast_tx,
                    &mut screencast_task,
                    &tabs,
                    &active_tab,
                );
            }
            ActorCmd::CloseTab { tab_id, respond } => {
                let result = if tabs.len() <= 1 {
                    Err(BrowserError::Other(anyhow::anyhow!(
                        "cannot close last tab"
                    )))
                } else if let Some(entry) = tabs.remove(&tab_id) {
                    let _ = entry.page.close().await;
                    if active_tab == tab_id {
                        active_tab = tabs.keys().next().cloned().unwrap_or_default();
                    }
                    Ok(())
                } else {
                    Err(BrowserError::TabNotFound(tab_id))
                };
                let _ = respond.send(result);
            }
            ActorCmd::SelectTab { tab_id, respond } => {
                let result = if tabs.contains_key(&tab_id) {
                    active_tab = tab_id;
                    Ok(())
                } else {
                    Err(BrowserError::TabNotFound(tab_id))
                };
                if result.is_ok() && design_mode {
                    if let Ok(page) = active_page(&tabs, &active_tab).await {
                        let _ = design_inspect::enable_on_page(page).await;
                    }
                }
                let _ = respond.send(result);
                restart_screencast_if_needed(
                    &screencast_tx,
                    &mut screencast_task,
                    &tabs,
                    &active_tab,
                );
            }
            ActorCmd::Navigate { url, respond } => {
                let result = navigate_tab(&mut tabs, &active_tab, &url, &lock).await;
                if design_mode {
                    if let Ok(page) = active_page(&tabs, &active_tab).await {
                        let _ = design_inspect::enable_on_page(page).await;
                    }
                }
                let _ = respond.send(result);
                restart_screencast_if_needed(
                    &screencast_tx,
                    &mut screencast_task,
                    &tabs,
                    &active_tab,
                );
            }
            ActorCmd::Console { limit, respond } => {
                let result = console_and_network_tab(&tabs, &active_tab, limit).await;
                let _ = respond.send(result);
            }
            ActorCmd::State(respond) => {
                let result = state_tab(&tabs, &active_tab, &lock).await;
                let _ = respond.send(result);
            }
            ActorCmd::Snapshot { root_ref, respond } => {
                let result = snapshot_tab(&tabs, &active_tab, root_ref.as_deref()).await;
                let _ = respond.send(result);
            }
            ActorCmd::Screenshot(respond) => {
                let result = screenshot_tab(&tabs, &active_tab, viewport_w, viewport_h).await;
                let _ = respond.send(result);
            }
            ActorCmd::HitTest { x, y, respond } => {
                let result = hit_test_tab(&tabs, &active_tab, x, y).await;
                let _ = respond.send(result);
            }
            ActorCmd::SetDesignMode { enabled, respond } => {
                let result = async {
                    let page = active_page(&tabs, &active_tab).await?;
                    if enabled {
                        design_inspect::enable_on_page(&page).await?;
                    } else {
                        design_inspect::disable_on_page(&page).await?;
                    }
                    Ok(())
                }
                .await;
                if result.is_ok() {
                    design_mode = enabled;
                }
                let _ = respond.send(result);
            }
            ActorCmd::PollDesignInspect(respond) => {
                let result = async {
                    if !design_mode {
                        return Ok(BrowserDesignInspectState::default());
                    }
                    let page = active_page(&tabs, &active_tab).await?;
                    let mut state = design_inspect::poll_on_page(&page).await?;
                    // Page navigated away / script lost — re-inject.
                    if !state.enabled {
                        design_inspect::enable_on_page(&page).await?;
                        state = design_inspect::poll_on_page(&page).await?;
                    }
                    Ok(state)
                }
                .await;
                let _ = respond.send(result);
            }
            ActorCmd::Click { ref_id, respond } => {
                let result = click_ref(&tabs, &active_tab, &ref_id).await;
                let _ = respond.send(result);
            }
            ActorCmd::TypeText {
                ref_id,
                text,
                submit,
                respond,
            } => {
                let result = type_ref(&tabs, &active_tab, &ref_id, &text, submit).await;
                let _ = respond.send(result);
            }
            ActorCmd::PressKey { key, respond } => {
                let result = press_key(&tabs, &active_tab, &key).await;
                let _ = respond.send(result);
            }
            ActorCmd::Scroll {
                direction,
                amount,
                respond,
            } => {
                let result = scroll_tab(&tabs, &active_tab, &direction, amount).await;
                let _ = respond.send(result);
            }
            ActorCmd::Cdp {
                method,
                params,
                respond,
            } => {
                let result =
                    cdp_tab(&tabs, &active_tab, &method, params, viewport_w, viewport_h).await;
                let _ = respond.send(result);
            }
            ActorCmd::SetLock {
                lock: new_lock,
                respond,
            } => {
                let mut g = lock.lock().await;
                *g = new_lock;
                let _ = respond.send(Ok(new_lock));
            }
            ActorCmd::SetViewport {
                width,
                height,
                device_scale_factor,
                respond,
            } => {
                let vp = resolve_viewport_spec(Some(ViewportSpec::new(
                    width,
                    height,
                    device_scale_factor,
                )));
                let result = apply_viewport(
                    &tabs,
                    &active_tab,
                    vp.width,
                    vp.height,
                    vp.device_scale_factor,
                )
                .await;
                if result.is_ok() {
                    viewport_w = vp.width;
                    viewport_h = vp.height;
                    restart_screencast_if_needed(
                        &screencast_tx,
                        &mut screencast_task,
                        &tabs,
                        &active_tab,
                    );
                }
                let _ = respond.send(result.map(|_| BrowserViewport {
                    width: vp.width,
                    height: vp.height,
                }));
            }
            ActorCmd::SubscribeScreencast(tx) => {
                screencast_tx = Some(tx.clone());
                if let Some(page) = tabs.get(&active_tab).map(|t| t.page.clone()) {
                    if let Some(task) = screencast_task.take() {
                        task.abort();
                    }
                    screencast_task = Some(spawn_screencast_task(page, tx));
                }
            }
            ActorCmd::Shutdown => {
                if let Some(task) = screencast_task.take() {
                    task.abort();
                }
                break;
            }
        }
    }
    if let Some(task) = screencast_task.take() {
        task.abort();
    }
}

fn spawn_screencast_task(
    page: Page,
    tx: broadcast::Sender<ScreencastFrame>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let params = StartScreencastParams::builder()
            .format(StartScreencastFormat::Jpeg)
            // High quality — low JPEG quality looks soft vs native Chrome on Retina.
            .quality(92_i64)
            .every_nth_frame(1_i64)
            .build();
        if page.execute(params).await.is_err() {
            return;
        }
        let mut events = match page.event_listener::<EventScreencastFrame>().await {
            Ok(stream) => stream,
            Err(_) => return,
        };
        while let Some(frame) = events.next().await {
            if tx.receiver_count() == 0 {
                continue;
            }
            let meta = &frame.metadata;
            let image_base64 =
                <chromiumoxide::Binary as AsRef<str>>::as_ref(&frame.data).to_string();
            let _ = tx.send(ScreencastFrame {
                image_base64,
                metadata: ScreencastMetadata {
                    offset_top: meta.offset_top as i64,
                    page_scale_factor: meta.page_scale_factor,
                    device_width: meta.device_width as i64,
                    device_height: meta.device_height as i64,
                    scroll_offset_x: meta.scroll_offset_x as i64,
                    scroll_offset_y: meta.scroll_offset_y as i64,
                    timestamp: None,
                },
            });
            if let Ok(ack) = ScreencastFrameAckParams::builder()
                .session_id(frame.session_id)
                .build()
            {
                let _ = page.execute(ack).await;
            }
        }
        let _ = page.execute(StopScreencastParams::default()).await;
    })
}

fn restart_screencast_if_needed(
    screencast_tx: &Option<broadcast::Sender<ScreencastFrame>>,
    screencast_task: &mut Option<tokio::task::JoinHandle<()>>,
    tabs: &HashMap<String, TabMeta>,
    active_tab: &str,
) {
    let Some(tx) = screencast_tx.clone() else {
        return;
    };
    let Some(page) = tabs.get(active_tab).map(|t| t.page.clone()) else {
        return;
    };
    if let Some(task) = screencast_task.take() {
        task.abort();
    }
    *screencast_task = Some(spawn_screencast_task(page, tx));
}

async fn page_url_string(page: &Page, fallback: &str) -> String {
    page.url()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| fallback.to_string())
}

async fn active_page<'a>(
    tabs: &'a HashMap<String, TabMeta>,
    active: &str,
) -> BrowserResult<&'a Page> {
    tabs.get(active)
        .map(|t| &t.page)
        .ok_or_else(|| BrowserError::TabNotFound(active.to_string()))
}

async fn navigate_tab(
    tabs: &mut HashMap<String, TabMeta>,
    active: &str,
    url: &str,
    lock: &Arc<Mutex<LockHolder>>,
) -> BrowserResult<BrowserState> {
    let _url = validate_navigation_url(url)?;
    let entry = tabs
        .get_mut(active)
        .ok_or_else(|| BrowserError::TabNotFound(active.to_string()))?;
    entry
        .page
        .goto(url)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    let _ = ensure_page_instrumentation(&entry.page).await;
    entry.url = page_url_string(&entry.page, url).await;
    entry.title = entry
        .page
        .get_title()
        .await
        .ok()
        .flatten()
        .unwrap_or_default();
    state_tab(tabs, active, lock).await
}

async fn state_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    lock: &Arc<Mutex<LockHolder>>,
) -> BrowserResult<BrowserState> {
    let entry = tabs
        .get(active)
        .ok_or_else(|| BrowserError::TabNotFound(active.to_string()))?;
    let url = page_url_string(&entry.page, &entry.url).await;
    let title = entry
        .page
        .get_title()
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| entry.title.clone());
    let lock_val = *lock.lock().await;
    Ok(BrowserState {
        url,
        title,
        lock: lock_val,
    })
}

async fn snapshot_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    root_ref: Option<&str>,
) -> BrowserResult<BrowserSnapshot> {
    let page = active_page(tabs, active).await?;
    #[derive(Deserialize)]
    struct Snap {
        title: String,
        url: String,
        yaml: String,
    }
    let snap: Snap = page
        .evaluate(snapshot_script(root_ref).as_str())
        .await
        .map_err(|e| BrowserError::Other(e.into()))?
        .into_value()
        .map_err(|e| BrowserError::Other(e.into()))?;

    let mut yaml = snap.yaml;
    if root_ref.is_none() {
        if let Some(ax) = fetch_ax_tree_yaml(page).await {
            if !ax.is_empty() {
                yaml = format!(
                    "{yaml}\n\n# Accessibility tree (CDP supplement; use ref= for clicks)\n{ax}"
                );
            }
        }
    }

    Ok(BrowserSnapshot {
        url: snap.url,
        title: snap.title,
        yaml,
    })
}

async fn apply_viewport(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    width: u32,
    height: u32,
    device_scale_factor: f64,
) -> BrowserResult<()> {
    let page = active_page(tabs, active).await?;
    let params = SetDeviceMetricsOverrideParams::builder()
        .width(width)
        .height(height)
        .device_scale_factor(device_scale_factor)
        .mobile(false)
        .build()
        .map_err(|e| BrowserError::Other(anyhow::anyhow!("{e}")))?;
    page.execute(params)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

async fn screenshot_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    viewport_w: u32,
    viewport_h: u32,
) -> BrowserResult<BrowserScreenshot> {
    let page = active_page(tabs, active).await?;
    let bytes = page
        .screenshot(
            chromiumoxide::page::ScreenshotParams::builder()
                .format(CaptureScreenshotFormat::Png)
                .full_page(false)
                .build(),
        )
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(BrowserScreenshot {
        image_base64: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),
        viewport: BrowserViewport {
            width: viewport_w,
            height: viewport_h,
        },
    })
}

async fn hit_test_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    x: f64,
    y: f64,
) -> BrowserResult<Option<BrowserHitTestResult>> {
    let page = active_page(tabs, active).await?;
    #[derive(Deserialize)]
    struct Hit {
        tag: String,
        id: Option<String>,
        classes: Vec<String>,
        text: Option<String>,
        css_selector: String,
        xpath: Option<String>,
    }
    // Use r## so a JS `"#` sequence cannot terminate the Rust raw string.
    let script = format!(
        r##"((x, y) => {{
          const el = document.elementFromPoint(x, y);
          if (!el || !(el instanceof Element)) return null;
          const prev = el.getAttribute('data-anycode-pick-outline');
          el.style.outline = '2px solid #7868ff';
          el.style.outlineOffset = '1px';
          el.setAttribute('data-anycode-pick-outline', '1');
          setTimeout(() => {{
            if (el.getAttribute('data-anycode-pick-outline') === '1') {{
              el.style.outline = prev || '';
              el.style.outlineOffset = '';
              el.removeAttribute('data-anycode-pick-outline');
            }}
          }}, 1200);
          const tag = el.tagName.toLowerCase();
          const id = el.id || null;
          const classes = el.classList ? Array.from(el.classList).slice(0, 8) : [];
          let text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
          if (text.length > 80) text = text.slice(0, 79) + '…';
          function escId(value) {{
            if (window.CSS && CSS.escape) return CSS.escape(value);
            return String(value).replace(/([^\w-])/g, '\\$1');
          }}
          function cssPath(node) {{
            if (!(node instanceof Element)) return '';
            if (node.id) return '#' + escId(node.id);
            const parts = [];
            let cur = node;
            while (cur && cur.nodeType === 1 && parts.length < 6) {{
              let part = cur.tagName.toLowerCase();
              if (cur.id) {{
                parts.unshift('#' + escId(cur.id));
                break;
              }}
              const parent = cur.parentElement;
              if (parent) {{
                const siblings = Array.from(parent.children).filter(c => c.tagName === cur.tagName);
                if (siblings.length > 1) {{
                  const idx = siblings.indexOf(cur) + 1;
                  part += ':nth-of-type(' + idx + ')';
                }}
              }}
              parts.unshift(part);
              cur = parent;
              if (cur && cur.tagName && cur.tagName.toLowerCase() === 'body') {{
                parts.unshift('body');
                break;
              }}
            }}
            return parts.join(' > ');
          }}
          function xpathOf(node) {{
            if (!(node instanceof Element)) return null;
            if (node.id) return '//*[@id="' + String(node.id).replace(/"/g, '\\"') + '"]';
            const parts = [];
            let cur = node;
            while (cur && cur.nodeType === 1 && parts.length < 8) {{
              let ix = 1;
              let sib = cur.previousElementSibling;
              while (sib) {{
                if (sib.tagName === cur.tagName) ix++;
                sib = sib.previousElementSibling;
              }}
              parts.unshift(cur.tagName.toLowerCase() + '[' + ix + ']');
              cur = cur.parentElement;
              if (cur && cur.tagName && cur.tagName.toLowerCase() === 'html') break;
            }}
            return '/' + parts.join('/');
          }}
          return {{
            tag,
            id,
            classes,
            text: text || null,
            css_selector: cssPath(el),
            xpath: xpathOf(el),
          }};
        }})({x}, {y})"##
    );
    let hit: Option<Hit> = page
        .evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::Other(e.into()))?
        .into_value()
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(hit.map(|h| BrowserHitTestResult {
        x,
        y,
        tag: h.tag,
        id: h.id.filter(|s| !s.is_empty()),
        classes: h.classes,
        text: h.text.filter(|s| !s.is_empty()),
        css_selector: h.css_selector,
        xpath: h.xpath.filter(|s| !s.is_empty()),
    }))
}

async fn click_ref(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    ref_id: &str,
) -> BrowserResult<()> {
    let page = active_page(tabs, active).await?;
    let script = format!(
        r#"((ref) => {{
          const el = window.__anycodeBrowserRefs && window.__anycodeBrowserRefs[ref];
          if (!el) throw new Error('ref not found: ' + ref);
          try {{
            el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'instant' }});
          }} catch {{
            try {{ el.scrollIntoView(true); }} catch {{}}
          }}
          const prev = el.getAttribute('data-anycode-pick-outline');
          el.style.outline = '2px solid #7868ff';
          el.style.outlineOffset = '1px';
          el.setAttribute('data-anycode-pick-outline', '1');
          setTimeout(() => {{
            if (el.getAttribute('data-anycode-pick-outline') === '1') {{
              el.style.outline = prev || '';
              el.style.outlineOffset = '';
              el.removeAttribute('data-anycode-pick-outline');
            }}
          }}, 1200);
          el.click();
          return true;
        }})("{ref_id}")"#
    );
    page.evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::RefNotFound(format!("{ref_id}: {e}")))?;
    Ok(())
}

async fn type_ref(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    ref_id: &str,
    text: &str,
    submit: bool,
) -> BrowserResult<()> {
    let page = active_page(tabs, active).await?;
    let escaped = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        r#"((ref, text, submit) => {{
          const el = window.__anycodeBrowserRefs && window.__anycodeBrowserRefs[ref];
          if (!el) throw new Error('ref not found: ' + ref);
          try {{
            el.scrollIntoView({{ block: 'center', inline: 'nearest', behavior: 'instant' }});
          }} catch {{
            try {{ el.scrollIntoView(true); }} catch {{}}
          }}
          const prev = el.getAttribute('data-anycode-pick-outline');
          el.style.outline = '2px solid #7868ff';
          el.style.outlineOffset = '1px';
          el.setAttribute('data-anycode-pick-outline', '1');
          setTimeout(() => {{
            if (el.getAttribute('data-anycode-pick-outline') === '1') {{
              el.style.outline = prev || '';
              el.style.outlineOffset = '';
              el.removeAttribute('data-anycode-pick-outline');
            }}
          }}, 1200);
          el.focus();
          if ('value' in el) el.value = text;
          else el.textContent = text;
          el.dispatchEvent(new Event('input', {{ bubbles: true }}));
          if (submit) el.dispatchEvent(new KeyboardEvent('keydown', {{ key: 'Enter', bubbles: true }}));
          return true;
        }})("{ref_id}", {escaped}, {submit})"#,
        submit = submit
    );
    page.evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::RefNotFound(format!("{ref_id}: {e}")))?;
    Ok(())
}

async fn press_key(tabs: &HashMap<String, TabMeta>, active: &str, key: &str) -> BrowserResult<()> {
    let page = active_page(tabs, active).await?;
    let escaped = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".into());
    let script = format!(
        r#"((key) => {{
          document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', {{ key, bubbles: true }}));
          return true;
        }})({escaped})"#
    );
    page.evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

async fn scroll_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    direction: &str,
    amount: i32,
) -> BrowserResult<()> {
    let page = active_page(tabs, active).await?;
    let (dx, dy) = match direction {
        "up" => (0, -amount),
        "down" => (0, amount),
        "left" => (-amount, 0),
        "right" => (amount, 0),
        _ => (0, amount),
    };
    let script = format!("window.scrollBy({dx}, {dy}); true");
    page.evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

async fn ensure_page_instrumentation(page: &Page) -> BrowserResult<()> {
    // Install once per document: wrap console.* and keep a ring buffer.
    let script = r#"(function(){
      if (window.__anycodeBrowserInst) return true;
      window.__anycodeBrowserInst = true;
      window.__anycodeConsole = [];
      const push = (level, args) => {
        try {
          const text = Array.from(args).map(a => {
            try { return typeof a === 'string' ? a : JSON.stringify(a); }
            catch { return String(a); }
          }).join(' ');
          window.__anycodeConsole.push({ level, text: String(text).slice(0, 500), at: Date.now() });
          if (window.__anycodeConsole.length > 200) window.__anycodeConsole.shift();
        } catch {}
      };
      for (const level of ['log','info','warn','error','debug']) {
        const orig = console[level] ? console[level].bind(console) : null;
        console[level] = function(...args) {
          push(level, args);
          if (orig) return orig(...args);
        };
      }
      window.addEventListener('error', (e) => {
        push('error', [e.message || 'window.error']);
      });
      window.addEventListener('unhandledrejection', (e) => {
        push('error', ['unhandledrejection', String(e.reason)]);
      });
      return true;
    })()"#;
    page.evaluate(script)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

async fn console_and_network_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    limit: usize,
) -> BrowserResult<Value> {
    let page = active_page(tabs, active).await?;
    let _ = ensure_page_instrumentation(page).await;
    let lim = limit.clamp(1, 200);
    let script = format!(
        r#"(function(limit){{
      const logs = (window.__anycodeConsole || []).slice(-limit);
      const resources = (performance.getEntriesByType('resource') || [])
        .slice(-limit)
        .map(e => ({{
          name: String(e.name || '').slice(0, 300),
          type: e.initiatorType || '',
          duration_ms: Math.round(e.duration || 0),
          transfer_size: e.transferSize || 0,
        }}));
      return {{ console: logs, network: resources, url: location.href }};
    }})({lim})"#
    );
    let val = page
        .evaluate(script.as_str())
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(val.value().cloned().unwrap_or(json!({
        "console": [],
        "network": [],
    })))
}

async fn cdp_tab(
    tabs: &HashMap<String, TabMeta>,
    active: &str,
    method: &str,
    params: Value,
    viewport_w: u32,
    viewport_h: u32,
) -> BrowserResult<Value> {
    if !cdp_method_allowed(method) {
        return Err(BrowserError::CdpDenied(method.to_string()));
    }
    let page = active_page(tabs, active).await?;
    if method == "Runtime.evaluate" {
        let expr = params
            .get("expression")
            .and_then(|v| v.as_str())
            .unwrap_or("null");
        let val = page
            .evaluate(expr)
            .await
            .map_err(|e| BrowserError::Other(anyhow::Error::from(e)))?;
        return Ok(serde_json::json!({ "result": val.value() }));
    }
    if method == "Page.captureScreenshot" {
        let shot = screenshot_tab(tabs, active, viewport_w, viewport_h).await?;
        return Ok(serde_json::json!({ "data": shot.image_base64 }));
    }
    Err(BrowserError::CdpDenied(format!(
        "method {method} not wired yet"
    )))
}
