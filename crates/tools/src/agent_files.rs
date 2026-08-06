//! 文件式子代理定义（`.anycode/agents/*.md`）。
//!
//! 对齐 Claude Code 的 markdown agent 定义：YAML frontmatter（`name` / `description` /
//! `extends` / `tools` / `skills` / `model`）+ 正文 = 子代理系统提示词。
//! 解析/扫描管线 1:1 复刻 [`crate::skills`]：`---` fence 剥离 + serde_yaml，多 root 扫描
//! **后者优先**（用户目录 → 项目目录，项目覆盖用户），坏文件 warn-and-continue 不致命。
//!
//! 信任模型 = 结构收窄：文件 profile 经 `apply_tool_filters` 只能相对 `extends` 基线
//! **收窄**工具面（allow 交集 / deny 减法），无任何字段可放大权限；加载时 bootstrap
//! 打 `kind="agent_file_loaded"` 审计日志。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Agent id 字符集规则（与 `SkillCatalog::is_valid_skill_id` 一致）：字母/数字/`.`/`_`/`-`。
#[must_use]
pub fn is_valid_agent_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// frontmatter 的 `tools` 字段：纯列表 = allowlist；映射 = allow/deny。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AgentFileTools {
    List(Vec<String>),
    Maps {
        #[serde(default)]
        allow: Option<Vec<String>>,
        #[serde(default)]
        deny: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AgentFileSkills {
    #[serde(default)]
    pub allowlist: Option<Vec<String>>,
}

/// `.anycode/agents/<id>.md` 的 YAML frontmatter。
#[derive(Debug, Clone, Deserialize)]
pub struct AgentFileManifest {
    /// 必须等于文件名干（与 skills 的 `name == 目录名` 对齐）。
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// 基线 builtin：general-purpose / explore / plan / workspace-assistant / goal。
    #[serde(default)]
    pub extends: Option<String>,
    /// 路由模型 shorthand（v1 不接 ModelProfile，见 bootstrap 说明）。
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Option<AgentFileTools>,
    #[serde(default)]
    pub skills: Option<AgentFileSkills>,
}

/// 定义来源（项目级覆盖用户级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentFileSource {
    User,
    Project,
}

/// 扫描解析后的单个文件式 agent 定义。
#[derive(Debug, Clone)]
pub struct AgentFileDef {
    pub id: String,
    pub description: Option<String>,
    pub extends: String,
    pub model: Option<String>,
    pub tools_allow: Option<Vec<String>>,
    pub tools_deny: Option<Vec<String>>,
    pub skills_allowlist: Option<Vec<String>>,
    /// markdown 正文（去 frontmatter，trimmed）；空正文为 `None`（走 overlay 老语义）。
    pub system_prompt: Option<String>,
    pub source: AgentFileSource,
    pub path: PathBuf,
}

/// 解析 markdown 文本为（manifest, 正文）。frontmatter 缺失/坏 YAML → `None`。
/// 复刻 `parse_skill_manifest_text` + `extract_skill_body` 的 fence 处理（含 `\r\n`）。
#[must_use]
pub fn parse_agent_file_text(text: &str) -> Option<(AgentFileManifest, String)> {
    let t = text.trim_start();
    let rest = t.strip_prefix("---")?.trim_start();
    let end = rest.find("\n---")?;
    let yaml = &rest[..end];
    let manifest: AgentFileManifest = serde_yaml::from_str(yaml).ok()?;
    // 正文：跳过闭合 fence（容忍 `\r`）
    let body = rest[end + 4..]
        .trim_start_matches(['\r', '\n'])
        .trim()
        .to_string();
    Some((manifest, body))
}

/// 扫描根目录列表（`(root, source)`），返回合并后的定义：**后序 root 覆盖同 id**。
/// 目录缺失跳过；坏文件/非法 id/name 不匹配 → warn + continue，永不致命。
#[must_use]
pub fn scan_agent_files(roots: &[(PathBuf, AgentFileSource)]) -> Vec<AgentFileDef> {
    let mut by_id: std::collections::HashMap<String, AgentFileDef> =
        std::collections::HashMap::new();
    for (root, source) in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if !is_valid_agent_id(stem) {
                tracing::warn!(
                    target: "anycode_tools",
                    path = %path.display(),
                    "agent file skipped: invalid id (letters, digits, . _ - only)"
                );
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                tracing::warn!(
                    target: "anycode_tools",
                    path = %path.display(),
                    "agent file skipped: unreadable"
                );
                continue;
            };
            let Some((manifest, body)) = parse_agent_file_text(&text) else {
                tracing::warn!(
                    target: "anycode_tools",
                    path = %path.display(),
                    "agent file skipped: frontmatter missing or unparseable"
                );
                continue;
            };
            if manifest.name.trim() != stem {
                tracing::warn!(
                    target: "anycode_tools",
                    path = %path.display(),
                    name = %manifest.name,
                    expected = stem,
                    "agent file skipped: frontmatter `name` must equal file stem"
                );
                continue;
            }
            let (tools_allow, tools_deny) = match manifest.tools {
                Some(AgentFileTools::List(list)) => (Some(list), None),
                Some(AgentFileTools::Maps { allow, deny }) => (allow, deny),
                None => (None, None),
            };
            let def = AgentFileDef {
                id: stem.to_string(),
                description: manifest.description.filter(|s| !s.trim().is_empty()),
                extends: manifest
                    .extends
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| "general-purpose".to_string()),
                model: manifest.model.filter(|s| !s.trim().is_empty()),
                tools_allow,
                tools_deny,
                skills_allowlist: manifest
                    .skills
                    .and_then(|s| s.allowlist)
                    .filter(|l| !l.is_empty()),
                system_prompt: if body.trim().is_empty() {
                    None
                } else {
                    Some(body)
                },
                source: *source,
                path: path.clone(),
            };
            by_id.insert(def.id.clone(), def);
        }
    }
    let mut out: Vec<AgentFileDef> = by_id.into_values().collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 默认扫描根：`~/.anycode/agents`（用户级）→ `<project>/.anycode/agents`（项目级，覆盖用户）。
