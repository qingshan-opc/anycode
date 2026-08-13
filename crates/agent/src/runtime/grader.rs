//! P0.1 意图 rubric + P0.3 critic 复核:完成守卫的 LLM 判定层。
//!
//! 确定性 validators 验证「这是个合格的产物」;grader 验证「这是用户要的那个
//! 东西」。设计对标 Managed Agents outcomes:rubric 由独立上下文生成一次
//! (避免被主 agent 的实现选择带偏),判定时逐条评分,default-to-refuted
//! (缺 structured output / schema 非法一律 refuted,与 dashboard llm_gate 一致)。
//!
//! 可用性边界:LLM 传输失败或输出无法形成判定 → `Unavailable`(记逃逸度量,
//! 不阻断完成)——常驻环内复核以可用性优先;显式 refuted 判定才阻断。
//! (dashboard 手动发布闸 llm_gate 保持更严的 default-to-refuted 语义。)

use anycode_core::{
    Artifact, GateSeverity, LLMClient, Message, MessageContent, MessageRole, ModelConfig,
    RubricItem, TaskFamily, VerificationReport,
};
use std::sync::Arc;

const MAX_RUBRIC_ITEMS: usize = 6;
const MAX_PROMPT_CHARS: usize = 4_000;
const MAX_ASSISTANT_CHARS: usize = 3_000;

pub enum GraderVerdict {
    Pass,
    /// 返修消息(含未通过 rubric 条目与理由)。
    Refuted(String),
    /// LLM 不可用(传输层失败);调用方记逃逸度量后放行。
    Unavailable,
}

/// 生成任务级 rubric(每会话一次)。失败返回 None,调用方跳过 grader 层。
pub async fn generate_rubric(
    llm: &Arc<dyn LLMClient>,
    config: &ModelConfig,
    task_prompt: &str,
    family: Option<TaskFamily>,
) -> Option<Vec<RubricItem>> {
    let prompt = format!(
        "You are writing acceptance criteria for an AI agent's deliverable. \
         Given the user's request, produce up to {MAX_RUBRIC_ITEMS} concise, objectively \
         checkable acceptance criteria (rubric items). Each must be verifiable from the \
         produced artifacts and the agent's final report — no subjective taste judgments.\n\n\
         Task family: {}\nUser request:\n{}\n\n\
         Reply with ONLY a JSON array: [{{\"id\":\"r1\",\"requirement\":\"...\",\"severity\":\"p0\"|\"p1\"}}]",
        family.map(|f| f.as_str()).unwrap_or("general"),
        truncate(task_prompt, MAX_PROMPT_CHARS),
    );
    let text = chat_text(llm, config, &prompt).await.ok()?;
    parse_rubric(&text)
}

