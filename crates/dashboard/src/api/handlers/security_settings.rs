use super::*;
use crate::config_patch::{read_config_root, write_config_root};
use anycode_config::{embedded_desktop_env, validate_permission_mode};
use serde_json::json;

#[derive(Serialize)]
pub struct SecuritySettingsView {
    pub sandbox_mode: bool,
    pub permission_mode: String,
    pub require_approval: bool,
    pub embedded_desktop: bool,
    pub bypass_allowed: bool,
    pub config_path: String,
}

#[derive(Deserialize)]
pub struct SecuritySettingsPatch {
    pub sandbox_mode: Option<bool>,
    pub permission_mode: Option<String>,
    pub require_approval: Option<bool>,
}

fn security_payload(
    cfg: &serde_json::Value,
    config_path: &std::path::Path,
) -> SecuritySettingsView {
    let security = cfg.get("security").and_then(|v| v.as_object());
    let sandbox_mode = security
        .and_then(|s| s.get("sandbox_mode"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let permission_mode = security
        .and_then(|s| s.get("permission_mode"))
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let require_approval = security
        .and_then(|s| s.get("require_approval"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let embedded = embedded_desktop_env();
    SecuritySettingsView {
        sandbox_mode,
        permission_mode,
        require_approval,
        embedded_desktop: embedded,
        bypass_allowed: !embedded,
        config_path: config_path.display().to_string(),
    }
}

pub async fn get_security_settings() -> impl IntoResponse {
    let (path, cfg) = match read_config_root() {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    Json(json!({ "security": security_payload(&cfg, &path) })).into_response()
}

pub async fn put_security_settings(Json(body): Json<SecuritySettingsPatch>) -> impl IntoResponse {
    let (path, mut cfg) = match read_config_root() {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    if let Some(mode) = body.permission_mode.as_deref() {
        if let Err(e) = validate_permission_mode(mode.trim()) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "invalid_permission_mode", "message": e.to_string() })),
            )
                .into_response();
        }
        if embedded_desktop_env() && mode.eq_ignore_ascii_case("bypass") {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "error": "bypass_not_allowed",
                    "message": "permission_mode=bypass is disabled in anyCode Desktop shipping builds"
                })),
            )
                .into_response();
        }
    }

    {
        let root = cfg.as_object_mut().expect("config root object");
        let security = root
            .entry("security")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .expect("security object");
        if let Some(v) = body.sandbox_mode {
            security.insert("sandbox_mode".into(), json!(v));
        }
        if let Some(mode) = body.permission_mode {
            security.insert("permission_mode".into(), json!(mode.trim()));
        }
        if let Some(v) = body.require_approval {
            security.insert("require_approval".into(), json!(v));
        }
    }

    match write_config_root(&cfg) {
        Ok(_) => Json(json!({
            "ok": true,
            "security": security_payload(&cfg, &path),
            "restart_hint": "Start a new conversation or restart anyCode for security policy changes to apply.",
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
