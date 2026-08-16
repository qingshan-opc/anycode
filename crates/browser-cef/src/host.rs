//! macOS CEF host: Alloy-style browser as a child NSView over the Workbench panel.

use crate::{EmbedRect, TabInfo};
use cef::*;
use objc2::runtime::AnyObject;
use objc2::{msg_send, ClassType, MainThreadMarker};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_DEBUG_PORT: u16 = 9333;
/// How many ports to probe starting at DEFAULT_DEBUG_PORT when it is taken.
const DEBUG_PORT_PROBE_RANGE: u16 = 20;

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static DEBUG_PORT: AtomicU16 = AtomicU16::new(0);
/// Browsers removed in `OnBeforeClose` — drop after the callback returns
/// (next message-pump turn) so we never destroy CefRefPtr mid-lifecycle.
static PENDING_BROWSER_DROPS: Mutex<Vec<Browser>> = Mutex::new(Vec::new());
static PENDING_POPUP_URLS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FORCE_CLOSE_AFTER: Mutex<Vec<(i32, Instant)>> = Mutex::new(Vec::new());
static LAYOUT_PENDING: AtomicBool = AtomicBool::new(false);

#[link(name = "anycode_cef_mac_app", kind = "static")]
extern "C" {
    fn anycode_cef_force_link_app_protocol();
}

struct BrowserTab {
    id: i32,
    browser: Browser,
    url: String,
    title: String,
    /// Hidden placeholder kept alive after the user closes the last visible tab.
    placeholder: bool,
}

struct HostState {
    tabs: Vec<BrowserTab>,
    active_id: Option<i32>,
    /// Raw NSView* stored as usize so HostState can be Send.
    container: usize,
}

impl HostState {
    fn new() -> Self {
        Self {
            tabs: Vec::new(),
            active_id: None,
            container: 0,
        }
    }

    fn active_tab(&self) -> Option<&BrowserTab> {
        let id = self.active_id?;
        self.tabs.iter().find(|t| t.id == id)
    }
}

fn state() -> &'static Mutex<HostState> {
    static STATE: OnceLock<Mutex<HostState>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(HostState::new()))
}

/// Set by the desktop host so CEF can ask AppKit to run `DoMessageLoopWork`.
static SCHEDULE_PUMP: OnceLock<Mutex<Option<Box<dyn Fn(i64) + Send>>>> = OnceLock::new();

pub fn set_schedule_pump_callback(cb: Box<dyn Fn(i64) + Send>) {
    let cell = SCHEDULE_PUMP.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(cb);
    }
}

fn schedule_pump_work(delay_ms: i64) {
    if let Some(cell) = SCHEDULE_PUMP.get() {
        if let Ok(g) = cell.lock() {
            if let Some(cb) = g.as_ref() {
                cb(delay_ms);
            }
        }
    }
}

wrap_browser_process_handler! {
    struct EmbedBrowserProcessHandler {}

    impl BrowserProcessHandler {
        fn on_schedule_message_pump_work(&self, delay_ms: i64) {
            schedule_pump_work(delay_ms);
        }
    }
}

wrap_app! {
    struct EmbedApp {}

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(EmbedBrowserProcessHandler::new())
        }
    }
}

#[derive(Clone, Default)]
struct ClientInner {}

wrap_client! {
    struct EmbedClient {
        inner: RefCell<ClientInner>,
    }

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(EmbedLifeSpan::new())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(EmbedDisplay::new())
        }
    }
}

fn set_browser_view_hidden(browser: &Browser, hidden: bool) {
    let Some(host) = browser.host() else {
        return;
    };
    let handle = host.window_handle();
    if handle.is_null() {
        return;
    }
    let view = unsafe { &*(handle as *const NSView) };
    view.setHidden(hidden);
}

fn integral_bounds(bounds: NSRect) -> NSRect {
    NSRect {
        origin: NSPoint {
            x: bounds.origin.x.floor(),
            y: bounds.origin.y.floor(),
        },
        size: NSSize {
            width: bounds.size.width.floor().max(1.0),
            height: bounds.size.height.floor().max(1.0),
        },
    }
}