/// rubric + 对抗复核一次完成:rubric 为空时退化为纯 critic(声明/证据对齐审查)。
pub async fn grade_completion(
    llm: &Arc<dyn LLMClient>,
    config: &ModelConfig,
    rubric: &[RubricItem],
    task_prompt: &str,
    assistant_text: &str,
    artifacts: &[Artifact],
    report: Option<&VerificationReport>,
) -> GraderVerdict {
    let rubric_text = if rubric.is_empty() {
        "(no rubric; judge claim/evidence alignment adversarially)".to_string()
    } else {
        rubric
            .iter()
            .map(|r| format!("- [{}] {} ({})", r.id, r.requirement, r.severity.as_str()))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let artifacts_text = if artifacts.is_empty() {
        "(none declared)".to_string()
    } else {
        artifacts
            .iter()
            .take(16)
            .map(|a| {
                format!(
                    "- {} ({})",
                    a.path.as_deref().unwrap_or(&a.name),
                    a.kind.as_deref().unwrap_or("file")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let gates_text = report
        .map(|r| {
            r.results
                .iter()
                .map(|x| format!("- {} [{}]: {:?}", x.gate_id, x.severity.as_str(), x.outcome))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_else(|| "(no deterministic gates ran)".into());

    let prompt = format!(
        "You are an adversarial completion grader. Your DEFAULT verdict is REFUTED — only \
         return \"pass\" when the evidence clearly satisfies every rubric item and the agent's \
         claims match the declared artifacts. Deterministic gates below already checked file-level \
         validity; your job is semantic fit to the user's request.\n\n\
         ## User request\n{}\n\n## Rubric\n{}\n\n## Declared artifacts\n{}\n\n\
         ## Deterministic gate results\n{}\n\n## Agent's final report\n{}\n\n\
         Reply with ONLY a JSON object: \
         {{\"items\":[{{\"id\":\"r1\",\"pass\":true,\"reason\":\"...\"}}],\
         \"verdict\":\"pass\"|\"refuted\",\"reason\":\"...\"}}",
        truncate(task_prompt, MAX_PROMPT_CHARS),
        rubric_text,
        artifacts_text,
        gates_text,
        truncate(assistant_text, MAX_ASSISTANT_CHARS),
    );

    let text = match chat_text(llm, config, &prompt).await {
        Ok(t) => t,
        Err(_) => return GraderVerdict::Unavailable,
    };
    match parse_verdict(&text) {
        Some(true) => GraderVerdict::Pass,
        Some(false) => {
            let reason = parse_reason(&text).unwrap_or_else(|| "grader refuted".into());
            GraderVerdict::Refuted(format!(
                "## Adversarial grader refuted completion\n{reason}\nFix the issues above (or correct your final report if it overstated the evidence), then claim done again."
            ))
        }
        // 输出无法形成判定(解析失败)按 Unavailable 处理:grader 可靠性问题不应
        // 误伤交付(与 dashboard 手动 llm gate 的 default-to-refuted 刻意不同——
        // 那里是发布闸,这里是常驻环内复核,可用性优先,逃逸由 grader_verdict 度量)。
        None => GraderVerdict::Unavailable,
    }
}

async fn chat_text(
    llm: &Arc<dyn LLMClient>,
    config: &ModelConfig,
    prompt: &str,
) -> Result<String, anycode_core::CoreError> {
    let msg = Message {
        id: anycode_core::TaskId::new_v4(),
        role: MessageRole::User,
        content: MessageContent::Text(prompt.to_string()),
        timestamp: chrono::Utc::now(),
        metadata: Default::default(),
    };
    let resp = llm.chat(vec![msg], vec![], config).await?;
    Ok(match resp.message.content {
        MessageContent::Text(t) => t,
        _ => String::new(),
    })
}

fn truncate(s: &str, max: usize) -> &str {
    if s.chars().count() <= max {
        s
    } else {
        let byte_idx = s.char_indices().nth(max).map(|(i, _)| i).unwrap_or(s.len());
        &s[..byte_idx]
    }
}

/// 从模型输出中提取 JSON 对象/数组(容忍 ```json 围栏与前后杂言)。
fn extract_json<'a>(text: &'a str, open: char, close: char) -> Option<&'a str> {
    let start = text.find(open)?;
    let mut depth = 0usize;
    for (i, c) in text[start..].char_indices() {
        match c {
            c if c == open => depth += 1,
            c if c == close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&text[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_rubric(text: &str) -> Option<Vec<RubricItem>> {
    let raw = extract_json(text, '[', ']')?;
    let items: Vec<serde_json::Value> = serde_json::from_str(raw).ok()?;
    let out: Vec<RubricItem> = items
        .iter()
        .take(MAX_RUBRIC_ITEMS)
        .filter_map(|v| {
            let req = v.get("requirement")?.as_str()?.trim();
            if req.is_empty() {
                return None;
            }
            let severity = match v.get("severity").and_then(|s| s.as_str()) {
                Some("p0") => GateSeverity::P0,
                Some("p1") => GateSeverity::P1,
                _ => GateSeverity::Info,
            };
            Some(RubricItem {
                id: v
                    .get("id")
                    .and_then(|s| s.as_str())
                    .unwrap_or("r?")
                    .to_string(),
                requirement: req.to_string(),
                severity,
            })
        })
        .collect();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

fn parse_verdict(text: &str) -> Option<bool> {
    let raw = extract_json(text, '{', '}')?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    match v.get("verdict").and_then(|x| x.as_str())? {
        "pass" => Some(true),
        "refuted" => Some(false),
        _ => None,
    }
}

fn parse_reason(text: &str) -> Option<String> {
    let raw = extract_json(text, '{', '}')?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let mut lines = Vec::new();
    if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
        for it in items {
            if it.get("pass").and_then(|p| p.as_bool()) == Some(false) {
                let id = it.get("id").and_then(|s| s.as_str()).unwrap_or("?");
                let reason = it.get("reason").and_then(|s| s.as_str()).unwrap_or("");
                lines.push(format!("- [{id}] {reason}"));
            }
        }
    }
    if let Some(r) = v.get("reason").and_then(|s| s.as_str()) {
        if !r.trim().is_empty() {
            lines.push(r.to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rubric_array_with_fence() {
        let text = "Here you go:\n```json\n[{\"id\":\"r1\",\"requirement\":\"page has hero\",\"severity\":\"p0\"}]\n```";
        let items = parse_rubric(text).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].severity, GateSeverity::P0);
    }

    #[test]
    fn rubric_empty_array_is_none() {
        assert!(parse_rubric("[]").is_none());
        assert!(parse_rubric("no json here").is_none());
    }

    #[test]
    fn verdict_parsing() {
        assert_eq!(
            parse_verdict(r#"{"items":[],"verdict":"pass","reason":"ok"}"#),
            Some(true)
        );
        assert_eq!(
            parse_verdict("blah {\"verdict\":\"refuted\",\"reason\":\"no evidence\"} tail"),
            Some(false)
        );
        assert_eq!(parse_verdict("{\"unexpected\":true}"), None);
    }

    #[test]
    fn reason_collects_failed_items_and_top_reason() {
        let text = r#"{"items":[{"id":"r1","pass":false,"reason":"missing hero"}],"verdict":"refuted","reason":"insufficient evidence"}"#;
        let r = parse_reason(text).unwrap();
        assert!(r.contains("[r1] missing hero"));
        assert!(r.contains("insufficient evidence"));
    }
}
