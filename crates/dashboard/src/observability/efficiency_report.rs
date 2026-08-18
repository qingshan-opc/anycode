//! Weekly efficiency report: deterministic aggregation over local telemetry.
//!
//! 数据源：`~/.anycode/audit/tool-calls.jsonl`（D1 起带 duration_ms/call_id，旧行降级跳过）、
//! dashboard SQLite 的 `chat_turn_events`（turn_done 状态分布）与 `project_events`
//!（llm_usage 的 tokens/elapsed）。纯 Rust 聚合，无 LLM；产物落在
//! `~/.anycode/reports/efficiency-<YYYY-Www>.{json,md}`，Workbench 经 API 读取。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub const REPORT_WINDOW_DAYS: u32 = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStat {
    pub tool_name: String,
    pub calls: u64,
    pub error_rate: f64,
    pub denied_rate: f64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCount {
    pub status: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmUsageSummary {
    pub llm_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub p50_ms: Option<u64>,
    pub p95_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyReport {
    pub generated_at: String,
    pub window_days: u32,
    pub tools: Vec<ToolStat>,
    /// execute-phase 行中 (tool_name, input_hash) 在窗口内重复出现的占比（≈无效重搜率）。
    pub repeat_input_rate: f64,
    pub turn_status: Vec<StatusCount>,
    pub llm: LlmUsageSummary,
    /// P2.8 门禁度量(逃逸率/返工率/grader 判定),数据源
    /// `~/.anycode/logs/delivery-gates.jsonl`;无数据或旧版报告为 None。
    #[serde(default)]
    pub gates: Option<anycode_agent::DeliveryGatesSummary>,
}

/// tool-calls.jsonl 的一行（字段对旧格式宽容：缺的按 None 处理）。
#[derive(Debug, Clone, Deserialize)]
pub struct AuditRow {
    pub ts: String,
    pub phase: String,
    pub tool_name: String,
    pub input_hash: String,
    pub outcome: String,
    pub duration_ms: Option<u64>,
}

/// llm_usage 事件的一行（从 payload_json 提取）。
#[derive(Debug, Clone)]
pub struct UsageRow {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
}

fn nearest_rank_percentile(sorted: &[u64], pct: f64) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((sorted.len() as f64) * pct).ceil() as usize;
    sorted.get(rank.saturating_sub(1)).copied()
}

/// 纯聚合核心（单测友好）。
pub fn build_report(
    window_days: u32,
    tool_rows: &[AuditRow],
    turn_status_counts: Vec<StatusCount>,
    usage_rows: &[UsageRow],
    gate_lines: Vec<String>,
) -> EfficiencyReport {
    // per-tool 统计
    let mut by_tool: HashMap<&str, (u64, u64, Vec<u64>)> = HashMap::new(); // (errors, denied, durations)
    let mut calls: HashMap<&str, u64> = HashMap::new();
    for row in tool_rows {
        *calls.entry(row.tool_name.as_str()).or_default() += 1;
        let entry = by_tool.entry(row.tool_name.as_str()).or_default();
        match row.outcome.as_str() {
            "tool_error" | "runtime_error" => entry.0 += 1,
            "denied" => entry.1 += 1,
            _ => {}
        }
        if let Some(d) = row.duration_ms {
            entry.2.push(d);
        }
    }
    let mut tools: Vec<ToolStat> = calls
        .iter()
        .map(|(name, &n)| {
            let (errors, denied, durations) = by_tool.get(name).cloned().unwrap_or_default();
            let mut sorted = durations;
            sorted.sort_unstable();
            ToolStat {
                tool_name: (*name).to_string(),
                calls: n,
                error_rate: errors as f64 / n.max(1) as f64,
                denied_rate: denied as f64 / n.max(1) as f64,
                p50_ms: nearest_rank_percentile(&sorted, 0.50),
                p95_ms: nearest_rank_percentile(&sorted, 0.95),
            }
        })
        .collect();
    tools.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
    });

    // 重搜率：execute-phase 行里 (tool, input_hash) 非首次出现的占比
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut exec_total = 0u64;
    let mut exec_repeat = 0u64;
    for row in tool_rows.iter().filter(|r| r.phase == "execute") {
        exec_total += 1;
        if !seen.insert((row.tool_name.as_str(), row.input_hash.as_str())) {
            exec_repeat += 1;
        }
    }
    let repeat_input_rate = if exec_total == 0 {
        0.0
    } else {
        exec_repeat as f64 / exec_total as f64
    };

    let mut turn_status = turn_status_counts;
    turn_status.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.status.cmp(&b.status)));

    let mut elapsed: Vec<u64> = usage_rows.iter().map(|r| r.elapsed_ms).collect();
    elapsed.sort_unstable();
    let llm = LlmUsageSummary {
        llm_calls: usage_rows.len() as u64,
        input_tokens: usage_rows.iter().map(|r| r.input_tokens).sum(),
        output_tokens: usage_rows.iter().map(|r| r.output_tokens).sum(),
        p50_ms: nearest_rank_percentile(&elapsed, 0.50),
        p95_ms: nearest_rank_percentile(&elapsed, 0.95),
    };

    let gates = {
        let s = anycode_agent::summarize_lines(gate_lines);
        // 完全无门禁数据时置 None,避免报告出现全零段。
        (s.guard_evaluations > 0 || s.skipped > 0 || s.fallback_used > 0).then_some(s)
    };

    EfficiencyReport {
        generated_at: chrono::Utc::now().to_rfc3339(),
        window_days,
        tools,
        repeat_input_rate,
        turn_status,
        llm,
        gates,
    }
}

