//! Tauri commands that host an embedded CEF view over the Workbench panel.

use anycode_browser_cef::{EmbedRect, TabInfo};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

static PUMP_STARTED: AtomicBool = AtomicBool::new(false);

/// 空闲兜底 pump 间隔（约 60fps）。external message pump 模式下 CDP 服务器依赖
/// do_message_loop_work 被调用才能服务请求；无活动渲染/输入时 CEF 不会调度，
/// 因此按此间隔强制 pump，避免 9333 端口饿死导致 attach 超时。
const IDLE_PUMP_INTERVAL_MS: u64 = 16;

/// 无存活 tab 时的空闲 pump 间隔。CDP attach 只在有浏览器实例后才发生，零 tab
/// 时高频 pump 没有受益方，却持续把 CEF 内部生命周期代码（已观察到在其内部
/// 查表空指针崩溃）放在暴露面上——降到 10fps 砍掉约 84% 的无谓 pump。
const NO_TAB_IDLE_PUMP_INTERVAL_MS: u64 = 100;

fn pump_deadlines() -> &'static (Mutex<Option<Instant>>, Condvar) {
    static CELL: OnceLock<(Mutex<Option<Instant>>, Condvar)> = OnceLock::new();
    CELL.get_or_init(|| (Mutex::new(None), Condvar::new()))
}

#[derive(Debug, Clone, Serialize)]
pub struct CefEmbedStatus {
    pub ready: bool,
    pub remote_debugging_port: u16,
    pub url: Option<String>,
    pub title: Option<String>,
    pub tabs: Vec<TabInfo>,
    pub active_tab_id: Option<i32>,
    /// True when the crash guard disabled the embed for this run.
    pub crash_guard_disabled: bool,
}

fn framework_dir() -> PathBuf {
    // Prefer the app bundle Frameworks so Helper.app + CEF.framework stay siblings.
    // (CEF_PATH is only a download/cache location for prepare-cef / builds.)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos) = exe.parent() {
            if let Some(contents) = macos.parent() {
                let fw = contents.join("Frameworks");
                if fw.join("Chromium Embedded Framework.framework").exists() {
                    return fw;
                }
            }
        }
    }
    if let Ok(p) = std::env::var("CEF_PATH") {
        let p = PathBuf::from(p);
        if p.join("Chromium Embedded Framework.framework").exists() {
            return p;
        }
    }
    PathBuf::from("/Applications/anyCode.app/Contents/Frameworks")
}

fn helper_path() -> PathBuf {
    if let Ok(p) = std::env::var("ANYCODE_CEF_HELPER") {
        return PathBuf::from(p);
    }
    let frameworks = framework_dir();
    // Preferred: Helper.app/Contents/MacOS/anyCode Helper (CEF subprocess layout)
    let nested = frameworks
        .join("anyCode Helper.app")
        .join("Contents/MacOS")
        .join("anyCode Helper");
    if nested.exists() {
        return nested;
    }
    // Dev fallback: sibling binary next to anycode-desktop
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("anycode-cef-helper");
            if cand.exists() {
                return cand;
            }
        }
    }
    nested
}

/// Default on for sharp native preview.
///
/// Kill-switch / JPEG fallback: set `ANYCODE_CEF_EMBED=0` (also `false`/`off`/`no`)
/// before launch to skip in-process CEF and prefer the JPEG screencast path in
/// the Browser panel. There is no Workbench settings toggle yet — env (or the
/// crash-guard auto-disable) is the supported way to prefer JPEG.
fn cef_embed_env_enabled() -> bool {
    !matches!(
        std::env::var("ANYCODE_CEF_EMBED")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

// ---------- CEF crash guard ----------
// CEF runs in-process: a segfault inside it kills the whole app (DiagnosticReports
// shows the same Chromium-internal null deref inside cef_do_message_loop_work
// across releases — not patchable from our side). To stop a crash loop from
// bricking the workbench, count consecutive unclean exits that happened while
// CEF was active; once the count reaches GUARD_TRIP_THRESHOLD the embed is
// disabled for one run (the browser panel falls back to JPEG screencast). A
// clean exit resets the counter, so the guard self-heals on the next launch.
static CRASH_GUARD_TRIPPED: AtomicBool = AtomicBool::new(false);
const GUARD_TRIP_THRESHOLD: u32 = 2;

#[derive(Debug, Default, Clone, Copy, serde::Serialize, serde::Deserialize)]
struct CefGuardState {
    count: u32,
    active: bool,
}

fn guard_path() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".anycode/cef-guard.json")
}

fn read_guard(path: &std::path::Path) -> CefGuardState {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_guard(path: &std::path::Path, state: CefGuardState) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&state) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

