//! Quick-auth preset metadata shared by CLI setup and Dashboard wizard.

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy)]
pub struct QuickAuthChoice {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    pub plan: &'static str,
    pub default_model: &'static str,
    pub base_url: &'static str,
    pub key_envs: &'static [&'static str],
    /// Device-link auth (no API key prompt).
    pub device_auth: bool,
}

/// Single source of truth for quick-auth presets (CLI + Dashboard).
/// 本地客户端精简：只保留 anyCode Cloud 托管与 DeepSeek 官方 API Key 两条路。
pub const QUICK_AUTH_CHOICES: &[QuickAuthChoice] = &[
    QuickAuthChoice {
        id: "anycode-cloud",
        label: "anyCode Cloud (DeepSeek V4 hosted)",
        provider: "anycode_cloud",
        plan: "cloud",
        default_model: "auto",
        base_url: "",
        key_envs: &[],
        device_auth: true,
    },
    QuickAuthChoice {
        id: "deepseek-api-key",
        label: "DeepSeek API Key",
        provider: "deepseek",
        plan: "general",
        default_model: "deepseek-v4-pro",
        base_url: "https://api.deepseek.com/chat/completions",
        key_envs: &["DEEPSEEK_API_KEY"],
        device_auth: false,
    },
];

pub fn quick_auth_presets() -> Value {
    let presets: Vec<Value> = QUICK_AUTH_CHOICES
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "label": c.label,
                "provider": c.provider,
                "plan": c.plan,
                "default_model": c.default_model,
                "base_url": c.base_url,
                "key_envs": c.key_envs,
                "device_auth": c.device_auth,
            })
        })
        .collect();
    Value::Array(presets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_choices_count() {
        let value = quick_auth_presets();
        let presets = value.as_array().expect("presets array");
        assert_eq!(presets.len(), QUICK_AUTH_CHOICES.len());
    }
}