fn apply_active_visibility_unlocked(
    tabs: &[(Browser, bool)],
    container: usize,
    active_id: Option<i32>,
) {
    for (browser, is_active) in tabs {
        set_browser_view_hidden(browser, !*is_active);
    }
    if container == 0 {
        return;
    }
    let container_view = unsafe { &*(container as *const NSView) };
    let bounds = integral_bounds(container_view.bounds());
    let mask =
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable;
    for (browser, is_active) in tabs {
        let Some(host) = browser.host() else {
            continue;
        };
        let handle = host.window_handle();
        if handle.is_null() {
            continue;
        }
        let view = unsafe { &*(handle as *const NSView) };
        view.setFrame(bounds);
        view.setAutoresizingMask(mask);
        if *is_active {
            host.was_resized();
            host.notify_screen_info_changed();
        }
    }
    let _ = active_id;
}

fn snapshot_tabs_for_layout() -> Option<(Vec<(Browser, bool)>, usize, Option<i32>)> {
    let g = state().lock().ok()?;
    let tabs: Vec<(Browser, bool)> = g
        .tabs
        .iter()
        .map(|t| (t.browser.clone(), Some(t.id) == g.active_id))
        .collect();
    Some((tabs, g.container, g.active_id))
}

fn request_layout_after_unlock() {
    LAYOUT_PENDING.store(true, Ordering::SeqCst);
}

fn drain_pending_browser_work() {
    // 1) Deferred Browser drops — never while holding HostState or inside CEF callbacks.
    let drop_list = PENDING_BROWSER_DROPS
        .lock()
        .ok()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    drop(drop_list);

    // 2) Deferred popup→tab creates (never create browsers inside OnBeforePopup).
    let popup_urls = PENDING_POPUP_URLS
        .lock()
        .ok()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default();
    for url in popup_urls {
        if let Err(e) = create_browser_in_container(&url) {
            tracing::warn!(target: "anycode_browser_cef", error = %e, "deferred popup→tab failed");
        }
    }

    // 3) Timed force-close for tabs that stayed after soft close.
    let due_force: Vec<i32> = FORCE_CLOSE_AFTER
        .lock()
        .ok()
        .map(|mut pending| {
            let now = Instant::now();
            let mut due = Vec::new();
            pending.retain(|(id, deadline)| {
                if now >= *deadline {
                    due.push(*id);
                    false
                } else {
                    true
                }
            });
            due
        })
        .unwrap_or_default();
    for id in due_force {
        let browser = {
            let Ok(g) = state().lock() else {
                continue;
            };
            g.tabs
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.browser.clone())
        };
        if let Some(browser) = browser {
            if let Some(host) = browser.host() {
                host.close_browser(1);
            }
        }
    }

    // 4) Layout — snapshot under lock, CEF/AppKit calls outside.
    if !LAYOUT_PENDING.swap(false, Ordering::SeqCst) {
        return;
    }
    if let Some((tabs, container, active_id)) = snapshot_tabs_for_layout() {
        apply_active_visibility_unlocked(&tabs, container, active_id);
    }
}

fn set_container_retina_scale(container: &NSView) {
    let scale = container
        .window()
        .map(|w| w.backingScaleFactor())
        .unwrap_or(2.0);
    if let Some(layer) = container.layer() {
        // Avoid 1× layer contents on a 2× window (top-left quarter paint).
        let _: () = unsafe { msg_send![&*layer, setContentsScale: scale] };
    }
}

fn container_child_bounds() -> Rect {
    let container = state().lock().ok().map(|g| g.container).unwrap_or(0);
    if container == 0 {
        return Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
    }
    let view = unsafe { &*(container as *const NSView) };
    let bounds = integral_bounds(view.bounds());
    Rect {
        x: 0,
        y: 0,
        // Never hand CEF a zero-size child widget — a transient 0×0 container
        // (layout settling, occluded window) must not create a 0-area browser.
        width: (bounds.size.width as i32).max(1),
        height: (bounds.size.height as i32).max(1),
    }
}