fn reports_dir() -> Option<PathBuf> {
    Some(anycode_core::user_home_dir()?.join(".anycode/reports"))
}

fn audit_log_path() -> Option<PathBuf> {
    Some(anycode_core::user_home_dir()?.join(".anycode/audit/tool-calls.jsonl"))
}

pub fn iso_week_stamp(now: chrono::DateTime<chrono::Utc>) -> String {
    let week = now.format("%G-W%V").to_string();
    week
}

/// 本周报告已存在则跳过（Ok(None)）；否则生成并返回路径。
pub async fn generate_weekly_report(db: &crate::db::DashboardDb) -> Result<Option<PathBuf>> {
    let Some(dir) = reports_dir() else {
        return Ok(None);
    };
    let stamp = iso_week_stamp(chrono::Utc::now());
    let json_path = dir.join(format!("efficiency-{stamp}.json"));
    if json_path.exists() {
        return Ok(None);
    }

    let tool_rows = read_audit_rows(REPORT_WINDOW_DAYS);
    let turn_status = query_turn_status_counts(db, REPORT_WINDOW_DAYS).await?;
    let usage_rows = query_llm_usage_rows(db, REPORT_WINDOW_DAYS).await?;
    let gate_lines = read_gate_lines(REPORT_WINDOW_DAYS);
    let report = build_report(
        REPORT_WINDOW_DAYS,
        &tool_rows,
        turn_status,
        &usage_rows,
        gate_lines,
    );

    std::fs::create_dir_all(&dir).context("create reports dir")?;
    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&json_path, json)?;
    std::fs::write(
        dir.join(format!("efficiency-{stamp}.md")),
        render_markdown(&report),
    )?;
    Ok(Some(json_path))
}

/// 最新一份周报（Workbench API 用）。
pub fn read_latest_report() -> Result<Option<EfficiencyReport>> {
    let Some(dir) = reports_dir() else {
        return Ok(None);
    };
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("efficiency-") && n.ends_with(".json"))
                })
                .collect()
        })
        .unwrap_or_default();
    candidates.sort();
    let Some(latest) = candidates.pop() else {
        return Ok(None);
    };
    let raw =
        std::fs::read_to_string(&latest).with_context(|| format!("read {}", latest.display()))?;
    let report = serde_json::from_str(&raw).context("parse report json")?;
    Ok(Some(report))
}

fn read_audit_rows(window_days: u32) -> Vec<AuditRow> {
    let Some(path) = audit_log_path() else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days as i64);
    raw.lines()
        .filter_map(|line| serde_json::from_str::<AuditRow>(line).ok())
        .filter(|row| {
            chrono::DateTime::parse_from_rfc3339(&row.ts)
                .map(|ts| ts >= cutoff)
                .unwrap_or(false)
        })
        .collect()
}

/// P2.8:读取窗口内的 delivery-gates.jsonl 行(坏行保留给 summarize 内部跳过)。
fn read_gate_lines(window_days: u32) -> Vec<String> {
    let Some(home) = anycode_core::user_home_dir() else {
        return Vec::new();
    };
    let path = home.join(anycode_agent::DELIVERY_GATES_LOG);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::days(window_days as i64);
    raw.lines()
        .filter(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            v.get("ts")
                .and_then(|t| t.as_str())
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|ts| ts >= cutoff)
                .unwrap_or(false)
        })
        .map(|s| s.to_string())
        .collect()
}