/// Evaluate the guard once at startup. `active` in the marker means the previous
/// run died without its clean-exit handler running while CEF was initialized.
fn evaluate_crash_guard_at(path: &std::path::Path) -> bool {
    let prev = read_guard(path);
    let count = if prev.active {
        prev.count.saturating_add(1)
    } else {
        prev.count
    };
    let tripped = count >= GUARD_TRIP_THRESHOLD;
    write_guard(
        path,
        CefGuardState {
            count,
            active: false,
        },
    );
    tripped
}

/// Called once at app startup; trips the process-wide kill-switch when the
/// previous runs kept dying with CEF active.
pub fn evaluate_crash_guard() {
    let tripped = evaluate_crash_guard_at(&guard_path());
    if tripped {
        CRASH_GUARD_TRIPPED.store(true, Ordering::SeqCst);
        eprintln!(
            "anycode-desktop: CEF crash guard tripped ({GUARD_TRIP_THRESHOLD} consecutive unclean \
             exits with CEF active) — embed disabled for this run, browser panel uses JPEG fallback"
        );
        let _ = std::fs::write(
            PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".anycode/cef-status.txt"),
            format!("crash-guard tripped threshold={GUARD_TRIP_THRESHOLD}\n"),
        );
    }
}

/// After CEF init succeeded: this run is CEF-active until the clean-exit handler.
fn mark_cef_active_at(path: &std::path::Path) {
    let mut state = read_guard(path);
    state.active = true;
    write_guard(path, state);
}

/// Clean-exit handler: reset the counter so the next launch re-enables CEF.
fn mark_clean_exit_at(path: &std::path::Path) {
    write_guard(path, CefGuardState::default());
}

/// Called from the app Exit/ExitRequested handler (after shutdown_cef).
pub fn mark_clean_exit() {
    mark_clean_exit_at(&guard_path());
}

pub fn crash_guard_tripped() -> bool {
    CRASH_GUARD_TRIPPED.load(Ordering::SeqCst)
}

fn ensure_cef() -> Result<(), String> {
    if !cef_embed_env_enabled() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Err("CEF embed disabled via ANYCODE_CEF_EMBED=0".into());
    }
    if crash_guard_tripped() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Err(
            "CEF embed auto-disabled by crash guard (previous runs crashed inside CEF); \
             JPEG fallback active — restart the app to retry native preview"
                .into(),
        );
    }
    // Prefer Frameworks/Chromium Embedded Framework.framework parent as CEF_PATH for loader.
    let fw = framework_dir();
    let cef_root = if fw.join("Chromium Embedded Framework.framework").exists() {
        // LibraryLoader looks relative to the .app; also set CEF_PATH for dyld.
        std::env::set_var(
            "DYLD_FALLBACK_LIBRARY_PATH",
            format!(
                "{}:{}:{}",
                std::env::var("DYLD_FALLBACK_LIBRARY_PATH").unwrap_or_default(),
                fw.display(),
                fw.join("Chromium Embedded Framework.framework/Libraries")
                    .display()
            ),
        );
        fw.clone()
    } else if PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".local/share/cef/Chromium Embedded Framework.framework")
        .exists()
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share/cef")
    } else {
        return Err(
            "Chromium Embedded Framework not found. Run scripts/prepare-cef.sh then sync desktop."
                .into(),
        );
    };
    let helper = helper_path();
    if !helper.exists() {
        return Err(format!(
            "CEF helper missing at {} — run scripts/prepare-cef.sh",
            helper.display()
        ));
    }
    match anycode_browser_cef::ensure_initialized(&helper, &cef_root) {
        Ok(()) => {
            mark_cef_active_at(&guard_path());
            eprintln!(
                "anycode-desktop: CEF ready helper={} cef_root={} port={}",
                helper.display(),
                cef_root.display(),
                anycode_browser_cef::remote_debugging_port()
            );
            let _ = std::fs::write(
                PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".anycode/cef-status.txt"),
                format!(
                    "ok helper={} cef_root={} port={}\n",
                    helper.display(),
                    cef_root.display(),
                    anycode_browser_cef::remote_debugging_port()
                ),
            );
            Ok(())
        }
        Err(e) => {
            eprintln!("anycode-desktop: CEF init failed: {e}");
            let _ = std::fs::write(
                PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".anycode/cef-status.txt"),
                format!("err {e}\n"),
            );
            // Kill-switch: do not leave a half-published CDP port for attach.
            std::env::remove_var("ANYCODE_CEF_CDP_PORT");
            Err(e)
        }
    }
}

