//! Native Chromium CDP browser for anycode agents and workbench.

pub mod ax_tree;
pub mod chromium;
pub mod design_inspect;
pub mod error;
pub mod policy;
pub mod service;
pub mod session_actor;
pub mod snapshot;
pub mod types;
pub mod viewport;

pub use chromium::{chromium_doctor_message, resolve_chromium_executable};
pub use error::{BrowserError, BrowserResult};
pub use service::BrowserService;
pub use types::{
    BrowserDesignInspectState, BrowserHitTestResult, BrowserScreenshot, BrowserSessionInfo,
    BrowserSnapshot, BrowserState, BrowserTabInfo, BrowserViewport, LockHolder, ScreencastFrame,
    ViewportSpec,
};
pub use viewport::{resolve_viewport, resolve_viewport_spec, system_viewport};
