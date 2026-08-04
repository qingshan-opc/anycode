//! Non-macOS stub for CEF embed commands.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CefTabInfo {
    pub id: i32,
    pub url: String,
    pub title: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CefEmbedStatus {
    pub ready: bool,
    pub remote_debugging_port: u16,
    pub url: Option<String>,
    pub title: Option<String>,
    pub tabs: Vec<CefTabInfo>,
    pub active_tab_id: Option<i32>,
}

fn empty_status() -> CefEmbedStatus {
    CefEmbedStatus {
        ready: false,
        remote_debugging_port: 0,
        url: None,
        title: None,
        tabs: Vec::new(),
        active_tab_id: None,
    }
}

pub fn clear_stale_cdp_port_if_disabled() {
    let _ = std::env::remove_var("ANYCODE_CEF_CDP_PORT");
}

#[tauri::command]
pub fn cef_browser_status() -> Result<CefEmbedStatus, String> {
    Ok(empty_status())
}

#[tauri::command]
pub fn cef_browser_show(
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
    _url: String,
) -> Result<CefEmbedStatus, String> {
    Err("Embedded Chromium (CEF) is only supported on macOS".into())
}

#[tauri::command]
pub fn cef_browser_resize(_x: f64, _y: f64, _width: f64, _height: f64) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
pub fn cef_browser_hide() {}

#[tauri::command]
pub fn cef_browser_navigate(_url: String) -> Result<CefEmbedStatus, String> {
    Err("Embedded Chromium (CEF) is only supported on macOS".into())
}

#[tauri::command]
pub fn cef_browser_new_tab(_url: Option<String>) -> Result<CefEmbedStatus, String> {
    Err("Embedded Chromium (CEF) is only supported on macOS".into())
}

#[tauri::command]
pub fn cef_browser_select_tab(_id: i32) -> Result<CefEmbedStatus, String> {
    Err("Embedded Chromium (CEF) is only supported on macOS".into())
}

#[tauri::command]
pub fn cef_browser_close_tab(_id: i32) -> Result<CefEmbedStatus, String> {
    Err("Embedded Chromium (CEF) is only supported on macOS".into())
}
