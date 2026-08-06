//! Register declarative agent profiles and merge routing / skill allowlists.

use crate::model_resolve::resolve_model_profile;
use anycode_agent::{
    is_builtin_extends, profile_spec_for_builtin, resolve_profile as resolve_agent_profile,
    AgentProfileSpec, AgentRuntime, ProfileAgent, ResolvedAgentProfile,
};
use anycode_config::{
    AgentProfileFile, AgentProfileSkillsFile, AgentProfileToolsFile, AgentsConfig,
};
use anycode_core::prelude::*;
use std::collections::HashMap;
use std::sync::Arc;

pub fn merge_profile_routing(
    config: &anycode_config::Config,
    model_overrides: &mut HashMap<AgentType, ModelConfig>,
) {
    for (id, profile) in &config.agents.profiles {
        let Some(routing) = profile.routing.as_ref() else {
            continue;
        };
        if model_overrides.contains_key(&AgentType::new(id)) {
            continue;
        }
        if let Ok(model) = resolve_model_profile(config, routing) {
            model_overrides.insert(AgentType::new(id.clone()), model);
        }
    }
}

fn profile_spec_from_file(spec: &AgentProfileFile) -> AgentProfileSpec {
    AgentProfileSpec {
        extends: spec.extends.clone(),
        description: spec.description.clone(),
        tools_allow: spec.tools.as_ref().and_then(|t| t.allow.clone()),
        tools_deny: spec.tools.as_ref().and_then(|t| t.deny.clone()),
        skills_allowlist: spec.skills.as_ref().and_then(|s| s.allowlist.clone()),
        prompt_overlay: spec.prompt_overlay.clone(),
        system_prompt: spec.system_prompt.clone(),
    }
}

pub fn resolve_profile_from_file(
    id: &str,
    spec: &AgentProfileFile,
    include_skill_on_explore_plan: bool,
) -> ResolvedAgentProfile {
    resolve_agent_profile(
        id,
        &profile_spec_from_file(spec),
        include_skill_on_explore_plan,
    )
}

pub fn resolve_profile(
    id: &str,
    spec: &AgentProfileFile,
    include_skill_on_explore_plan: bool,
) -> Option<ResolvedAgentProfile> {
    Some(resolve_profile_from_file(
        id,
        spec,
        include_skill_on_explore_plan,
    ))
}

/// 空工具面判定：builtin extends 的基础面恒非空，空面只可能来自显式 allow/deny
/// 收窄。此类 profile 必须拒注册，否则空 `agent.tools()` 会被
/// `resolve_agent_tool_names` 兜底成全量注册表（fail-open）。
pub(crate) fn narrowed_to_empty_surface(resolved: &ResolvedAgentProfile) -> bool {
    resolved.tools.is_empty()
}

pub async fn register_declarative_agents(
    runtime: &Arc<AgentRuntime>,
    config: &anycode_config::Config,
    default_model: &ModelConfig,
    model_overrides: &HashMap<AgentType, ModelConfig>,
) {
    let include_skill = config.skills.enabled && config.skills.expose_on_explore_plan;
    for (id, spec) in &config.agents.profiles {
        if is_builtin_extends(id) {
            tracing::warn!(
                target: "anycode_cli",
                "skipping agent profile `{id}`: id conflicts with builtin agent"
            );
            continue;
        }
        let Some(resolved) = resolve_profile(id, spec, include_skill) else {
            continue;
        };
        if narrowed_to_empty_surface(&resolved) {
            // Fail-closed：builtin extends 的基础面非空，空面只可能是显式
            // allow/deny 收窄所致。注册空面 agent 会被 `resolve_agent_tool_names`
            // 兜底成全量注册表（fail-open），此处拒注册——使用时按未知 agent 报错。
            tracing::warn!(
                target: "anycode_cli",
                kind = "agent_profile_empty_surface",
                agent = %id,
                "skipping agent profile `{id}`: tool allow/deny narrowed the surface to zero"
            );
            continue;
        }
        let model = model_overrides
            .get(&AgentType::new(id))
            .cloned()
            .unwrap_or_else(|| default_model.clone());
        let agent = Box::new(ProfileAgent::new(resolved, model)) as Box<dyn Agent>;
        runtime.register_agent(agent).await;
    }
}

pub fn merge_profile_skill_allowlists(
    agents: &AgentsConfig,
    agent_allowlists: &mut HashMap<String, Vec<String>>,
) {
    for (id, spec) in &agents.profiles {
        if let Some(list) = spec
            .skills
            .as_ref()
            .and_then(|s| s.allowlist.as_ref())
            .filter(|v| !v.is_empty())
        {
            agent_allowlists.insert(id.clone(), list.clone());
        }
    }
}