async fn query_turn_status_counts(
    db: &crate::db::DashboardDb,
    days: u32,
) -> Result<Vec<StatusCount>> {
    use sqlx::Row;
    let span = format!("-{} days", days.clamp(1, 90));
    let rows = sqlx::query(
        r#"SELECT COALESCE(json_extract(payload_json, '$.status'), 'completed') AS status,
                  COUNT(*) AS n
           FROM chat_turn_events
           WHERE kind = 'turn_done' AND datetime(occurred_at) >= datetime('now', ?)
           GROUP BY status ORDER BY n DESC"#,
    )
    .bind(&span)
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| StatusCount {
            status: r.get::<String, _>("status"),
            count: r.get::<i64, _>("n") as u64,
        })
        .collect())
}

async fn query_llm_usage_rows(db: &crate::db::DashboardDb, days: u32) -> Result<Vec<UsageRow>> {
    use sqlx::Row;
    let span = format!("-{} days", days.clamp(1, 90));
    let rows = sqlx::query(
        r#"SELECT CAST(json_extract(payload_json, '$.input_tokens') AS INTEGER) AS input_tokens,
                  CAST(json_extract(payload_json, '$.output_tokens') AS INTEGER) AS output_tokens,
                  CAST(json_extract(payload_json, '$.elapsed_ms') AS INTEGER) AS elapsed_ms
           FROM project_events
           WHERE event_type IN ('llm_usage', 'llm_response_end')
             AND datetime(occurred_at) >= datetime('now', ?)"#,
    )
    .bind(&span)
    .fetch_all(db.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| UsageRow {
            input_tokens: r.get::<Option<i64>, _>("input_tokens").unwrap_or(0).max(0) as u64,
            output_tokens: r.get::<Option<i64>, _>("output_tokens").unwrap_or(0).max(0) as u64,
            elapsed_ms: r.get::<Option<i64>, _>("elapsed_ms").unwrap_or(0).max(0) as u64,
        })
        .collect())
}

