//! Convert local runtime traces (episodes + tool audit log) into `TeacherTrajectory`
//! candidates for the offline teacher lab.
//!
//! 约束（ADR 014）：候选必须经人工评审后才能晋级 teacher lab（对齐
//! `eval/skill-bakeoff/SCORECARD` 的 human gate）；本模块只产出候选 JSONL，
//! 运行时不加载磁盘 pack，teacher key 永不进入用户运行时。

use crate::TeacherTrajectory;
use anycode_core::{EpisodeEvent, EpisodeRecord, TaskFamily};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// 单个任务内超过此数量的 denied/error 工具调用 → 轨迹不可靠，丢弃。
pub const MAX_BAD_TOOL_CALLS: usize = 2;

/// tool-calls.jsonl 行的最小视图（其余字段忽略；兼容缺 duration_ms/call_id 的旧行）。
#[derive(Debug, Clone, Deserialize)]
pub struct AuditOutcomeRow {
    pub task_id: String,
    pub outcome: String,
}

/// episodes + audit 行 → teacher 候选。`low_model_replay_gain` 置 0.0 占位，
/// 由 teacher lab 低模型回放后回填。
pub fn extract_candidates(
    episodes: &[EpisodeRecord],
    audit_rows: &[AuditOutcomeRow],
) -> Vec<TeacherTrajectory> {
    let mut bad_calls: HashMap<&str, usize> = HashMap::new();
    for row in audit_rows {
        if matches!(
            row.outcome.as_str(),
            "denied" | "tool_error" | "runtime_error"
        ) {
            *bad_calls.entry(row.task_id.as_str()).or_default() += 1;
        }
    }
    episodes
        .iter()
        .filter(|ep| bad_calls.get(ep.task_id.as_str()).copied().unwrap_or(0) <= MAX_BAD_TOOL_CALLS)
        .filter_map(candidate_from_episode)
        .collect()
}

fn candidate_from_episode(ep: &EpisodeRecord) -> Option<TeacherTrajectory> {
    let mut prompt = None;
    let mut family = TaskFamily::General;
    let mut tool_order = Vec::new();
    let mut notes = Vec::new();
    let mut acceptances = 0usize;
    let mut all_passed = true;
    for evt in &ep.events {
        match evt {
            EpisodeEvent::TaskIntent { summary, family: f } => {
                prompt = Some(summary.clone());
                family = family_from_str(f);
            }
            EpisodeEvent::KeyDecision {
                decision,
                rationale,
            } => {
                if rationale.is_empty() {
                    notes.push(decision.clone());
                } else {
                    notes.push(format!("{decision} ({rationale})"));
                }
            }
            EpisodeEvent::ToolTrace { tool, .. } => tool_order.push(tool.clone()),
            EpisodeEvent::Acceptance { passed, .. } => {
                acceptances += 1;
                all_passed &= passed;
            }
            EpisodeEvent::UserCorrection { before, after } => {
                // 用户纠正是最高价值的修复信号，显式标记
                notes.push(format!("user_correction: {before} -> {after}"));
            }
            EpisodeEvent::Deliverable { .. } => {}
        }
    }
    let prompt = prompt?;
    if tool_order.is_empty() {
        return None;
    }
    Some(TeacherTrajectory {
        id: ep.id.clone(),
        family,
        prompt,
        tool_order,
        notes,
        passed_gates: acceptances > 0 && all_passed,
        low_model_replay_gain: 0.0,
    })
}

fn family_from_str(s: &str) -> TaskFamily {
    match s {
        "web_design" => TaskFamily::WebDesign,
        "cross_file_coding" => TaskFamily::CrossFileCoding,
        "refactor" => TaskFamily::Refactor,
        "research" => TaskFamily::Research,
        "office_delivery" => TaskFamily::OfficeDelivery,
        "database_sql" => TaskFamily::DatabaseSql,
        _ => TaskFamily::General,
    }
}

