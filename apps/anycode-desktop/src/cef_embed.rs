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

/// Default on for sharp native preview. Set ANYCODE_CEF_EMBED=0 to force
/// JPEG screencast instead (kill-switch).
fn cef_embed_env_enabled() -> bool {
    !matches!(
        std::env::var("ANYCODE_CEF_EMBED")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

fn ensure_cef() -> Result<(), String> {
    if !cef_embed_env_enabled() {
        let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Err("CEF embed disabled via ANYCODE_CEF_EMBED=0".into());
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
            let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
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
                            // 因此无 deadline 时也按空闲间隔 pump 一次。
                            guard = cv
                                .wait_timeout(guard, Duration::from_millis(IDLE_PUMP_INTERVAL_MS))
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
        let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
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
        let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
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
        let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
        return Ok(CefEmbedStatus {
            ready: false,
            remote_debugging_port: 0,
            url: None,
            title: None,
            tabs: Vec::new(),
            active_tab_id: None,
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
    run_on_main(&app, move || {
        anycode_browser_cef::close_tab(id)?;
        Ok(status_snapshot())
    })
}
