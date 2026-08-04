//! Embedded Chromium (CEF) for the Workbench browser panel.
//!
//! Desktop builds enable the `host` feature. The CEF helper subprocess uses `helper`.

#![cfg_attr(not(any(feature = "host", feature = "helper")), allow(dead_code))]

use serde::{Deserialize, Serialize};

/// Panel rect in parent window **content-view** coordinates (points, top-left origin from UI).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct EmbedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One CEF browser tab surfaced to the Workbench UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub id: i32,
    pub url: String,
    pub title: String,
    pub active: bool,
}

#[cfg(all(target_os = "macos", feature = "host"))]
mod host;

#[cfg(all(target_os = "macos", feature = "host"))]
pub use host::{
    close_tab, current_url, do_message_loop_work, ensure_initialized, hide, list_tabs, navigate,
    new_tab, remote_debugging_port, resize, select_tab, set_schedule_pump_callback, show_in_parent,
    shutdown_cef, title,
};

#[cfg(not(all(target_os = "macos", feature = "host")))]
mod stub {
    use super::EmbedRect;

    pub fn ensure_initialized(
        _helper: &std::path::Path,
        _framework: &std::path::Path,
    ) -> Result<(), String> {
        Err("CEF embed is only available on macOS desktop builds".into())
    }
    pub fn show_in_parent(
        _ns_window: *mut std::ffi::c_void,
        _rect: EmbedRect,
        _url: &str,
    ) -> Result<(), String> {
        Err("CEF embed is only available on macOS desktop builds".into())
    }
    pub fn resize(_rect: EmbedRect) -> Result<(), String> {
        Err("CEF embed unavailable".into())
    }
    pub fn hide() {}
    pub fn navigate(_url: &str) -> Result<(), String> {
        Err("CEF embed unavailable".into())
    }
    pub fn new_tab(_url: &str) -> Result<(), String> {
        Err("CEF embed unavailable".into())
    }
    pub fn select_tab(_id: i32) -> Result<(), String> {
        Err("CEF embed unavailable".into())
    }
    pub fn close_tab(_id: i32) -> Result<(), String> {
        Err("CEF embed unavailable".into())
    }
    pub fn list_tabs() -> Vec<super::TabInfo> {
        Vec::new()
    }
    pub fn current_url() -> Option<String> {
        None
    }
    pub fn title() -> Option<String> {
        None
    }
    pub fn remote_debugging_port() -> u16 {
        0
    }
    pub fn do_message_loop_work() {}
    pub fn set_schedule_pump_callback(_cb: Box<dyn Fn(i64) + Send>) {}
    pub fn shutdown_cef() {}
}

#[cfg(not(all(target_os = "macos", feature = "host")))]
pub use stub::*;