/// 候选 JSONL 输出：`~/.anycode/trajectories/candidates-<date>.jsonl`。
pub fn write_candidates_jsonl(
    path: impl AsRef<Path>,
    candidates: &[TeacherTrajectory],
) -> anyhow::Result<usize> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = String::new();
    for c in candidates {
        out.push_str(&serde_json::to_string(c)?);
        out.push('\n');
    }
    std::fs::write(path, out)?;
    Ok(candidates.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn episode(id: &str, task_id: &str, events: Vec<EpisodeEvent>) -> EpisodeRecord {
        EpisodeRecord {
            id: id.into(),
            session_id: "s1".into(),
            task_id: task_id.into(),
            events,
            created_at: Utc::now(),
            evidence_hash: String::new(),
        }
    }

    fn intent() -> EpisodeEvent {
        EpisodeEvent::TaskIntent {
            summary: "fix the parser".into(),
            family: "cross_file_coding".into(),
        }
    }

    #[test]
    fn extracts_candidate_with_correction_note() {
        let ep = episode(
            "ep1",
            "t1",
            vec![
                intent(),
                EpisodeEvent::ToolTrace {
                    tool: "Edit".into(),
                    outcome: "ok".into(),
                    ok: true,
                },
                EpisodeEvent::UserCorrection {
                    before: "unwrap".into(),
                    after: "map_err".into(),
                },
                EpisodeEvent::Acceptance {
                    criterion: "cargo test".into(),
                    passed: true,
                    evidence: "ok".into(),
                },
            ],
        );
        let out = extract_candidates(&[ep], &[]);
        assert_eq!(out.len(), 1);
        let c = &out[0];
        assert_eq!(c.family, TaskFamily::CrossFileCoding);
        assert_eq!(c.prompt, "fix the parser");
        assert_eq!(c.tool_order, vec!["Edit"]);
        assert!(c.passed_gates);
        assert!(c.notes.iter().any(|n| n.starts_with("user_correction:")));
        assert_eq!(c.low_model_replay_gain, 0.0);
    }

    #[test]
    fn drops_episode_with_too_many_bad_tool_calls() {
        let ep = episode(
            "ep2",
            "t2",
            vec![
                intent(),
                EpisodeEvent::ToolTrace {
                    tool: "Bash".into(),
                    outcome: "ok".into(),
                    ok: true,
                },
            ],
        );
        let rows: Vec<AuditOutcomeRow> = (0..MAX_BAD_TOOL_CALLS + 1)
            .map(|_| AuditOutcomeRow {
                task_id: "t2".into(),
                outcome: "denied".into(),
            })
            .collect();
        assert!(extract_candidates(&[ep], &rows).is_empty());
        // 阈值内仍保留
        let rows_ok: Vec<AuditOutcomeRow> = (0..MAX_BAD_TOOL_CALLS)
            .map(|_| AuditOutcomeRow {
                task_id: "t2".into(),
                outcome: "tool_error".into(),
            })
            .collect();
        let ep = episode(
            "ep2",
            "t2",
            vec![
                intent(),
                EpisodeEvent::ToolTrace {
                    tool: "Bash".into(),
                    outcome: "ok".into(),
                    ok: true,
                },
            ],
        );
        assert_eq!(extract_candidates(&[ep], &rows_ok).len(), 1);
    }

    #[test]
    fn failed_acceptance_marks_gates_not_passed() {
        let ep = episode(
            "ep3",
            "t3",
            vec![
                intent(),
                EpisodeEvent::ToolTrace {
                    tool: "Bash".into(),
                    outcome: "ok".into(),
                    ok: true,
                },
                EpisodeEvent::Acceptance {
                    criterion: "pytest".into(),
                    passed: false,
                    evidence: "1 failed".into(),
                },
            ],
        );
        let out = extract_candidates(&[ep], &[]);
        assert_eq!(out.len(), 1);
        assert!(!out[0].passed_gates);
    }

    #[test]
    fn skips_episode_without_intent_or_tools() {
        let no_intent = episode(
            "ep4",
            "t4",
            vec![EpisodeEvent::ToolTrace {
                tool: "Bash".into(),
                outcome: "ok".into(),
                ok: true,
            }],
        );
        let no_tools = episode("ep5", "t5", vec![intent()]);
        assert!(extract_candidates(&[no_intent, no_tools], &[]).is_empty());
    }

    #[test]
    fn writes_jsonl_one_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("candidates.jsonl");
        let candidates = vec![TeacherTrajectory {
            id: "x".into(),
            family: TaskFamily::General,
            prompt: "p".into(),
            tool_order: vec!["Bash".into()],
            notes: vec![],
            passed_gates: true,
            low_model_replay_gain: 0.0,
        }];
        let n = write_candidates_jsonl(&path, &candidates).unwrap();
        assert_eq!(n, 1);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 1);
        let parsed: TeacherTrajectory = serde_json::from_str(raw.trim()).unwrap();
        assert_eq!(parsed.id, "x");
    }
}
