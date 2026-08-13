//! Helpers for `config.json` → `mcp.servers` (stdio / HTTP MCP declarations).

use serde_json::{json, Value};

pub const CONFIG_KEY: &str = "mcp";
const SENSITIVE_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "authorization",
    "bearer",
    "bearer_token",
    "client_secret",
    "oauth_credentials_path",
    "password",
    "refresh_token",
    "secret",
    "token",
];

pub fn read_mcp_servers(cfg: &Value) -> Vec<Value> {
    cfg.get(CONFIG_KEY)
        .and_then(|m| m.get("servers"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

pub fn set_mcp_servers(cfg: &mut Value, servers: Vec<Value>) {
    let root = cfg.as_object_mut().expect("config root must be object");
    let mcp = root
        .entry(CONFIG_KEY)
        .or_insert_with(|| json!({ "browser": { "enabled": false }, "servers": [] }));
    if let Some(obj) = mcp.as_object_mut() {
        obj.insert("servers".into(), Value::Array(servers));
    }
}

pub fn redact_mcp_servers(servers: &[Value]) -> Vec<Value> {
    servers.iter().map(redact_value).collect()
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| {
                    if is_sensitive_key(k) {
                        (
                            k.clone(),
                            json!({ "configured": !is_empty_secret(v), "preview": "***" }),
                        )
                    } else {
                        (k.clone(), redact_value(v))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(redact_value).collect()),
        _ => value.clone(),
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect::<String>();
    SENSITIVE_KEYS.iter().any(|s| {
        let sensitive = s.replace(['-', '_'], "");
        normalized == sensitive || normalized.ends_with(&sensitive)
    })
}

fn is_empty_secret(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        _ => false,
    }
}

/// Handoff-bundle redaction: keep the server entry structurally importable
/// but blank every sensitive value — the recipient re-enters secrets via the
/// MCP settings UI. Unlike [`redact_mcp_servers`] (UI display shape), the
/// output stays a valid MCP server declaration.
pub fn handoff_redact_mcp_servers(servers: &[Value]) -> Vec<Value> {
    servers.iter().map(handoff_redact_value).collect()
}

fn handoff_redact_value(value: &Value) -> Value {
    match value {
        Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| {
                    if is_sensitive_key(k) && !is_empty_secret(v) {
                        (k.clone(), Value::String(String::new()))
                    } else {
                        (k.clone(), handoff_redact_value(v))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.iter().map(handoff_redact_value).collect()),
        _ => value.clone(),
    }
}

/// How many sensitive values `handoff_redact_mcp_servers` blanked between an
/// original entry and its redacted twin (for manifest metadata).
pub fn count_redacted_secrets(original: &Value, redacted: &Value) -> usize {
    fn walk(orig: &Value, red: &Value, acc: &mut usize) {
        match (orig, red) {
            (Value::Object(a), Value::Object(b)) => {
                for (k, v) in a {
                    if is_sensitive_key(k) && !is_empty_secret(v) {
                        let blanked = matches!(b.get(k), Some(Value::String(s)) if s.is_empty());
                        if blanked {
                            *acc += 1;
                            continue;
                        }
                    }
                    if let Some(rv) = b.get(k) {
                        walk(v, rv, acc);
                    }
                }
            }
            (Value::Array(a), Value::Array(b)) => {
                for (v, rv) in a.iter().zip(b.iter()) {
                    walk(v, rv, acc);
                }
            }
            _ => {}
        }
    }
    let mut acc = 0;
    walk(original, redacted, &mut acc);
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_servers() {
        let mut cfg = json!({ "provider": "z.ai" });
        assert!(read_mcp_servers(&cfg).is_empty());
        set_mcp_servers(&mut cfg, vec![json!({"slug":"fs","command":"echo fs"})]);
        let servers = read_mcp_servers(&cfg);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["slug"], "fs");
    }

    #[test]
    fn redacts_sensitive_server_fields_recursively() {
        let servers = vec![json!({
            "slug": "remote",
            "bearer_token": "secret-token",
            "headers": {
                "Authorization": "Bearer secret-token",
                "X-Workspace": "safe"
            },
            "nested": [{ "oauth_credentials_path": "/tmp/creds.json" }]
        })];

        let redacted = redact_mcp_servers(&servers);
        assert_eq!(redacted[0]["slug"], "remote");
        assert_eq!(redacted[0]["headers"]["X-Workspace"], "safe");
        assert_eq!(redacted[0]["bearer_token"]["configured"], true);
        assert_eq!(redacted[0]["headers"]["Authorization"]["preview"], "***");
        assert_eq!(
            redacted[0]["nested"][0]["oauth_credentials_path"]["configured"],
            true
        );
    }

    #[test]
    fn handoff_redaction_blanks_secrets_but_keeps_shape() {
        let servers = vec![json!({
            "slug": "remote",
            "type": "http",
            "url": "https://mcp.example.com/sse",
            "headers": { "Authorization": "Bearer secret-token", "X-Team": "core" },
            "env": { "MCP_API_KEY": "abc123", "PLAIN": "ok" }
        })];
        let redacted = handoff_redact_mcp_servers(&servers);
        // Structure survives — the entry is still an importable declaration.
        assert_eq!(redacted[0]["slug"], "remote");
        assert_eq!(redacted[0]["type"], "http");
        assert_eq!(redacted[0]["url"], "https://mcp.example.com/sse");
        assert_eq!(redacted[0]["headers"]["X-Team"], "core");
        assert_eq!(redacted[0]["env"]["PLAIN"], "ok");
        // Secrets are blanked, not display-shaped.
        assert_eq!(redacted[0]["headers"]["Authorization"], "");
        assert_eq!(redacted[0]["env"]["MCP_API_KEY"], "");
        // No secret marker values leak into the serialized form.
        let text = serde_json::to_string(&redacted).unwrap();
        assert!(!text.contains("abc123"));
        assert!(!text.contains("secret-token"));
        assert_eq!(count_redacted_secrets(&servers[0], &redacted[0]), 2);
    }
}
