//! Agent implementation materialized from a declarative profile.

use crate::agent_profiles::ResolvedAgentProfile;
use crate::agents::agent_execute_delegated_to_runtime;
use anycode_core::prelude::*;
use async_trait::async_trait;

pub struct ProfileAgent {
    profile: ResolvedAgentProfile,
    agent_type: AgentType,
    model_config: ModelConfig,
}

impl ProfileAgent {
    pub fn new(profile: ResolvedAgentProfile, model_config: ModelConfig) -> Self {
        let agent_type = AgentType::new(&profile.id);
        Self {
            profile,
            agent_type,
            model_config,
        }
    }
}

#[async_trait]
impl Agent for ProfileAgent {
    fn agent_type(&self) -> &AgentType {
        &self.agent_type
    }

    fn description(&self) -> &str {
        &self.profile.description
    }

    fn tools(&self) -> Vec<ToolName> {
        self.profile.tools.clone()
    }

    fn system_prompt_overlay(&self) -> Option<&str> {
        self.profile.prompt_overlay.as_deref()
    }

    /// 文件式 agent（markdown 正文）语义：正文即完整系统提示词，替换默认段落；
    /// compose 的 replace 路径仍保留 config append + 任务 append（含 `SUBAGENT_SYSTEM_APPEND`）+ overlay。
    fn system_prompt_replaces_default_sections(&self) -> Option<&str> {
        self.profile
            .system_prompt
            .as_deref()
            .filter(|s| !s.trim().is_empty())
    }

    fn runtime_mode(&self) -> RuntimeMode {
        self.profile.runtime_mode
    }

    async fn execute(&mut self, _task: Task) -> Result<TaskResult, CoreError> {
        let _ = &self.model_config;
        agent_execute_delegated_to_runtime()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::RuntimeMode;

    fn profile(system_prompt: Option<String>) -> ResolvedAgentProfile {
        ResolvedAgentProfile {
            id: "file-agent".into(),
            extends: "explore".into(),
            description: "File-defined agent".into(),
            tools: vec!["FileRead".into()],
            skills_allowlist: None,
            prompt_overlay: None,
            system_prompt,
            runtime_mode: RuntimeMode::Explore,
        }
    }

    #[test]
    fn replaces_default_sections_iff_system_prompt_set() {
        let with = ProfileAgent::new(
            profile(Some("You are a reviewer.".into())),
            ModelConfig::default(),
        );
        assert_eq!(
            with.system_prompt_replaces_default_sections(),
            Some("You are a reviewer.")
        );

        let without = ProfileAgent::new(profile(None), ModelConfig::default());
        assert!(without.system_prompt_replaces_default_sections().is_none());
    }
}