fn agent_profile_file_from_spec(spec: &AgentProfileSpec) -> AgentProfileFile {
    AgentProfileFile {
        extends: spec.extends.clone(),
        description: spec.description.clone(),
        tools: if spec.tools_allow.is_some() || spec.tools_deny.is_some() {
            Some(AgentProfileToolsFile {
                allow: spec.tools_allow.clone(),
                deny: spec.tools_deny.clone(),
            })
        } else {
            None
        },
        skills: spec
            .skills_allowlist
            .as_ref()
            .map(|allowlist| AgentProfileSkillsFile {
                allowlist: Some(allowlist.clone()),
            }),
        routing: None,
        prompt_overlay: spec.prompt_overlay.clone(),
        system_prompt: spec.system_prompt.clone(),
    }
}

/// 文件式 agent 定义（`.anycode/agents/*.md`）→ `AgentProfileFile`。
/// markdown 正文 → `system_prompt`（替换默认段落语义）。`model:` frontmatter v1 不接
/// routing（`ModelProfile` 为结构化类型），由嵌套调用方的 per-call `model` 参数承接。
pub fn agent_file_to_profile(def: &anycode_tools::agent_files::AgentFileDef) -> AgentProfileFile {
    AgentProfileFile {
        extends: def.extends.clone(),
        description: def.description.clone(),
        tools: if def.tools_allow.is_some() || def.tools_deny.is_some() {
            Some(AgentProfileToolsFile {
                allow: def.tools_allow.clone(),
                deny: def.tools_deny.clone(),
            })
        } else {
            None
        },
        skills: def
            .skills_allowlist
            .as_ref()
            .map(|allowlist| AgentProfileSkillsFile {
                allowlist: Some(allowlist.clone()),
            }),
        routing: None,
        prompt_overlay: None,
        system_prompt: def.system_prompt.clone(),
    }
}

/// 扫描 `~/.anycode/agents` 与 `<project>/.anycode/agents`，把文件式 agent 合并进
/// `config.agents.profiles`。**必须在 `merge_profile_routing` 之前调用**，使文件 agent
/// 的 routing / skills allowlist / 注册走既有管线。
///
/// 优先级链：**用户目录 < 项目目录 < config.json < builtin**。
/// - 扫描阶段：`scan_agent_files` 后序 root（项目）覆盖同 id（用户）。
/// - config.json 已有同 id → 文件被遮蔽（skip + `agent_file_shadowed` 日志）。
/// - builtin 冲突：由 `register_declarative_agents` 的 `is_builtin_extends` 检查兜底跳过。
///
/// 信任模型 = 结构收窄：`apply_tool_filters` 只能相对 `extends` 基线收窄工具面
/// （allow 交集 / deny 减法），skills allowlist 只收不扩，无放大字段。加载时打
/// `agent_file_loaded` 审计日志。返回合并入的条目数。
pub fn merge_file_agents_into_config(
    agents: &mut AgentsConfig,
    project_root: Option<&std::path::Path>,
) -> usize {
    let roots = anycode_tools::agent_files::default_agent_roots(project_root);
    let defs = anycode_tools::agent_files::scan_agent_files(&roots);
    merge_agent_defs_into_config(agents, defs)
}

/// 合并已扫描的文件式 agent 定义（与扫描解耦，便于测试注入）。语义与优先级见
/// [`merge_file_agents_into_config`]。
pub fn merge_agent_defs_into_config(
    agents: &mut AgentsConfig,
    defs: Vec<anycode_tools::agent_files::AgentFileDef>,
) -> usize {
    let mut merged = 0usize;
    for def in defs {
        let id = def.id.clone();
        if agents.profiles.contains_key(&id) {
            tracing::info!(
                target: "anycode_cli",
                kind = "agent_file_shadowed",
                agent = %id,
                "file agent definition shadowed by config.json profile"
            );
            continue;
        }
        tracing::info!(
            target: "anycode_cli",
            kind = "agent_file_loaded",
            agent = %id,
            source = ?def.source,
            path = %def.path.display(),
            extends = %def.extends,
            tools_narrowed = def.tools_allow.is_some() || def.tools_deny.is_some(),
            "file agent definition loaded"
        );
        agents.profiles.insert(id, agent_file_to_profile(&def));
        merged += 1;
    }
    merged
}

/// Shipped role presets (extends builtins) for quick start.
pub fn shipped_role_profiles() -> AgentsConfig {
    use anycode_agent::SHIPPED_ROLE_IDS;
    let mut profiles = std::collections::HashMap::new();
    for id in SHIPPED_ROLE_IDS {
        if let Some(spec) = profile_spec_for_builtin(id) {
            profiles.insert(id.to_string(), agent_profile_file_from_spec(&spec));
        }
    }
    AgentsConfig {
        profiles,
        defaults: Default::default(),
    }
}

