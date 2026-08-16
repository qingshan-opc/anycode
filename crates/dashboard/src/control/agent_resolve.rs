//! Resolve dashboard web-chat agent ids (UI "Auto" → config default or general-purpose).

pub const DEFAULT_WEB_CHAT_AGENT: &str = "general-purpose";

/// Empty / whitespace agent → `agents.defaults.run` from config, else [`DEFAULT_WEB_CHAT_AGENT`].
pub fn resolve_web_chat_agent(agent: Option<&str>) -> String {
    if let Some(a) = agent.map(str::trim).filter(|s| !s.is_empty()) {
        return a.to_string();
    }
    if let Ok((_, cfg)) = crate::config_patch::read_config_root() {
        if let Some(run) = cfg
            .get("agents")
            .and_then(|a| a.get("defaults"))
            .and_then(|d| d.get("run"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return run.to_string();
        }
    }
    DEFAULT_WEB_CHAT_AGENT.to_string()
}

/// Normalize delegate / nested-agent ids (`Explore`, `general-purpose`, …).
pub fn resolve_delegate_agent(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return DEFAULT_WEB_CHAT_AGENT.to_string();
    }
    let lower = t.to_ascii_lowercase();
    match lower.as_str() {
        "explore" | "explorer" => "explore".to_string(),
        "plan" | "planner" => "plan".to_string(),
        "general-purpose" | "general_purpose" | "builder" => "general-purpose".to_string(),
        "goal" | "goal-runner" => "goal".to_string(),
        "workspace-assistant" | "channel-ops" | "channel" => "workspace-assistant".to_string(),
        _ => anycode_agent::normalize_agent_id(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_agent_is_preserved() {
        assert_eq!(resolve_web_chat_agent(Some("code")), "code");
        assert_eq!(resolve_web_chat_agent(Some("  plan  ")), "plan");
    }

    #[test]
    fn empty_agent_falls_back_to_general_purpose() {
        assert_eq!(resolve_web_chat_agent(None), DEFAULT_WEB_CHAT_AGENT);
        assert_eq!(resolve_web_chat_agent(Some("")), DEFAULT_WEB_CHAT_AGENT);
        assert_eq!(resolve_web_chat_agent(Some("   ")), DEFAULT_WEB_CHAT_AGENT);
    }

    #[test]
    fn delegate_agent_normalizes_claude_casing() {
        assert_eq!(resolve_delegate_agent("Explore"), "explore");
        assert_eq!(resolve_delegate_agent("Plan"), "plan");
        assert_eq!(resolve_delegate_agent("general-purpose"), "general-purpose");
    }
}