/// Single background scheduler: merges OnScheduleMessagePumpWork delays into one
/// deadline and posts DoMessageLoopWork to the AppKit thread. Never pumps inline
/// from the CEF schedule callback, and never spams an 80ms idle loop.
fn start_message_pump<R: Runtime>(app: &AppHandle<R>) {
    if PUMP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    let (deadlines, cv) = pump_deadlines();

    anycode_browser_cef::set_schedule_pump_callback(Box::new({
        move |delay_ms| {
            let delay_ms = delay_ms.max(0) as u64;
            let at = Instant::now() + Duration::from_millis(delay_ms);
            if let Ok(mut g) = deadlines.lock() {
                match *g {
                    Some(existing) if existing <= at => {}
                    _ => *g = Some(at),
                }
                cv.notify_one();
            }
        }
    }));

    std::thread::Builder::new()
        .name("anycode-cef-pump".into())
        .spawn(move || {
            let (deadlines, cv) = pump_deadlines();
            loop {
                let mut guard = deadlines.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    let now = Instant::now();
                    match *guard {
                        None => {
                            // 空闲兜底：CEF 在 external message pump 模式下只通过
                            // OnScheduleMessagePumpWork 请求 pump，若它从不调度（例如
                            // 没有活动渲染/输入），CDP 服务器会一直饿死导致 attach 超时。
                            // 因此无 deadline 时也按空闲间隔 pump 一次。零 tab 时降频——
                            // 没有浏览器就没有 CDP 客户端，而每次 pump 都会执行 CEF 内部
                            // UI 线程任务（其生命周期代码曾崩溃，见 cef-guard 注释）。
                            let idle_ms = if anycode_browser_cef::has_tabs() {
                                IDLE_PUMP_INTERVAL_MS
                            } else {
                                NO_TAB_IDLE_PUMP_INTERVAL_MS
                            };
                            guard = cv
                                .wait_timeout(guard, Duration::from_millis(idle_ms))
                                .map(|(g, _)| g)
                                .unwrap_or_else(|e| e.into_inner().0);
                            *guard = None;
                            drop(guard);
                            break;
                        }
                        Some(at) if at > now => {
                            let wait = at.saturating_duration_since(now);
                            guard = cv
                                .wait_timeout(guard, wait)
                                .map(|(g, _)| g)
                                .unwrap_or_else(|e| e.into_inner().0);
                        }
                        Some(_) => {
                            *guard = None;
                            drop(guard);
                            break;
                        }
                    }
                }
                let handle = handle.clone();
                let _ = handle.run_on_main_thread(|| {
                    anycode_browser_cef::do_message_loop_work();
                });
            }
        })
        .expect("spawn anycode-cef-pump");
}

fn publish_cdp_port() {
    if !cef_embed_env_enabled() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return;
    }
    let port = anycode_browser_cef::remote_debugging_port();
    if port > 0 {
        std::env::set_var("ANYCODE_CEF_CDP_PORT", port.to_string());
        eprintln!("anycode-desktop: ANYCODE_CEF_CDP_PORT={port}");
    }
}

fn assets_present() -> bool {
    cef_embed_env_enabled()
        && helper_path().exists()
        && (framework_dir()
            .join("Chromium Embedded Framework.framework")
            .exists()
            || PathBuf::from(std::env::var("HOME").unwrap_or_default())
                .join(".local/share/cef/Chromium Embedded Framework.framework")
                .exists())
}

fn status_snapshot() -> CefEmbedStatus {
    let tabs = anycode_browser_cef::list_tabs();
    let active_tab_id = tabs.iter().find(|t| t.active).map(|t| t.id);
    CefEmbedStatus {
        ready: assets_present() && anycode_browser_cef::remote_debugging_port() > 0,
        remote_debugging_port: anycode_browser_cef::remote_debugging_port(),
        url: anycode_browser_cef::current_url(),
        title: anycode_browser_cef::title(),
        tabs,
        active_tab_id,
        crash_guard_disabled: crash_guard_tripped(),
    }
}

fn on_main_thread() -> bool {
    objc2::MainThreadMarker::new().is_some()
}

fn run_on_main<F, T>(app: &AppHandle, f: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    if on_main_thread() {
        return f();
    }
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

pub fn clear_stale_cdp_port_if_disabled() {
    if !cef_embed_env_enabled() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
    }
}

#[tauri::command]
pub fn cef_browser_status(_app: AppHandle) -> Result<CefEmbedStatus, String> {
    if matches!(
        std::env::var("ANYCODE_CEF_DEBUG").as_deref(),
        Ok("1") | Ok("true")
    ) {
        eprintln!(
            "anycode-desktop: cef_browser_status invoked (assets_present={}, port={})",
            assets_present(),
            anycode_browser_cef::remote_debugging_port()
        );
    }
    // Do not CefInitialize here — init happens in cef_browser_show once the
    // AppKit run loop is settled and the panel has a host rect.
    if !cef_embed_env_enabled() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Ok(CefEmbedStatus {
            ready: false,
            remote_debugging_port: 0,
            url: None,
            title: None,
            tabs: Vec::new(),
            active_tab_id: None,
            crash_guard_disabled: false,
        });
    }
    if crash_guard_tripped() {
        std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Ok(CefEmbedStatus {
            ready: false,
            remote_debugging_port: 0,
            url: None,
            title: None,
            tabs: Vec::new(),
            active_tab_id: None,
            crash_guard_disabled: true,
        });
    }
    let tabs = anycode_browser_cef::list_tabs();
    let active_tab_id = tabs.iter().find(|t| t.active).map(|t| t.id);
    Ok(CefEmbedStatus {
        ready: assets_present(),
        remote_debugging_port: anycode_browser_cef::remote_debugging_port(),
        url: anycode_browser_cef::current_url(),
        title: anycode_browser_cef::title(),
        tabs,
        active_tab_id,
        crash_guard_disabled: false,
    })
}

