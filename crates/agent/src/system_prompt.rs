//! 系统提示多段合成（override / append / 默认段与记忆的优先级）。

use crate::model_instructions::ModelInstructionsConfig;
use crate::prompt_assembler::PromptAssembler;
use anycode_core::Agent;
use std::collections::HashMap;
use std::path::Path;

/// 运行时系统提示配置（通常来自 `config.json` + 解析后的 `@path` 文件内容）。
#[derive(Debug, Clone, Default)]
pub struct RuntimePromptConfig {
    /// 若非空：整段 system 仅此内容（不注入默认段、记忆、append）。
    pub system_prompt_override: Option<String>,
    /// 接在合成 system 末尾（在 per-task append 之前）。
    pub system_prompt_append: Option<String>,
    /// Injected after the tool list when not using override (from discovered `SKILL.md` skills).
    pub skills_section: Option<String>,
    /// 按 `agent_type` 覆盖 skills 段（仅列出配置允许的技能 id）；未命中则回退 `skills_section`。
    pub skills_section_by_agent: HashMap<String, String>,
    pub workspace_section: Option<String>,
    pub channel_section: Option<String>,
    pub workflow_section: Option<String>,
    pub goal_section: Option<String>,
    #[allow(clippy::vec_box)]
    pub prompt_fragments: Vec<String>,
    /// Configuration for model instructions file discovery (AGENTS.md).
    pub model_instructions: ModelInstructionsConfig,
    /// Path to a model instructions file (e.g., `AGENTS.md`) whose content is injected into the system prompt.
    /// Supports absolute paths or paths relative to the working directory.
    pub model_instructions_file: Option<std::path::PathBuf>,
    /// Cached content of the model instructions file (resolved at runtime).
    pub model_instructions_content: Option<String>,
}

impl RuntimePromptConfig {
    /// Resolve and load the model instructions file content.
    /// If `model_instructions_file` is set, reads the file and stores content in `model_instructions_content`.
    /// If the file path is relative, it is resolved relative to `working_dir`.
    /// Returns `Ok(())` if successful or if no file is configured.
    /// Returns `Err` only on I/O errors when the file is configured but cannot be read.
    pub fn resolve_model_instructions_file(
        &mut self,
        working_dir: &Path,
    ) -> Result<(), std::io::Error> {
        let Some(ref path) = self.model_instructions_file else {
            return Ok(());
        };

        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            working_dir.join(path)
        };

        if !resolved.is_file() {
            tracing::debug!(
                target: "anycode_agent",
                path = %resolved.display(),
                "model_instructions_file not found, skipping"
            );
            return Ok(());
        }

        match std::fs::read_to_string(&resolved) {
            Ok(content) => {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    tracing::info!(
                        target: "anycode_agent",
                        path = %resolved.display(),
                        len = trimmed.len(),
                        "loaded model instructions file"
                    );
                    self.model_instructions_content = Some(content);
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    target: "anycode_agent",
                    path = %resolved.display(),
                    error = %e,
                    "failed to read model_instructions_file"
                );
                Err(e)
            }
        }
    }

    /// Create a new RuntimePromptConfig with model instructions file set.
    pub fn with_model_instructions_file(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.model_instructions_file = path;
        self
    }
}

pub(crate) fn default_stack_sections(
    agent: &dyn Agent,
    cwd: &str,
    skills_section: Option<&str>,
) -> Vec<String> {
    let tools = agent.tools();
    let include_browser = tools.iter().any(|t| t.starts_with("Browser"));
    let mut parts = crate::prompt_catalog::default_stack_sections(cwd, &tools, include_browser);
    if let Some(sk) = skills_section {
        let t = sk.trim();
        if !t.is_empty() {
            parts.push(t.to_string());
        }
    }
    parts.push("<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->".to_string());
    if crate::prompt_catalog::prefix_stable_mode() {
        // P1.5: volatile content (date, reply language) rides the trailing
        // dynamic block so the static prefix above is byte-stable across
        // turns, sessions, languages, and day boundaries.
        parts.extend(crate::prompt_catalog::dynamic_tail_sections());
    }
    parts
}

