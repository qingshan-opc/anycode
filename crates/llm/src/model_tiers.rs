//! General model-lifecycle tiers: `current | legacy | deprecated | removed`.
//!
//! DeepSeek's per-catalog tier (see `deepseek_catalog.rs`) predates this file;
//! this registry generalizes the idea to every provider. Rules are matched
//! case-insensitively, exact id first, then prefix. `deprecated` models are
//! hidden from default pickers but keep working for users who already selected
//! them; `removed` models are rejected with a guided migration message by the
//! consumer (see e.g. the managed-local removal path).

/// Lifecycle tier for a model id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelTier {
    Current,
    /// Superseded alias kept for backward compatibility (e.g. provider-side
    /// redirects); still fully functional.
    Legacy,
    /// Outdated — hidden from default pickers, usable if already selected,
    /// shown with a warning badge.
    Deprecated,
    /// No longer available — consumers must hard-error with a guided
    /// migration to `replacement`.
    Removed,
}

impl ModelTier {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Legacy => "legacy",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
        }
    }
}

/// One deprecation rule. `model_prefix` matches the model id as an exact id or
/// a prefix (`gpt-4-` covers `gpt-4-turbo` but not `gpt-4o`).
pub struct ModelTierRule {
    /// Normalized provider id, or `"*"` to match any provider.
    pub provider: &'static str,
    pub model_prefix: &'static str,
    pub tier: ModelTier,
    /// Suggested migration target shown in the guided-migration message.
    pub replacement: Option<&'static str>,
    pub note: &'static str,
}

/// Audited deprecation list (2026-08). Conservative: only clearly superseded
/// ids; regional/provider-specific ids stay untouched.
pub const MODEL_TIER_RULES: &[ModelTierRule] = &[
    ModelTierRule {
        provider: "deepseek",
        model_prefix: "deepseek-chat",
        tier: ModelTier::Deprecated,
        replacement: Some("deepseek-v4-flash"),
        note: "兼容别名,2026-07-24 弃用;映射 V4 Flash 非思考模式",
    },
    ModelTierRule {
        provider: "deepseek",
        model_prefix: "deepseek-reasoner",
        tier: ModelTier::Deprecated,
        replacement: Some("deepseek-v4-flash"),
        note: "兼容别名,2026-07-24 弃用;映射 V4 Flash 思考模式",
    },
    ModelTierRule {
        provider: "anthropic",
        model_prefix: "claude-instant",
        tier: ModelTier::Deprecated,
        replacement: Some("claude-haiku-4-5"),
        note: "Claude 1.x instant 系列已退役",
    },
    ModelTierRule {
        provider: "anthropic",
        model_prefix: "claude-2",
        tier: ModelTier::Deprecated,
        replacement: Some("claude-sonnet-4-5"),
        note: "Claude 2.x 系列已退役",
    },
    ModelTierRule {
        provider: "anthropic",
        model_prefix: "claude-3",
        tier: ModelTier::Deprecated,
        replacement: Some("claude-sonnet-4-5"),
        note: "Claude 3.x 系列已被 4.x/5 取代",
    },
    ModelTierRule {
        provider: "openai",
        model_prefix: "gpt-3.5",
        tier: ModelTier::Deprecated,
        replacement: Some("gpt-4o-mini"),
        note: "GPT-3.5 系列已退役",
    },
    ModelTierRule {
        provider: "openai",
        model_prefix: "gpt-4-",
        tier: ModelTier::Deprecated,
        replacement: Some("gpt-4o"),
        note: "GPT-4 原版/turbo 已被 gpt-4o 取代",
    },
    ModelTierRule {
        provider: "openai",
        model_prefix: "o1-preview",
        tier: ModelTier::Deprecated,
        replacement: Some("o3"),
        note: "o1-preview 已被 o 系列正式版取代",
    },
    ModelTierRule {
        provider: "openai",
        model_prefix: "o1-mini",
        tier: ModelTier::Deprecated,
        replacement: Some("o4-mini"),
        note: "o1-mini 已被 o4-mini 取代",
    },
    ModelTierRule {
        provider: "google",
        model_prefix: "gemini-1.0",
        tier: ModelTier::Deprecated,
        replacement: Some("gemini-2.5-flash"),
        note: "Gemini 1.0 系列已退役",
    },
    ModelTierRule {
        provider: "google",
        model_prefix: "gemini-1.5",
        tier: ModelTier::Deprecated,
        replacement: Some("gemini-2.5-flash"),
        note: "Gemini 1.5 系列已被 2.x 取代",
    },
    // Local managed MiniCPM launcher was removed from the product (roadmap
    // P1.7). Catch both the preset id form (`managed-minicpm5-1b`, any
    // provider) and the raw SGLang model form.
    ModelTierRule {
        provider: "*",
        model_prefix: "managed-minicpm",
        tier: ModelTier::Removed,
        replacement: Some("ollama qwen3"),
        note: "本地托管 MiniCPM5-1B 已移除;请改用 Ollama 本地模型或云端精选套件",
    },
    ModelTierRule {
        provider: "sglang",
        model_prefix: "minicpm",
        tier: ModelTier::Removed,
        replacement: Some("ollama qwen3"),
        note: "本地托管 MiniCPM5-1B 已移除;请改用 Ollama 本地模型或云端精选套件",
    },
];