fn create_browser_in_container(url: &str) -> Result<(), String> {
    let container = {
        let g = state().lock().map_err(|e| e.to_string())?;
        if g.container == 0 {
            return Err("CEF container not ready".into());
        }
        g.container
    };
    let bounds = container_child_bounds();
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..Default::default()
    }
    .set_as_child((container as *mut std::ffi::c_void).cast(), &bounds);

    let settings = BrowserSettings::default();
    let start_url = if url.trim().is_empty() {
        "about:blank"
    } else {
        url
    };
    let url_cef = CefString::from(start_url);
    let mut client = EmbedClient::new(RefCell::new(ClientInner {}));

    let ok = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url_cef),
        Some(&settings),
        None,
        None,
    );
    if ok != 1 {
        return Err("browser_host_create_browser failed".into());
    }
    // The container is hidden after a last-tab close; new_tab / popup paths
    // don't go through ensure_container, so unhide here or the fresh browser
    // renders into an invisible container.
    unsafe {
        let view = &*(container as *const NSView);
        view.setHidden(false);
    }
    Ok(())
}

wrap_life_span_handler! {
    struct EmbedLifeSpan {}

    impl LifeSpanHandler {
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            // Only hijack real navigation popups into in-panel tabs.
            // Returning 1 for *all* popups cancels HTML <select> menus and
            // other Alloy widgets that also arrive via OnBeforePopup.
            let url = target_url
                .map(CefString::to_string)
                .unwrap_or_default();
            let url = url.trim().to_string();
            let as_tab = matches!(
                target_disposition,
                WindowOpenDisposition::NEW_FOREGROUND_TAB
                    | WindowOpenDisposition::NEW_BACKGROUND_TAB
                    | WindowOpenDisposition::NEW_WINDOW
            ) || (matches!(target_disposition, WindowOpenDisposition::NEW_POPUP)
                && !url.is_empty()
                && url != "about:blank");
            if as_tab && !url.is_empty() {
                // Never create browsers inside OnBeforePopup — defer to next pump.
                if let Ok(mut q) = PENDING_POPUP_URLS.lock() {
                    q.push(url);
                }
                schedule_pump_work(0);
                return 1;
            }
            // Allow native select / widget popups.
            0
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser.cloned() else {
                return;
            };
            let id = browser.identifier();
            let url = browser
                .main_frame()
                .map(|f| {
                    let u = f.url();
                    CefString::from(&u).to_string()
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "about:blank".into());
            if let Ok(mut g) = state().lock() {
                if g.tabs.iter().any(|t| t.id == id) {
                    return;
                }
                g.tabs.push(BrowserTab {
                    id,
                    browser,
                    url,
                    title: String::new(),
                    placeholder: false,
                });
                g.active_id = Some(id);
            }
            request_layout_after_unlock();
            schedule_pump_work(0);
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else {
                return;
            };
            let id = browser.identifier();
            let removed = {
                let Ok(mut g) = state().lock() else {
                    return;
                };
                let removed = g
                    .tabs
                    .iter()
                    .position(|t| t.id == id)
                    .map(|idx| g.tabs.remove(idx).browser);
                if g.active_id == Some(id) {
                    g.active_id = g.tabs.last().map(|t| t.id);
                }
                removed
            };
            if let Some(browser) = removed {
                if let Ok(mut pending) = PENDING_BROWSER_DROPS.lock() {
                    pending.push(browser);
                }
            }
            // Never call was_resized / setHidden while holding the state mutex
            // inside OnBeforeClose — CEF may re-enter DisplayHandler and deadlock.
            request_layout_after_unlock();
            schedule_pump_work(0);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> ::std::os::raw::c_int {
            // Returning 0 makes CEF forward close to the top-level NSWindow
            // (Tauri main window) → the whole app quits when closing a tab.
            // Return 1 and detach the Alloy child view so only that tab dies.
            if let Some(browser) = browser {
                if let Some(host) = browser.host() {
                    let handle = host.window_handle();
                    if !handle.is_null() {
                        let view = unsafe { &*(handle as *const NSView) };
                        view.removeFromSuperview();
                    }
                }
            }
            1
        }
    }
}

wrap_display_handler! {
    struct EmbedDisplay {}

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let Some(browser) = browser else {
                return;
            };
            let id = browser.identifier();
            let title = title.map(CefString::to_string).unwrap_or_default();
            if let Ok(mut g) = state().lock() {
                if let Some(tab) = g.tabs.iter_mut().find(|t| t.id == id) {
                    tab.title = title;
                }
            }
        }

        fn on_address_change(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            let Some(browser) = browser else {
                return;
            };
            let id = browser.identifier();
            let url = url.map(CefString::to_string).unwrap_or_default();
            if let Ok(mut g) = state().lock() {
                if let Some(tab) = g.tabs.iter_mut().find(|t| t.id == id) {
                    tab.url = url;
                }
            }
        }
    }
}

