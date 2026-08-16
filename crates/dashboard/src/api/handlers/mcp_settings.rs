use super::*;
use crate::config_patch::{read_config_root, write_config_root};
use crate::mcp_config;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub struct McpGovernanceBody {
    pub strict: Option<bool>,
    pub max_calls_per_server: Option<usize>,
    pub allowed_tools: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct McpServersBody {
    pub servers: Option<Vec<serde_json::Value>>,
    pub governance: Option<McpGovernanceBody>,
}

fn read_mcp_governance(cfg: &Value) -> Value {
    let gov = cfg
        .get("mcp")
        .and_then(|m| m.get("governance"))
        .and_then(|v| v.as_object());
    json!({
        "strict": gov.and_then(|g| g.get("strict")).and_then(|v| v.as_bool()).unwrap_or(false),
        "max_calls_per_server": gov.and_then(|g| g.get("max_calls_per_server")).and_then(|v| v.as_u64()),
        "allowed_tools": gov
            .and_then(|g| g.get("allowed_tools"))
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        "env_override_note": "ANYCODE_MCP_STRICT / ANYCODE_MCP_ALLOWED_TOOLS / ANYCODE_MCP_MAX_CALLS_PER_SERVER override config at runtime",
    })
}

fn write_mcp_governance(cfg: &mut Value, body: &McpGovernanceBody) {
    let root = cfg.as_object_mut().expect("config root");
    let mcp = root.entry("mcp").or_insert_with(|| json!({}));
    let gov = mcp
        .as_object_mut()
        .expect("mcp object")
        .entry("governance")
        .or_insert_with(|| json!({}));
    let gov = gov.as_object_mut().expect("governance object");
    if let Some(v) = body.strict {
        gov.insert("strict".into(), json!(v));
    }
    if let Some(v) = body.max_calls_per_server {
        gov.insert("max_calls_per_server".into(), json!(v));
    }
    if let Some(tools) = &body.allowed_tools {
        gov.insert(
            "allowed_tools".into(),
            json!(tools
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()),
        );
    }
}

pub async fn get_mcp_servers() -> impl IntoResponse {
    let (_, cfg) = match read_config_root() {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let servers = mcp_config::read_mcp_servers(&cfg);
    Json(json!({
        "servers": mcp_config::redact_mcp_servers(&servers),
        "governance": read_mcp_governance(&cfg),
    }))
    .into_response()
}

pub async fn put_mcp_servers(Json(body): Json<McpServersBody>) -> impl IntoResponse {
    let (_, mut cfg) = match read_config_root() {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    if let Some(servers) = &body.servers {
        mcp_config::set_mcp_servers(&mut cfg, servers.clone());
    }
    if let Some(governance) = &body.governance {
        write_mcp_governance(&mut cfg, governance);
    }
    match write_config_root(&cfg) {
        Ok(path) => {
            let servers = mcp_config::read_mcp_servers(&cfg);
            Json(json!({
                "ok": true,
                "servers": mcp_config::redact_mcp_servers(&servers),
                "governance": read_mcp_governance(&cfg),
                "config_path": path.display().to_string(),
                "restart_hint": "Start a new conversation or restart the app for MCP changes to apply."
            }))
            .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}