/// Case-insensitive provider alias groups for rule matching: a rule written
/// for `google` also applies to `gemini`, etc.
fn provider_matches(rule_provider: &str, provider: &str) -> bool {
    if rule_provider == "*" {
        return true;
    }
    let p = provider.trim().to_ascii_lowercase();
    let aliases: &[&str] = match rule_provider {
        "google" => &["google", "gemini"],
        "anthropic" => &["anthropic", "bedrock", "amazon_bedrock"],
        other => return p == other,
    };
    aliases.contains(&p.as_str())
}

/// First matching rule for (provider, model), if any.
#[must_use]
pub fn rule_for_model(provider: &str, model: &str) -> Option<&'static ModelTierRule> {
    let m = model.trim().to_ascii_lowercase();
    if m.is_empty() {
        return None;
    }
    MODEL_TIER_RULES
        .iter()
        .find(|r| provider_matches(r.provider, provider) && m.starts_with(r.model_prefix))
}

/// Tier for (provider, model); defaults to `Current` when no rule matches.
#[must_use]
pub fn tier_for_model(provider: &str, model: &str) -> ModelTier {
    rule_for_model(provider, model)
        .map(|r| r.tier)
        .unwrap_or(ModelTier::Current)
}

/// Curated default suite pinned to the top of pickers: the lean high-quality
/// set anyCode recommends (see roadmap wish #6).
/// 本地客户端精简：只推荐 DeepSeek V4 两个模型。
pub const CURATED_MODEL_SUITE: &[(&str, &str)] = &[
    ("deepseek", "deepseek-v4-flash"),
    ("deepseek", "deepseek-v4-pro"),
];

/// Whether (provider, model) belongs to the curated default suite. Model ids
/// move fast — matching is prefix-based so `claude-sonnet-4-5-20250929` counts.
#[must_use]
pub fn is_curated_model(provider: &str, model: &str) -> bool {
    let m = model.trim().to_ascii_lowercase();
    CURATED_MODEL_SUITE
        .iter()
        .any(|(p, prefix)| provider_matches(p, provider) && m.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_legacy_aliases_are_deprecated() {
        assert_eq!(
            tier_for_model("deepseek", "deepseek-chat"),
            ModelTier::Deprecated
        );
        assert_eq!(
            tier_for_model("deepseek", "deepseek-reasoner"),
            ModelTier::Deprecated
        );
        assert_eq!(
            tier_for_model("deepseek", "deepseek-v4-flash"),
            ModelTier::Current
        );
    }

    #[test]
    fn prefix_matching_avoids_false_positives() {
        // gpt-4- prefix must not catch gpt-4o.
        assert_eq!(tier_for_model("openai", "gpt-4o"), ModelTier::Current);
        assert_eq!(tier_for_model("openai", "gpt-4o-mini"), ModelTier::Current);
        assert_eq!(
            tier_for_model("openai", "gpt-4-turbo"),
            ModelTier::Deprecated
        );
        assert_eq!(
            tier_for_model("openai", "gpt-3.5-turbo"),
            ModelTier::Deprecated
        );
    }

    #[test]
    fn provider_aliases_match() {
        assert_eq!(
            tier_for_model("gemini", "gemini-1.5-pro"),
            ModelTier::Deprecated
        );
        assert_eq!(
            tier_for_model("bedrock", "claude-3-sonnet"),
            ModelTier::Deprecated
        );
    }

    #[test]
    fn curated_suite_prefix_matching() {
        assert!(is_curated_model("deepseek", "deepseek-v4-flash"));
        assert!(is_curated_model("deepseek", "deepseek-v4-pro"));
        assert!(!is_curated_model("anthropic", "claude-sonnet-4-5-20250929"));
        assert!(!is_curated_model("openai", "gpt-3.5-turbo"));
    }

    #[test]
    fn removed_tier_stringify() {
        assert_eq!(ModelTier::Removed.as_str(), "removed");
        assert_eq!(ModelTier::Legacy.as_str(), "legacy");
    }

    #[test]
    fn managed_minicpm_is_removed_with_guidance() {
        for (provider, model) in [
            ("sglang", "minicpm5-1b"),
            ("openai", "managed-minicpm5-1b"),
            ("local", "managed-minicpm5-1b"),
        ] {
            assert_eq!(
                tier_for_model(provider, model),
                ModelTier::Removed,
                "{provider}/{model}"
            );
            let rule = rule_for_model(provider, model).unwrap();
            assert!(rule.replacement.is_some());
        }
        // The curated cloud suite must stay Current.
        assert_eq!(
            tier_for_model("deepseek", "deepseek-v4-flash"),
            ModelTier::Current
        );
    }
}