fn load_framework(exe: &Path) -> Result<library_loader::LibraryLoader, String> {
    let loader = library_loader::LibraryLoader::new(exe, false);
    if !loader.load() {
        return Err(format!(
            "failed to load Chromium Embedded Framework (exe={})",
            exe.display()
        ));
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    Ok(loader)
}

fn retain_loader(loader: library_loader::LibraryLoader) {
    static LOADER: OnceLock<library_loader::LibraryLoader> = OnceLock::new();
    let _ = LOADER.set(loader);
}

pub fn ensure_initialized(helper_path: &Path, _framework_dir: &Path) -> Result<(), String> {
    if INITIALIZED.load(Ordering::SeqCst) {
        return Ok(());
    }

    // Link + install CefAppProtocol methods on NSApplication (category +load).
    unsafe {
        anycode_cef_force_link_app_protocol();
    }

    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let loader = load_framework(&exe)?;
    retain_loader(loader);

    let args = args::Args::new();
    let main_args = args.as_main_args();

    if let Some(cmd) = args.as_cmd_line() {
        let switch = CefString::from("type");
        if cmd.has_switch(Some(&switch)) == 1 {
            let _ = execute_process(Some(&main_args), None, std::ptr::null_mut());
            return Err("CEF subprocess should use anycode-cef-helper".into());
        }
    }

    let mut app = EmbedApp::new();
    let cache_root = std::env::temp_dir().join("anycode-cef-cache");
    let _ = std::fs::create_dir_all(&cache_root);
    // The CDP port is how the agent's Browser* tools find this CEF instance
    // (ANYCODE_CEF_CDP_PORT). 9333 may legitimately be taken (another app, a
    // zombie CEF), so probe for a free port instead of silently failing to
    // bind the debugging server.
    let debug_port = pick_debug_port()?;
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        remote_debugging_port: i32::from(debug_port),
        browser_subprocess_path: CefString::from(helper_path.to_string_lossy().as_ref()),
        root_cache_path: CefString::from(cache_root.to_string_lossy().as_ref()),
        log_severity: LogSeverity::WARNING,
        ..Default::default()
    };

    let rc = initialize(
        Some(&main_args),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if rc != 1 {
        return Err(format!("cef::initialize failed (code={rc})"));
    }

    DEBUG_PORT.store(debug_port, Ordering::SeqCst);
    INITIALIZED.store(true, Ordering::SeqCst);
    tracing::info!(
        target: "anycode_browser_cef",
        port = debug_port,
        helper = %helper_path.display(),
        "CEF initialized (external message pump, Alloy child views)"
    );
    Ok(())
}

/// First free loopback port in `DEFAULT_DEBUG_PORT..+DEBUG_PORT_PROBE_RANGE`.
fn pick_debug_port() -> Result<u16, String> {
    for offset in 0..DEBUG_PORT_PROBE_RANGE {
        let port = DEFAULT_DEBUG_PORT + offset;
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(format!(
        "no free CDP port in {}..{} (another CEF instance?)",
        DEFAULT_DEBUG_PORT,
        DEFAULT_DEBUG_PORT + DEBUG_PORT_PROBE_RANGE - 1
    ))
}

fn content_view_ptr(ns_window: *mut std::ffi::c_void) -> Result<*mut AnyObject, String> {
    if ns_window.is_null() {
        return Err("null NSWindow".into());
    }
    let _mtm = MainThreadMarker::new().ok_or("CEF host must run on the main thread")?;
    let window = unsafe { &*(ns_window as *const NSWindow) };
    let view = window.contentView().ok_or("NSWindow has no contentView")?;
    Ok((&*view as *const NSView as *mut NSView).cast())
}

fn top_left_to_cocoa(parent: *mut AnyObject, rect: EmbedRect) -> Result<NSRect, String> {
    let view = unsafe { &*(parent as *const NSView) };
    let bounds = view.bounds();
    let height = bounds.size.height;
    // Floor to whole points — fractional frames confuse CEF layout on Retina.
    let x = rect.x.floor();
    let y = rect.y.floor();
    let width = rect.width.floor().max(1.0);
    let height_r = rect.height.floor().max(1.0);
    Ok(integral_bounds(NSRect {
        origin: NSPoint {
            x,
            y: (height - y - height_r).floor(),
        },
        size: NSSize {
            width,
            height: height_r,
        },
    }))
}

fn ensure_container(parent: *mut AnyObject, rect: EmbedRect) -> Result<*mut AnyObject, String> {
    let existing = {
        let g = state().lock().map_err(|e| e.to_string())?;
        g.container
    };
    if existing != 0 {
        let container = existing as *mut AnyObject;
        let frame = top_left_to_cocoa(parent, rect)?;
        let view = unsafe { &*(container as *const NSView) };
        view.setFrame(frame);
        view.setHidden(false);
        return Ok(container);
    }

    let _mtm = MainThreadMarker::new().ok_or("main thread required")?;
    let parent_view = unsafe { &*(parent as *const NSView) };
    let frame = top_left_to_cocoa(parent, rect)?;

    let container: *mut AnyObject = unsafe {
        let cls = NSView::class();
        let alloc: *mut AnyObject = msg_send![cls, alloc];
        let init: *mut AnyObject = msg_send![alloc, initWithFrame: frame];
        let sub: &NSView = &*init.cast::<NSView>();
        // Let Alloy widget popups (select menus) paint outside the panel rect
        // when they are child views of this container.
        sub.setWantsLayer(true);
        sub.setClipsToBounds(false);
        parent_view.addSubview(sub);
        set_container_retina_scale(sub);
        init
    };
    if container.is_null() {
        return Err("failed to create CEF container NSView".into());
    }
    if let Ok(mut g) = state().lock() {
        g.container = container as usize;
    }
    Ok(container)
}

pub fn show_in_parent(
    ns_window: *mut std::ffi::c_void,
    rect: EmbedRect,
    url: &str,
) -> Result<(), String> {
    if !INITIALIZED.load(Ordering::SeqCst) {
        return Err("CEF not initialized — call ensure_initialized first".into());
    }
    let parent = content_view_ptr(ns_window)?;
    let _container = ensure_container(parent, rect)?;

    let has_live = {
        let g = state().lock().map_err(|e| e.to_string())?;
        g.tabs.iter().any(|t| !t.placeholder)
    };
    if has_live {
        // Layout / design-mode toggles must never hijack the active tab URL.
        resize(rect)?;
        return Ok(());
    }

    if reactivate_placeholder_tab(url)? {
        resize(rect)?;
        return Ok(());
    }

    create_browser_in_container(url)
}

fn reactivate_placeholder_tab(url: &str) -> Result<bool, String> {
    let browser = {
        let mut g = state().lock().map_err(|e| e.to_string())?;
        let Some(idx) = g.tabs.iter().position(|t| t.placeholder) else {
            return Ok(false);
        };
        let tab_id = g.tabs[idx].id;
        let tab = &mut g.tabs[idx];
        tab.placeholder = false;
        tab.url = if url.trim().is_empty() {
            "about:blank".into()
        } else {
            url.to_string()
        };
        tab.title.clear();
        let browser = tab.browser.clone();
        g.active_id = Some(tab_id);
        browser
    };
    let start_url = normalize_nav_url(if url.trim().is_empty() {
        "about:blank"
    } else {
        url
    });
    if !start_url.is_empty() {
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&CefString::from(start_url.as_str())));
        }
    }
    let container = state().lock().ok().map(|g| g.container).unwrap_or(0);
    if container != 0 {
        let view = unsafe { &*(container as *const NSView) };
        view.setHidden(false);
    }
    if let Some((tabs, container_id, active_id)) = snapshot_tabs_for_layout() {
        apply_active_visibility_unlocked(&tabs, container_id, active_id);
    }
    schedule_pump_work(0);
    Ok(true)
}