#[must_use]
pub fn default_agent_roots(project_root: Option<&Path>) -> Vec<(PathBuf, AgentFileSource)> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        roots.push((home.join(".anycode").join("agents"), AgentFileSource::User));
    }
    if let Some(project) = project_root {
        roots.push((
            project.join(".anycode").join("agents"),
            AgentFileSource::Project,
        ));
    }
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(name: &str, body: &str) -> String {
        format!("---\nname: {name}\ndescription: test agent\n---\n{body}")
    }

    #[test]
    fn parses_manifest_and_body() {
        let text = md("my-agent", "You are a reviewer.\nBe strict.");
        let (manifest, body) = parse_agent_file_text(&text).expect("parse");
        assert_eq!(manifest.name, "my-agent");
        assert_eq!(manifest.description.as_deref(), Some("test agent"));
        assert_eq!(body, "You are a reviewer.\nBe strict.");
    }

    #[test]
    fn parses_crlf_fences() {
        let text = "---\r\nname: crlf-agent\r\n---\r\nbody text";
        // fence 查找基于 "\n---"；`\r\n---` 中 `\n---` 命中，正文剥离容忍 `\r`
        let (manifest, body) = parse_agent_file_text(text).expect("parse crlf");
        assert_eq!(manifest.name, "crlf-agent");
        assert_eq!(body, "body text");
    }

    #[test]
    fn empty_body_parses_with_empty_string() {
        let text = md("empty-body", "");
        let (manifest, body) = parse_agent_file_text(&text).expect("parse");
        assert_eq!(manifest.name, "empty-body");
        assert!(body.is_empty());
    }

    #[test]
    fn missing_frontmatter_is_none() {
        assert!(parse_agent_file_text("no fences here").is_none());
    }

    #[test]
    fn bad_yaml_is_none() {
        let text = "---\nname: [unclosed\n---\nbody";
        assert!(parse_agent_file_text(text).is_none());
    }

    #[test]
    fn tools_as_plain_list_and_maps() {
        let list: AgentFileManifest =
            serde_yaml::from_str("name: a\ntools: [FileRead, Grep]").unwrap();
        assert!(matches!(list.tools, Some(AgentFileTools::List(ref l)) if l.len() == 2));
        let maps: AgentFileManifest =
            serde_yaml::from_str("name: a\ntools:\n  allow: [FileRead]\n  deny: [Bash]").unwrap();
        assert!(matches!(
            maps.tools,
            Some(AgentFileTools::Maps {
                allow: Some(_),
                deny: Some(_)
            })
        ));
    }

    #[test]
    fn valid_agent_id_charset() {
        assert!(is_valid_agent_id("sql-reviewer.v2_x"));
        assert!(!is_valid_agent_id(""));
        assert!(!is_valid_agent_id("bad/slash"));
        assert!(!is_valid_agent_id("bad space"));
    }

    #[test]
    fn scan_precedence_and_warn_continue() {
        let temp = tempfile::tempdir().unwrap();
        let user = temp.path().join("user");
        let project = temp.path().join("project");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&project).unwrap();
        // 用户级与项目级同名 → 项目胜
        std::fs::write(user.join("shared.md"), md("shared", "user version")).unwrap();
        std::fs::write(project.join("shared.md"), md("shared", "project version")).unwrap();
        // 用户级独有
        std::fs::write(user.join("user-only.md"), md("user-only", "u")).unwrap();
        // 坏文件：name 与文件名干不匹配 → 跳过但不影响其它
        std::fs::write(project.join("mismatch.md"), md("other-name", "body")).unwrap();
        // 非法 id 字符
        std::fs::write(project.join("bad id.md"), md("bad id", "body")).unwrap();

        let defs = scan_agent_files(&[
            (user.clone(), AgentFileSource::User),
            (project.clone(), AgentFileSource::Project),
        ]);
        let ids: Vec<&str> = defs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["shared", "user-only"]);
        let shared = defs.iter().find(|d| d.id == "shared").unwrap();
        assert_eq!(shared.source, AgentFileSource::Project);
        assert_eq!(shared.system_prompt.as_deref(), Some("project version"));
    }

    #[test]
    fn scan_missing_dirs_ok() {
        let temp = tempfile::tempdir().unwrap();
        let defs = scan_agent_files(&[(temp.path().join("nope"), AgentFileSource::User)]);
        assert!(defs.is_empty());
    }

    #[test]
    fn default_roots_user_then_project() {
        let roots = default_agent_roots(Some(Path::new("/tmp/proj")));
        assert_eq!(roots.len(), 2);
        assert_eq!(roots[0].1, AgentFileSource::User);
        assert_eq!(roots[1].1, AgentFileSource::Project);
        assert!(roots[1].0.ends_with(".anycode/agents"));
        let roots_no_project = default_agent_roots(None);
        assert_eq!(roots_no_project.len(), 1);
    }

    #[test]
    fn extends_defaults_to_general_purpose() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("agents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("plain.md"), md("plain", "body")).unwrap();
        let defs = scan_agent_files(&[(dir, AgentFileSource::User)]);
        assert_eq!(defs[0].extends, "general-purpose");
    }
}
