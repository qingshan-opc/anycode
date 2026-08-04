use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LockHolder {
    Idle,
    Agent,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub tab_id: String,
    pub url: String,
    pub title: String,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserState {
    pub url: String,
    pub title: String,
    pub lock: LockHolder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserViewport {
    pub width: u32,
    pub height: u32,
}

/// Requested layout size + host pixel density for sharp screencasts on Retina.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ViewportSpec {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_device_scale_factor")]
    pub device_scale_factor: f64,
}

fn default_device_scale_factor() -> f64 {
    1.0
}

impl ViewportSpec {
    pub fn new(width: u32, height: u32, device_scale_factor: f64) -> Self {
        Self {
            width,
            height,
            device_scale_factor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserScreenshot {
    pub image_base64: String,
    pub viewport: BrowserViewport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSnapshot {
    pub url: String,
    pub title: String,
    pub yaml: String,
}

#[derive(Debug, Clone)]
pub struct ScreencastFrame {
    pub image_base64: String,
    pub metadata: ScreencastMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScreencastMetadata {
    pub offset_top: i64,
    pub page_scale_factor: f64,
    pub device_width: i64,
    pub device_height: i64,
    pub scroll_offset_x: i64,
    pub scroll_offset_y: i64,
    pub timestamp: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserSessionInfo {
    pub session_id: String,
    pub project_id: String,
    pub conversation_id: Option<String>,
}

/// Result of clicking a point on the screencast / viewport (elementFromPoint).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHitTestResult {
    pub x: f64,
    pub y: f64,
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    pub classes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub css_selector: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xpath: Option<String>,
}

/// Polled state from the in-page Design inspect controller (CEF / live page).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BrowserDesignInspectState {
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<BrowserHitTestResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pick: Option<BrowserHitTestResult>,
}