pub fn resize(rect: EmbedRect) -> Result<(), String> {
    let container = {
        let g = state().lock().map_err(|e| e.to_string())?;
        g.container
    };
    if container == 0 {
        return Ok(());
    }
    let container = container as *mut AnyObject;
    let view = unsafe { &*(container as *const NSView) };
    let parent = unsafe { view.superview() };
    let Some(parent) = parent else {
        return Ok(());
    };
    let parent_ptr = (&*parent as *const NSView as *mut NSView).cast();
    let frame = top_left_to_cocoa(parent_ptr, rect)?;
    view.setFrame(frame);
    view.setHidden(false);
    set_container_retina_scale(view);
    // Resize child Alloy views outside any HostState lock (via next pump drain
    // or immediately with a snapshot — prefer immediate for responsive layout).
    if let Some((tabs, container_id, active_id)) = snapshot_tabs_for_layout() {
        apply_active_visibility_unlocked(&tabs, container_id, active_id);
    }
    Ok(())
}

pub fn hide() {
    let container = state().lock().ok().map(|g| g.container).unwrap_or(0);
    if container == 0 {
        return;
    }
    let view = unsafe { &*(container as *const NSView) };
    view.setHidden(true);
}

fn normalize_nav_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }
    if url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("about:")
        || url.starts_with("file:")
        || url.starts_with("data:")
        || url.starts_with("chrome:")
    {
        return url.to_string();
    }
    // Bare host like "baidu.com" → https
    format!("https://{url}")
}