pub(crate) fn compose_default_sections(
    agent: &dyn Agent,
    cwd: &str,
    skills_section: Option<&str>,
) -> String {
    let mut parts = default_stack_sections(agent, cwd, skills_section);
    let desc = format!("# Custom Agent Instructions\n\n{}", agent.description());
    if crate::prompt_catalog::prefix_stable_mode() {
        // Keep the (static) agent description ahead of the dynamic tail.
        match parts
            .iter()
            .position(|p| p == "<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->")
        {
            Some(pos) => parts.insert(pos, desc),
            None => parts.push(desc),
        }
    } else {
        parts.push(desc);
    }
    parts.join("\n\n")
}

/// 合成最终一条 system 文本（段之间双换行拼接）。
pub fn compose_effective_system_prompt(
    config: &RuntimePromptConfig,
    agent: &dyn Agent,
    cwd: &str,
    task_append: Option<&str>,
) -> String {
    let skip_plugins = config
        .system_prompt_override
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    if let Some(rep) = agent.system_prompt_replaces_default_sections() {
        let t = rep.trim();
        if !t.is_empty() {
            let mut out = vec![t.to_string()];
            if let Some(a) = config.system_prompt_append.as_deref() {
                if !a.trim().is_empty() {
                    out.push(a.trim().to_string());
                }
            }
            if let Some(a) = task_append {
                if !a.trim().is_empty() {
                    out.push(a.trim().to_string());
                }
            }
            if let Some(overlay) = agent.system_prompt_overlay() {
                if !overlay.trim().is_empty() {
                    out.push(overlay.trim().to_string());
                }
            }
            return if skip_plugins {
                out.join("\n\n")
            } else {
                append_plugin_overlays(out.join("\n\n"), Some(Path::new(cwd)))
            };
        }
    }
    let composed = PromptAssembler {
        config,
        agent,
        cwd,
        task_append,
    }
    .compose();
    if skip_plugins {
        composed
    } else {
        append_plugin_overlays(composed, Some(Path::new(cwd)))
    }
}