fn render_markdown(r: &EfficiencyReport) -> String {
    let mut md = format!(
        "# Weekly efficiency report ({} days)\n\ngenerated: {}\n\n",
        r.window_days, r.generated_at
    );
    md.push_str(&format!(
        "- LLM calls: {} (in {} / out {} tokens, p50 {} ms, p95 {} ms)\n",
        r.llm.llm_calls,
        r.llm.input_tokens,
        r.llm.output_tokens,
        r.llm.p50_ms.unwrap_or(0),
        r.llm.p95_ms.unwrap_or(0)
    ));
    md.push_str(&format!(
        "- repeat-input (re-search) rate: {:.1}%\n\n",
        r.repeat_input_rate * 100.0
    ));
    md.push_str("## Tools\n\n| tool | calls | error % | denied % | p50 ms | p95 ms |\n|---|---|---|---|---|---|\n");
    for t in &r.tools {
        md.push_str(&format!(
            "| {} | {} | {:.1} | {:.1} | {} | {} |\n",
            t.tool_name,
            t.calls,
            t.error_rate * 100.0,
            t.denied_rate * 100.0,
            t.p50_ms.map(|v| v.to_string()).unwrap_or("-".into()),
            t.p95_ms.map(|v| v.to_string()).unwrap_or("-".into()),
        ));
    }
    md.push_str("\n## Turn status\n\n");
    for s in &r.turn_status {
        md.push_str(&format!("- {}: {}\n", s.status, s.count));
    }
    if let Some(g) = &r.gates {
        md.push_str("\n## Delivery gates\n\n");
        md.push_str(&format!(
            "- guard evaluations: {} (complete {} / repair {} / partial {} / failed {})\n",
            g.guard_evaluations, g.guard_complete, g.guard_repair, g.guard_partial, g.guard_failed
        ));
        md.push_str(&format!(
            "- escape rate: {:.1}% (fallback {} / skipped {} / unverified-pass {})\n",
            g.escape_rate() * 100.0,
            g.fallback_used,
            g.skipped,
            g.verification_escapes
        ));
        if !g.skipped_by_reason.is_empty() {
            for (reason, n) in &g.skipped_by_reason {
                md.push_str(&format!("  - skipped[{reason}]: {n}\n"));
            }
        }
        md.push_str(&format!(
            "- grader: pass {} / refuted {} / unavailable {}\n",
            g.grader_pass, g.grader_refuted, g.grader_unavailable
        ));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(phase: &str, tool: &str, hash: &str, outcome: &str, dur: Option<u64>) -> AuditRow {
        AuditRow {
            ts: "2026-08-06T00:00:00Z".into(),
            phase: phase.into(),
            tool_name: tool.into(),
            input_hash: hash.into(),
            outcome: outcome.into(),
            duration_ms: dur,
        }
    }

    #[test]
    fn build_report_aggregates_tools() {
        let rows = vec![
            row("execute", "Bash", "h1", "allowed", None),
            row("result", "Bash", "h1", "ok", Some(100)),
            row("execute", "Bash", "h1", "allowed", None), // 重复输入
            row("result", "Bash", "h1", "tool_error", Some(300)),
            row("result", "WebSearch", "h2", "ok", Some(50)),
            row("pre_check", "Grep", "h3", "denied", None),
        ];
        let r = build_report(7, &rows, vec![], &[], vec![]);
        let bash = r.tools.iter().find(|t| t.tool_name == "Bash").unwrap();
        assert_eq!(bash.calls, 4);
        assert_eq!(bash.error_rate, 0.25);
        assert_eq!(bash.p50_ms, Some(100));
        assert_eq!(bash.p95_ms, Some(300));
        let grep = r.tools.iter().find(|t| t.tool_name == "Grep").unwrap();
        assert_eq!(grep.denied_rate, 1.0);
        assert!(grep.p50_ms.is_none());
        // execute 行 2 条，其中 1 条重复
        assert_eq!(r.repeat_input_rate, 0.5);
    }

    #[test]
    fn build_report_handles_empty() {
        let r = build_report(7, &[], vec![], &[], vec![]);
        assert!(r.tools.is_empty());
        assert_eq!(r.repeat_input_rate, 0.0);
        assert!(r.llm.p50_ms.is_none());
    }

    #[test]
    fn build_report_summarizes_usage() {
        let usage = vec![
            UsageRow {
                input_tokens: 10,
                output_tokens: 5,
                elapsed_ms: 100,
            },
            UsageRow {
                input_tokens: 20,
                output_tokens: 7,
                elapsed_ms: 900,
            },
        ];
        let r = build_report(
            7,
            &[],
            vec![
                StatusCount {
                    status: "completed".into(),
                    count: 9,
                },
                StatusCount {
                    status: "max_turns".into(),
                    count: 1,
                },
            ],
            &usage,
            vec![],
        );
        assert_eq!(r.llm.llm_calls, 2);
        assert_eq!(r.llm.input_tokens, 30);
        assert_eq!(r.llm.p50_ms, Some(100));
        assert_eq!(r.llm.p95_ms, Some(900));
        assert_eq!(r.turn_status[0].status, "completed");
    }

    #[test]
    fn percentile_empty_is_none() {
        assert_eq!(nearest_rank_percentile(&[], 0.5), None);
        assert_eq!(nearest_rank_percentile(&[5, 10, 15], 0.5), Some(10));
        assert_eq!(nearest_rank_percentile(&[5, 10, 15], 0.95), Some(15));
    }

    #[test]
    fn build_report_includes_gate_summary_when_present() {
        let gate_lines = vec![
            serde_json::json!({"ts":"2026-08-06T00:00:00Z","event":"guard_verdict","decision":"complete"}).to_string(),
            serde_json::json!({"ts":"2026-08-06T00:00:00Z","event":"guard_fallback","inferred_family":"web_design"}).to_string(),
            serde_json::json!({"ts":"2026-08-06T00:00:00Z","event":"grader_verdict","verdict":"refuted"}).to_string(),
        ];
        let r = build_report(7, &[], vec![], &[], gate_lines);
        let g = r.gates.as_ref().expect("gates section");
        assert_eq!(g.guard_evaluations, 1);
        assert_eq!(g.fallback_used, 1);
        assert_eq!(g.grader_refuted, 1);
        let md = render_markdown(&r);
        assert!(md.contains("## Delivery gates"));
        assert!(md.contains("escape rate"));
    }

    #[test]
    fn build_report_omits_gates_when_no_data() {
        let r = build_report(7, &[], vec![], &[], vec![]);
        assert!(r.gates.is_none());
        assert!(!render_markdown(&r).contains("Delivery gates"));
    }
}