pub fn navigate(url: &str) -> Result<(), String> {
    let url = normalize_nav_url(url);
    if url.is_empty() {
        return Err("empty url".into());
    }
    let browser = {
        let g = state().lock().map_err(|e| e.to_string())?;
        g.active_tab().map(|t| t.browser.clone())
    };
    let Some(browser) = browser else {
        return Err("no CEF browser yet".into());
    };
    let Some(frame) = browser.main_frame() else {
        return Err("no main frame".into());
    };
    let cef_url = CefString::from(url.as_str());
    // LoadURL is async; must not pump the message loop synchronously here,
    // and must not hold HostState across the call (OnAddressChange re-enters).
    frame.load_url(Some(&cef_url));
    Ok(())
}

pub fn new_tab(url: &str) -> Result<(), String> {
    create_browser_in_container(url)
}

pub fn select_tab(id: i32) -> Result<(), String> {
    {
        let mut g = state().lock().map_err(|e| e.to_string())?;
        if !g.tabs.iter().any(|t| t.id == id) {
            return Err(format!("unknown tab {id}"));
        }
        g.active_id = Some(id);
    }
    if let Some((tabs, container, active_id)) = snapshot_tabs_for_layout() {
        apply_active_visibility_unlocked(&tabs, container, active_id);
    }
    Ok(())
}

pub fn close_tab(id: i32) -> Result<(), String> {
    let last_tab = {
        let g = state().lock().map_err(|e| e.to_string())?;
        if !g.tabs.iter().any(|t| t.id == id) {
            eprintln!(
                "anycode-cef: close_tab({id}) unknown (live tabs: {:?})",
                g.tabs.iter().map(|t| t.id).collect::<Vec<_>>()
            );
            return Err(format!("unknown tab {id}"));
        }
        g.tabs.len() == 1
    };
    eprintln!("anycode-cef: close_tab({id}) last_tab={last_tab}");
    if last_tab {
        // Last tab: keep the CEF host alive as a hidden placeholder. Force-
        // closing the final browser races the message pump (EXC_BAD_ACCESS in
        // do_message_loop_work). Hide the panel and blank the page instead.
        hide();
        let browser = {
            let mut g = state().lock().map_err(|e| e.to_string())?;
            g.active_id = None;
            g.tabs.iter_mut().find(|t| t.id == id).map(|t| {
                t.placeholder = true;
                t.url = "about:blank".into();
                t.title.clear();
                t.browser.clone()
            })
        };
        if let Some(browser) = browser {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from("about:blank")));
            }
            set_browser_view_hidden(&browser, true);
        }
        if let Ok(mut pending) = FORCE_CLOSE_AFTER.lock() {
            pending.retain(|(existing, _)| *existing != id);
        }
        schedule_pump_work(0);
        return Ok(());
    }
    let browser = {
        let g = state().lock().map_err(|e| e.to_string())?;
        g.tabs
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.browser.clone())
    };
    let Some(browser) = browser else {
        return Err(format!("unknown tab {id}"));
    };
    // Soft close only. Force after 2s if the tab is still present (drained on pump).
    // DoClose returns 1 so CEF must not performClose the Tauri NSWindow.
    if let Some(host) = browser.host() {
        host.close_browser(0);
    }
    if let Ok(mut pending) = FORCE_CLOSE_AFTER.lock() {
        pending.retain(|(existing, _)| *existing != id);
        pending.push((id, Instant::now() + Duration::from_secs(2)));
    }
    schedule_pump_work(0);
    // Ensure the 2s force-close deadline is observed even if CEF goes idle.
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(2));
        schedule_pump_work(0);
    });
    Ok(())
}