fn append_plugin_overlays(mut prompt: String, workspace: Option<&Path>) -> String {
    for plugin in crate::plugins::load_plugins(workspace)
        .into_iter()
        .filter(|p| p.enabled)
    {
        if let Some(overlay) = plugin
            .system_prompt_overlay
            .as_deref()
            .filter(|s| !s.trim().is_empty())
        {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(overlay.trim());
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::{AgentType, CoreError, Task, TaskResult, ToolName};
    use async_trait::async_trait;
    use std::collections::HashMap;

    struct StubAgent {
        agent_type: AgentType,
        desc: &'static str,
        replace: Option<&'static str>,
        tools: Vec<ToolName>,
    }

    #[async_trait]
    impl Agent for StubAgent {
        fn agent_type(&self) -> &AgentType {
            &self.agent_type
        }

        fn description(&self) -> &str {
            self.desc
        }

        fn tools(&self) -> Vec<ToolName> {
            self.tools.clone()
        }

        async fn execute(&mut self, _task: Task) -> Result<TaskResult, CoreError> {
            unreachable!()
        }

        fn system_prompt_replaces_default_sections(&self) -> Option<&str> {
            self.replace
        }
    }

    fn stub(tools: Vec<ToolName>) -> StubAgent {
        StubAgent {
            agent_type: AgentType::new("stub"),
            desc: "agent-desc",
            replace: None,
            tools,
        }
    }

    #[test]
    fn override_is_only_body() {
        let cfg = RuntimePromptConfig {
            system_prompt_override: Some("OVERRIDE_ONLY".to_string()),
            system_prompt_append: Some("SHOULD_NOT_APPEAR".to_string()),
            skills_section: Some("SHOULD_NOT_APPEAR_SKILLS".into()),
            ..Default::default()
        };
        let agent = stub(vec!["A".to_string()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/tmp", Some("TASK"));
        assert_eq!(out, "OVERRIDE_ONLY");
    }

    #[test]
    fn append_order_config_then_task() {
        let cfg = RuntimePromptConfig {
            system_prompt_override: None,
            system_prompt_append: Some("FROM_CONFIG".to_string()),
            skills_section: None,
            ..Default::default()
        };
        let agent = stub(vec!["T".to_string()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", Some("FROM_TASK"));
        assert!(out.contains("# Custom Agent Instructions"));
        assert!(out.contains("FROM_CONFIG"));
        assert!(out.contains("FROM_TASK"));
        let pos_c = out.find("FROM_CONFIG").unwrap();
        let pos_t = out.find("FROM_TASK").unwrap();
        assert!(pos_c < pos_t, "config append before task append");
    }

    #[test]
    fn cwd_appears_in_default_stack() {
        let cfg = RuntimePromptConfig::default();
        let agent = stub(vec!["X".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/my/cwd", None);
        assert!(out.contains("/my/cwd"));
    }

    #[test]
    fn skills_section_injected_after_tool_list() {
        let cfg = RuntimePromptConfig {
            skills_section: Some("## Available skills\n\n- **demo**: test".into()),
            ..Default::default()
        };
        let agent = stub(vec!["Skill".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        let pos_tools = out.find("Skill").unwrap();
        let pos_sk = out.find("Available skills").unwrap();
        assert!(pos_sk > pos_tools);
    }

    #[test]
    fn skills_section_per_agent_overrides_global() {
        let mut by_agent = HashMap::new();
        by_agent.insert(
            "stub".to_string(),
            "## Available skills\n\n- **only**: per-agent".to_string(),
        );
        let cfg = RuntimePromptConfig {
            skills_section: Some("## Available skills\n\n- **global**: all".into()),
            skills_section_by_agent: by_agent,
            ..Default::default()
        };
        let agent = stub(vec!["Skill".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert!(out.contains("per-agent"));
        assert!(!out.contains("global"));
    }

    #[test]
    fn agent_replace_skips_default_stack_but_keeps_append() {
        let cfg = RuntimePromptConfig {
            system_prompt_override: None,
            system_prompt_append: Some("TAIL".into()),
            skills_section: None,
            ..Default::default()
        };
        let mut agent = stub(vec!["Z".into()]);
        agent.replace = Some("CUSTOM_BODY");
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert!(out.starts_with("CUSTOM_BODY"));
        assert!(!out.contains("# Tone"));
        assert!(out.contains("TAIL"));
    }

    struct OverlayReplaceAgent {
        base: StubAgent,
        overlay: &'static str,
    }

    #[async_trait]
    impl Agent for OverlayReplaceAgent {
        fn agent_type(&self) -> &AgentType {
            self.base.agent_type()
        }
        fn description(&self) -> &str {
            self.base.description()
        }
        fn tools(&self) -> Vec<ToolName> {
            self.base.tools()
        }
        async fn execute(&mut self, _task: Task) -> Result<TaskResult, CoreError> {
            unreachable!()
        }
        fn system_prompt_replaces_default_sections(&self) -> Option<&str> {
            self.base.system_prompt_replaces_default_sections()
        }
        fn system_prompt_overlay(&self) -> Option<&str> {
            Some(self.overlay)
        }
    }

    /// 文件式 agent（md 正文）语义：body → config append → task append → overlay。
    #[test]
    fn replace_path_order_body_config_task_overlay() {
        let cfg = RuntimePromptConfig {
            system_prompt_override: None,
            system_prompt_append: Some("CFG_APPEND".into()),
            skills_section: None,
            ..Default::default()
        };
        let mut base = stub(vec!["Z".into()]);
        base.replace = Some("MD_BODY");
        let agent = OverlayReplaceAgent {
            base,
            overlay: "OVERLAY_LAST",
        };
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", Some("TASK_APPEND"));
        let pb = out.find("MD_BODY").unwrap();
        let pc = out.find("CFG_APPEND").unwrap();
        let pt = out.find("TASK_APPEND").unwrap();
        let po = out.find("OVERLAY_LAST").unwrap();
        assert!(
            pb < pc && pc < pt && pt < po,
            "order body<config<task<overlay"
        );
        assert!(!out.contains("# Tone"));
    }

    #[test]
    fn model_instructions_content_injected_into_prompt() {
        let cfg = RuntimePromptConfig {
            model_instructions_content: Some("You are a helpful assistant.".into()),
            ..Default::default()
        };
        let agent = stub(vec!["T".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert!(out.contains("# Model Instructions"));
        assert!(out.contains("You are a helpful assistant."));
    }

    #[test]
    fn model_instructions_not_shown_when_override() {
        let cfg = RuntimePromptConfig {
            system_prompt_override: Some("OVERRIDE_ONLY".to_string()),
            model_instructions_content: Some("Should not appear".into()),
            ..Default::default()
        };
        let agent = stub(vec!["T".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert_eq!(out, "OVERRIDE_ONLY");
        assert!(!out.contains("Should not appear"));
    }

    #[test]
    fn resolve_model_instructions_file_absolute() {
        use std::io::Write;

        let tmpdir = tempfile::tempdir().unwrap();
        let instructions_path = tmpdir.path().join("AGENTS.md");
        let mut f = std::fs::File::create(&instructions_path).unwrap();
        writeln!(f, "# Custom Instructions\nBe helpful.").unwrap();

        let mut cfg = RuntimePromptConfig {
            model_instructions_file: Some(instructions_path.clone()),
            ..Default::default()
        };

        cfg.resolve_model_instructions_file(std::path::Path::new("/some/working/dir"))
            .unwrap();

        assert!(cfg.model_instructions_content.is_some());
        assert!(cfg
            .model_instructions_content
            .as_ref()
            .unwrap()
            .contains("Be helpful."));
    }

    #[test]
    fn resolve_model_instructions_file_relative() {
        use std::io::Write;

        let tmpdir = tempfile::tempdir().unwrap();
        let instructions_path = tmpdir.path().join("AGENTS.md");
        let mut f = std::fs::File::create(&instructions_path).unwrap();
        writeln!(f, "# Relative Instructions\nFollow these rules.").unwrap();

        let mut cfg = RuntimePromptConfig {
            model_instructions_file: Some(std::path::PathBuf::from("AGENTS.md")),
            ..Default::default()
        };

        cfg.resolve_model_instructions_file(tmpdir.path()).unwrap();

        assert!(cfg.model_instructions_content.is_some());
        assert!(cfg
            .model_instructions_content
            .as_ref()
            .unwrap()
            .contains("Follow these rules."));
    }

    #[test]
    fn media_generation_guidance_in_default_stack() {
        let cfg = RuntimePromptConfig::default();
        let agent = stub(vec!["GenerateVideo".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert!(out.contains("GenerateVideo"));
        assert!(out.contains("Media generation"));
    }

    #[test]
    fn media_and_plan_omitted_without_matching_tools() {
        let cfg = RuntimePromptConfig::default();
        let agent = stub(vec!["Bash".into(), "Read".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        assert!(!out.contains("# Media generation"));
        assert!(!out.contains("# Plan progress"));
    }

    #[tokio::test]
    async fn reply_language_prefers_task_local_context_per_session() {
        // Two concurrent scopes with different languages must not interfere,
        // which the old ANYCODE_REPLY_LANG set_var plumbing could not ensure.
        let zh = tokio::spawn(anycode_core::scope_chat_turn(
            anycode_core::ChatTurnContext {
                dashboard_session_id: Some("sess_zh".into()),
                user_turn_id: Some(1),
                reply_language: Some("zh".into()),
                host_intent_hint: None,
            },
            async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                crate::prompt_catalog::reply_language_section()
            },
        ));
        let en = tokio::spawn(anycode_core::scope_chat_turn(
            anycode_core::ChatTurnContext {
                dashboard_session_id: Some("sess_en".into()),
                user_turn_id: Some(1),
                reply_language: Some("en".into()),
                host_intent_hint: None,
            },
            async {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                crate::prompt_catalog::reply_language_section()
            },
        ));
        let zh_section = zh.await.unwrap().expect("zh directive");
        let en_section = en.await.unwrap().expect("en directive");
        assert!(zh_section.contains("中文"));
        assert!(zh_section.contains("Key findings"));
        assert!(en_section.contains("must be in English"));
        assert!(en_section.contains("关键发现"));
    }

    #[tokio::test]
    async fn reply_language_section_precedes_tone_in_default_stack() {
        let _guard = STABLE_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANYCODE_PROMPT_PREFIX_STABLE");
        let out = anycode_core::scope_chat_turn(
            anycode_core::ChatTurnContext {
                dashboard_session_id: Some("sess".into()),
                user_turn_id: Some(1),
                reply_language: Some("zh".into()),
                host_intent_hint: None,
            },
            async {
                let cfg = RuntimePromptConfig::default();
                let agent = stub(vec!["Bash".into()]);
                compose_effective_system_prompt(&cfg, &agent, "/w", None)
            },
        )
        .await;
        let pos_lang = out
            .find("# Reply language")
            .expect("reply language section");
        let pos_tone = out.find("# Tone").expect("tone section");
        assert!(pos_lang < pos_tone, "Reply language must precede Tone");
        assert!(out.contains("zero visible text"));
    }

    #[test]
    fn resolve_model_instructions_file_missing_is_ok() {
        let mut cfg = RuntimePromptConfig {
            model_instructions_file: Some(std::path::PathBuf::from("/nonexistent/AGENTS.md")),
            ..Default::default()
        };

        // Should not error, just leaves content as None
        cfg.resolve_model_instructions_file(std::path::Path::new("/tmp"))
            .unwrap();

        assert!(cfg.model_instructions_content.is_none());
    }

    /// Env-mutating tests serialize on this lock (env is process-global).
    static STABLE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn prefix_stable_mode_moves_volatile_content_to_trailing_block() {
        let _guard = STABLE_ENV_LOCK.lock().unwrap();
        std::env::set_var("ANYCODE_PROMPT_PREFIX_STABLE", "1");
        let out = anycode_core::scope_chat_turn(
            anycode_core::ChatTurnContext {
                dashboard_session_id: Some("sess_stable".into()),
                user_turn_id: Some(1),
                reply_language: Some("zh".into()),
                host_intent_hint: None,
            },
            async {
                let cfg = RuntimePromptConfig::default();
                let agent = stub(vec!["Bash".into()]);
                compose_effective_system_prompt(&cfg, &agent, "/w", None)
            },
        )
        .await;
        std::env::remove_var("ANYCODE_PROMPT_PREFIX_STABLE");

        let boundary = out
            .find("<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->")
            .expect("boundary marker");
        let session_ctx = out.find("# Session Context").expect("dynamic tail");
        let date_pos = out.find("- Local date:").expect("date line");
        let desc_pos = out.find("# Custom Agent Instructions").expect("agent desc");
        let tone_pos = out.find("# Tone").expect("tone");
        let reply_lang = out.find("# Reply language").expect("reply language");

        // Volatile content strictly after the boundary, static content before.
        assert!(boundary < session_ctx, "dynamic tail after boundary");
        assert!(session_ctx < date_pos, "date rides the tail");
        assert!(desc_pos < boundary, "agent desc stays static");
        assert!(tone_pos < boundary, "tone stays static");
        assert!(reply_lang > boundary, "reply language moves to the tail");
        // The static environment section keeps cwd/OS but drops the date line.
        let static_zone = &out[..boundary];
        assert!(static_zone.contains("- Working directory: /w"));
        assert!(!static_zone.contains("- Local date:"));
    }

    #[test]
    fn default_mode_keeps_legacy_layout() {
        let _guard = STABLE_ENV_LOCK.lock().unwrap();
        std::env::remove_var("ANYCODE_PROMPT_PREFIX_STABLE");
        let cfg = RuntimePromptConfig::default();
        let agent = stub(vec!["Bash".into()]);
        let out = compose_effective_system_prompt(&cfg, &agent, "/w", None);
        // Legacy: date lives inside the environment section, no dynamic block.
        assert!(out.contains("- Local date:"));
        assert!(!out.contains("# Session Context"));
        let desc_pos = out.find("# Custom Agent Instructions").unwrap();
        let boundary = out.find("<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->").unwrap();
        assert!(desc_pos > boundary, "legacy desc stays last");
    }
}
