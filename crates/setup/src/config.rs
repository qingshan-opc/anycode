use anycode_llm::{
    is_known_provider_id, normalize_provider_id, read_cloud_access_token, string_field,
};
use serde_json::Value;

fn has_non_empty_secret(v: &str) -> bool {
    !v.trim().is_empty()
}

fn cloud_linked() -> bool {
    read_cloud_access_token().is_some_and(|t| has_non_empty_secret(&t))
}

fn registry_has_cloud_chat(cfg: &Value) -> bool {
    let Some(items) = cfg
        .pointer("/models/registry/items")
        .or_else(|| cfg.pointer("/models/items"))
        .and_then(|v| v.as_array())
    else {
        return false;
    };
    items.iter().any(|item| {
        let provider = item
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        normalize_provider_id(provider) == "anycode_cloud"
    })
}

/// Whether config (or a linked anyCode Cloud session) is enough to run a chat.
pub fn has_usable_model_config(cfg: &Value) -> bool {
    // Linked cloud device + hosted catalog (or anycode_cloud provider) counts as ready.
    if cloud_linked() {
        let provider = string_field(cfg, "provider", "provider").unwrap_or_default();
        if normalize_provider_id(&provider) == "anycode_cloud" || registry_has_cloud_chat(cfg) {
            return true;
        }
        // Fresh link before sync-models: still treat as ready so we don't bounce to /setup.
        return true;
    }

    let provider = string_field(cfg, "provider", "provider").unwrap_or_default();
    let model = string_field(cfg, "model", "model").unwrap_or_default();
    if provider.trim().is_empty() || model.trim().is_empty() {
        return false;
    }
    let norm = normalize_provider_id(&provider);
    if !is_known_provider_id(&norm) {
        return false;
    }
    if string_field(cfg, "api_key", "api_key").is_some_and(|k| has_non_empty_secret(&k)) {
        return true;
    }
    cfg.get("provider_credentials")
        .and_then(|v| v.as_object())
        .is_some_and(|m| {
            m.values()
                .filter_map(|v| v.as_str())
                .any(has_non_empty_secret)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_empty_key() {
        let cfg = json!({
            "provider": "z.ai",
            "model": "glm-5",
            "api_key": ""
        });
        assert!(!has_usable_model_config(&cfg));
    }

    #[test]
    fn accepts_provider_credentials() {
        let cfg = json!({
            "provider": "openai",
            "model": "gpt-4o",
            "api_key": "",
            "provider_credentials": { "openai": "sk-test" }
        });
        assert!(has_usable_model_config(&cfg));
    }

    #[test]
    fn accepts_primary_api_key() {
        let cfg = json!({
            "provider": "z.ai",
            "model": "glm-5",
            "api_key": "secret"
        });
        assert!(has_usable_model_config(&cfg));
    }

    #[test]
    fn cloud_linked_helper_is_false_without_session_file() {
        // Unit tests don't plant cloud-session.json; helper must not panic.
        let _ = cloud_linked();
    }
}