pub async fn build_agents_setup(
    runtime: &Arc<AgentRuntime>,
    config: &anycode_config::Config,
    default_model: &ModelConfig,
    model_overrides_snapshot: &HashMap<AgentType, ModelConfig>,
    expose_skill_on_explore_plan: bool,
) {
    register_declarative_agents(runtime, config, default_model, model_overrides_snapshot).await;
    let shipped = shipped_role_profiles();
    for (id, spec) in shipped.profiles {
        if config.agents.profiles.contains_key(&id) {
            continue;
        }
        if let Some(resolved) = resolve_profile(&id, &spec, expose_skill_on_explore_plan) {
            let model = model_overrides_snapshot
                .get(&AgentType::new(&id))
                .cloned()
                .unwrap_or_else(|| default_model.clone());
            runtime
                .register_agent(Box::new(ProfileAgent::new(resolved, model)))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewer_profile_denies_bash() {
        let spec = AgentProfileFile {
            extends: "explore".into(),
            description: Some("review".into()),
            tools: Some(AgentProfileToolsFile {
                allow: Some(vec!["FileRead".into(), "Grep".into()]),
                deny: None,
            }),
            skills: None,
            routing: None,
            prompt_overlay: None,
            system_prompt: None,
        };
        let resolved = resolve_profile("reviewer", &spec, false).unwrap();
        assert!(!resolved.tools.contains(&"Bash".to_string()));
        assert!(resolved.tools.contains(&"FileRead".to_string()));
    }

    #[test]
    fn empty_intersection_allow_is_flagged_for_skip() {
        // allow ∩ base = ∅ → 空面标记（register_declarative_agents 据此拒注册，
        // 避免空 tools 被 resolve_agent_tool_names 兜底成全量注册表）。
        let spec = AgentProfileFile {
            extends: "general-purpose".into(),
            description: None,
            tools: Some(AgentProfileToolsFile {
                allow: Some(vec!["NoSuchTool".into()]),
                deny: None,
            }),
            skills: None,
            routing: None,
            prompt_overlay: None,
            system_prompt: None,
        };
        let resolved = resolve_profile("e2e-zero", &spec, false).unwrap();
        assert!(narrowed_to_empty_surface(&resolved));

        // 对照：正常收窄非空 → 不标记。
        let ok = AgentProfileFile {
            tools: Some(AgentProfileToolsFile {
                allow: Some(vec!["FileRead".into()]),
                deny: None,
            }),
            ..spec
        };
        let resolved_ok = resolve_profile("e2e-ok", &ok, false).unwrap();
        assert!(!narrowed_to_empty_surface(&resolved_ok));
    }

    fn file_def(
        id: &str,
        body: &str,
        source: anycode_tools::agent_files::AgentFileSource,
    ) -> anycode_tools::agent_files::AgentFileDef {
        anycode_tools::agent_files::AgentFileDef {
            id: id.to_string(),
            description: Some(format!("{id} desc")),
            extends: "explore".into(),
            model: None,
            tools_allow: Some(vec!["FileRead".into(), "Grep".into()]),
            tools_deny: None,
            skills_allowlist: None,
            system_prompt: Some(body.to_string()),
            source,
            path: std::path::PathBuf::from(format!("/tmp/{id}.md")),
        }
    }

    #[test]
    fn merge_agent_defs_inserts_with_system_prompt() {
        let mut agents = AgentsConfig::default();
        let n = merge_agent_defs_into_config(
            &mut agents,
            vec![file_def(
                "sql-reviewer",
                "You review SQL.",
                anycode_tools::agent_files::AgentFileSource::Project,
            )],
        );
        assert_eq!(n, 1);
        let profile = &agents.profiles["sql-reviewer"];
        assert_eq!(profile.system_prompt.as_deref(), Some("You review SQL."));
        assert_eq!(profile.extends, "explore");
        // resolve 后 system_prompt 穿透且工具面收窄生效
        let resolved = resolve_profile_from_file("sql-reviewer", profile, false);
        assert_eq!(resolved.system_prompt.as_deref(), Some("You review SQL."));
        assert!(resolved.tools.contains(&"FileRead".to_string()));
        assert!(!resolved.tools.contains(&"Bash".to_string()));
    }

    #[test]
    fn merge_agent_defs_config_json_wins_over_file() {
        let mut agents = AgentsConfig::default();
        agents.profiles.insert(
            "sql-reviewer".into(),
            AgentProfileFile {
                extends: "plan".into(),
                description: Some("config version".into()),
                ..Default::default()
            },
        );
        let n = merge_agent_defs_into_config(
            &mut agents,
            vec![file_def(
                "sql-reviewer",
                "file version",
                anycode_tools::agent_files::AgentFileSource::Project,
            )],
        );
        assert_eq!(n, 0, "config.json 同 id 遮蔽文件定义");
        let profile = &agents.profiles["sql-reviewer"];
        assert_eq!(profile.extends, "plan");
        assert!(profile.system_prompt.is_none());
    }

    #[test]
    fn file_agent_md_body_replaces_default_system_prompt() {
        use anycode_core::Agent;
        let def = file_def(
            "md-agent",
            "You are a markdown-defined agent.",
            anycode_tools::agent_files::AgentFileSource::User,
        );
        let profile = agent_file_to_profile(&def);
        let resolved = resolve_profile_from_file("md-agent", &profile, false);
        let agent = ProfileAgent::new(resolved, ModelConfig::default());
        assert_eq!(
            agent.system_prompt_replaces_default_sections(),
            Some("You are a markdown-defined agent.")
        );
    }
}