#[tauri::command]
pub fn cef_browser_show(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    url: String,
) -> Result<CefEmbedStatus, String> {
    eprintln!(
        "anycode-desktop: cef_browser_show invoked rect=({x},{y} {width}x{height}) url={url}"
    );
    let app_main = app.clone();
    run_on_main(&app, move || {
        ensure_cef()?;
        start_message_pump(&app_main);
        let window = app_main
            .get_webview_window("main")
            .ok_or("main window missing")?;
        let ns_window = window.ns_window().map_err(|e| e.to_string())?;
        anycode_browser_cef::show_in_parent(
            ns_window,
            EmbedRect {
                x,
                y,
                width,
                height,
            },
            &url,
        )?;
        publish_cdp_port();
        Ok(status_snapshot())
    })
}

#[tauri::command]
pub fn cef_browser_resize(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    run_on_main(&app, move || {
        anycode_browser_cef::resize(EmbedRect {
            x,
            y,
            width,
            height,
        })
    })
}

#[tauri::command]
pub fn cef_browser_hide(app: AppHandle) {
    let _ = run_on_main(&app, || {
        anycode_browser_cef::hide();
        Ok(())
    });
}

#[tauri::command]
pub fn cef_browser_navigate(app: AppHandle, url: String) -> Result<CefEmbedStatus, String> {
    run_on_main(&app, move || {
        anycode_browser_cef::navigate(&url)?;
        Ok(status_snapshot())
    })
}

#[tauri::command]
pub fn cef_browser_new_tab(app: AppHandle, url: Option<String>) -> Result<CefEmbedStatus, String> {
    run_on_main(&app, move || {
        let url = url.unwrap_or_else(|| "about:blank".into());
        anycode_browser_cef::new_tab(&url)?;
        Ok(status_snapshot())
    })
}

#[tauri::command]
pub fn cef_browser_select_tab(app: AppHandle, id: i32) -> Result<CefEmbedStatus, String> {
    run_on_main(&app, move || {
        anycode_browser_cef::select_tab(id)?;
        Ok(status_snapshot())
    })
}

#[tauri::command]
pub fn cef_browser_close_tab(app: AppHandle, id: i32) -> Result<CefEmbedStatus, String> {
    eprintln!("anycode-desktop: cef_browser_close_tab invoked id={id}");
    let r = run_on_main(&app, move || {
        anycode_browser_cef::close_tab(id)?;
        Ok(status_snapshot())
    });
    let tab_count: Option<usize> = match &r {
        Ok(s) => Some(s.tabs.len()),
        Err(_) => None,
    };
    eprintln!(
        "anycode-desktop: cef_browser_close_tab id={id} -> tabs={tab_count:?} err={}",
        r.is_err()
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_guard_trips_after_threshold_unclean_active_exits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cef-guard.json");

        // Clean runs never trip and never accumulate.
        assert!(!evaluate_crash_guard_at(&path));
        mark_clean_exit_at(&path);
        assert!(!evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, 0);

        // First unclean exit with CEF active: counted, not yet tripped.
        mark_cef_active_at(&path);
        assert!(!evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, 1);

        // Second consecutive unclean active exit: trips.
        mark_cef_active_at(&path);
        assert!(evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, GUARD_TRIP_THRESHOLD);

        // A clean exit resets the counter so CEF is retried next launch.
        mark_clean_exit_at(&path);
        assert!(!evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, 0);
    }

    #[test]
    fn crash_guard_clean_exit_breaks_the_streak() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cef-guard.json");

        // Unclean active exit, then a clean one: count must not accumulate.
        mark_cef_active_at(&path);
        assert!(!evaluate_crash_guard_at(&path));
        mark_clean_exit_at(&path);
        mark_cef_active_at(&path);
        assert!(!evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, 1);
    }

    #[test]
    fn crash_guard_tolerates_corrupt_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cef-guard.json");
        std::fs::write(&path, "{not json").unwrap();
        assert!(!evaluate_crash_guard_at(&path));
        assert_eq!(read_guard(&path).count, 0);
    }
}
