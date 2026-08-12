//! Claude Code `model: sonnet | opus | haiku` → concrete `ModelConfig` for nested runs.

use anycode_core::{LLMProvider, ModelConfig};

/// Apply a nested-agent model hint on top of the parent session config.
///
/// Recognizes **`inherit`** (reuse the parent model verbatim — Claude Code `model: "inherit"`),
/// **`sonnet`**, **`opus`**, **`haiku`** (case-insensitive). For **`Anthropic`**
/// provider, uses Messages API model ids; for OpenAI-compatible gateways, uses
/// **`anthropic/<id>`** qualified refs. Any other non-empty string sets **`model`** verbatim.
/// Map a Claude Code family shorthand (`sonnet` / `opus` / `haiku`, case-insensitive)
/// to its concrete Anthropic model id. Single source of truth shared by the per-call
/// nested hint chain and declarative agent frontmatter `model:` mapping.
#[must_use]
pub fn concrete_model_for_family_hint(hint: &str) -> Option<&'static str> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "sonnet" => Some("claude-sonnet-4-5-20250929"),
        "opus" => Some("claude-opus-4-5-20250929"),
        "haiku" => Some("claude-haiku-4-5-20251001"),
        _ => None,
    }
}

pub fn resolve_nested_model_hint(base: &ModelConfig, hint: &str) -> ModelConfig {
    let mut out = base.clone();
    let h = hint.trim();
    if h.is_empty() {
        return out;
    }
    let lower = h.to_ascii_lowercase();
    if lower == "inherit" {
        // Claude Code `model: "inherit"`: reuse the parent's model configuration unchanged.
        return out;
    }
    let family = concrete_model_for_family_hint(&lower);
    if let Some(mid) = family {
        match &base.provider {
            LLMProvider::Anthropic => {
                out.model = mid.to_string();
            }
            LLMProvider::OpenAI | LLMProvider::Local | LLMProvider::Custom(_) => {
                out.model = format!("anthropic/{mid}");
            }
        }
        return out;
    }
    out.model = h.to_string();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::LLMProvider;

    #[test]
    fn family_hint_mapping_is_case_insensitive_and_trimmed() {
        assert_eq!(
            concrete_model_for_family_hint("  SONNET "),
            Some("claude-sonnet-4-5-20250929")
        );
        assert_eq!(
            concrete_model_for_family_hint("opus"),
            Some("claude-opus-4-5-20250929")
        );
        assert_eq!(
            concrete_model_for_family_hint("haiku"),
            Some("claude-haiku-4-5-20251001")
        );
        assert_eq!(concrete_model_for_family_hint("inherit"), None);
        assert_eq!(concrete_model_for_family_hint("gpt-4o"), None);
    }

    #[test]
    fn anthropic_sonnet_hint() {
        let base = ModelConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-opus-4-5-20250929".into(),
            base_url: None,
            temperature: None,
            max_tokens: None,
            api_key: None,
            ..Default::default()
        };
        let o = resolve_nested_model_hint(&base, "sonnet");
        assert_eq!(o.model, "claude-sonnet-4-5-20250929");
    }

    #[test]
    fn openai_compat_qualified() {
        let base = ModelConfig {
            provider: LLMProvider::OpenAI,
            model: "gpt-4o".into(),
            base_url: Some("https://example/v1".into()),
            temperature: None,
            max_tokens: None,
            api_key: None,
            ..Default::default()
        };
        let o = resolve_nested_model_hint(&base, "haiku");
        assert_eq!(o.model, "anthropic/claude-haiku-4-5-20251001");
    }

    #[test]
    fn inherit_reuses_parent_model_verbatim() {
        let base = ModelConfig {
            provider: LLMProvider::Anthropic,
            model: "claude-opus-4-5-20250929".into(),
            base_url: Some("https://api.example/v1".into()),
            temperature: Some(0.2),
            max_tokens: Some(8192),
            api_key: None,
            ..Default::default()
        };
        let o = resolve_nested_model_hint(&base, "inherit");
        assert_eq!(o.model, base.model);
        assert_eq!(o.provider, LLMProvider::Anthropic);
        assert_eq!(o.base_url, base.base_url);
        assert_eq!(o.temperature, base.temperature);
        assert_eq!(o.max_tokens, base.max_tokens);
    }

    #[test]
    fn inherit_is_case_insensitive_and_trimmed() {
        let base = ModelConfig {
            provider: LLMProvider::OpenAI,
            model: "gpt-4o".into(),
            base_url: None,
            temperature: None,
            max_tokens: None,
            api_key: None,
            ..Default::default()
        };
        let o = resolve_nested_model_hint(&base, "  INHERIT  ");
        assert_eq!(o.model, "gpt-4o");
        assert_eq!(o.model, base.model);
    }
}