pub fn list_tabs() -> Vec<TabInfo> {
    state()
        .lock()
        .ok()
        .map(|g| {
            g.tabs
                .iter()
                .filter(|t| !t.placeholder)
                .map(|t| TabInfo {
                    id: t.id,
                    url: t.url.clone(),
                    title: if t.title.is_empty() {
                        t.url.clone()
                    } else {
                        t.title.clone()
                    },
                    active: Some(t.id) == g.active_id,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Cheap liveness probe for the pump scheduler (no Vec allocation).
pub fn has_tabs() -> bool {
    state()
        .lock()
        .map(|g| g.tabs.iter().any(|t| !t.placeholder))
        .unwrap_or(false)
}

pub fn current_url() -> Option<String> {
    state()
        .lock()
        .ok()
        .and_then(|g| g.active_tab().map(|t| t.url.clone()))
        .filter(|s| !s.is_empty())
}

pub fn title() -> Option<String> {
    state()
        .lock()
        .ok()
        .and_then(|g| g.active_tab().map(|t| t.title.clone()))
        .filter(|s| !s.is_empty())
}

pub fn remote_debugging_port() -> u16 {
    DEBUG_PORT.load(Ordering::SeqCst)
}

/// Re-entrancy guard: CEF forbids nested `CefDoMessageLoopWork` calls.
static PUMP_BUSY: AtomicBool = AtomicBool::new(false);

pub fn do_message_loop_work() {
    if !INITIALIZED.load(Ordering::SeqCst) {
        return;
    }
    if PUMP_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    cef::do_message_loop_work();
    PUMP_BUSY.store(false, Ordering::SeqCst);
    drain_pending_browser_work();
}

pub fn shutdown_cef() {
    if !INITIALIZED.swap(false, Ordering::SeqCst) {
        return;
    }
    DEBUG_PORT.store(0, Ordering::SeqCst);
    let (browsers, container) = {
        let Ok(mut g) = state().lock() else {
            shutdown();
            return;
        };
        let browsers: Vec<Browser> = g.tabs.drain(..).map(|t| t.browser).collect();
        g.active_id = None;
        let container = g.container;
        g.container = 0;
        (browsers, container)
    };
    // Unlock before close_browser — OnBeforeClose takes HostState again.
    for browser in browsers {
        if let Some(host) = browser.host() {
            host.close_browser(1);
        }
    }
    if container != 0 {
        let view = unsafe { &*(container as *const NSView) };
        view.removeFromSuperview();
    }
    if let Ok(mut pending) = FORCE_CLOSE_AFTER.lock() {
        pending.clear();
    }
    if let Ok(mut pending) = PENDING_POPUP_URLS.lock() {
        pending.clear();
    }
    shutdown();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_debug_port_skips_occupied_ports() {
        // Skip when 9333 is already taken by something else (e.g. a running
        // dev instance) — we cannot control the environment in that case.
        let Ok(blocker) = std::net::TcpListener::bind(("127.0.0.1", DEFAULT_DEBUG_PORT)) else {
            return;
        };
        let port = pick_debug_port().unwrap();
        assert_eq!(port, DEFAULT_DEBUG_PORT + 1);
        drop(blocker);
    }

    #[test]
    fn pick_debug_port_uses_default_when_free() {
        // Only meaningful when nothing else holds 9333; guard to avoid flakes
        // when a real CEF instance is running on the dev machine.
        if std::net::TcpListener::bind(("127.0.0.1", DEFAULT_DEBUG_PORT)).is_err() {
            return;
        }
        assert_eq!(pick_debug_port().unwrap(), DEFAULT_DEBUG_PORT);
    }
}
