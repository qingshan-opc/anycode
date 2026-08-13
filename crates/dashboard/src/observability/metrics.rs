//! Project metrics and delivery readiness aggregates.

use crate::db::DashboardDb;
use crate::observability::llm_usage::EVENT_TYPE as LLM_USAGE_EVENT;
use crate::observability::project_trust::{readiness_from_inputs, readiness_score};
use crate::schema::{DeliveryReadiness, ProjectMetrics, ProjectReadinessItem};
use anyhow::Result;

const LEGACY_USAGE_EVENT: &str = "llm_response_end";

fn usage_event_filter_sql() -> String {
    format!("e.event_type IN ('{LLM_USAGE_EVENT}', '{LEGACY_USAGE_EVENT}')")
}

fn mock_model_filter_sql() -> &'static str {
    r#"
          AND LOWER(TRIM(COALESCE(s.model, ''))) != 'mock'
          AND LOWER(TRIM(COALESCE(s.model, ''))) NOT LIKE 'mock/%'
    "#
}

/// Prefer session.model, then llm_usage payload.model, else unknown.
fn resolved_usage_model_sql() -> &'static str {
    "COALESCE(NULLIF(TRIM(s.model), ''), NULLIF(TRIM(json_extract(e.payload_json, '$.model')), ''), 'unknown')"
}

/// Calendar dates for a rolling window ending today (UTC), oldest → newest.
/// Always returns exactly `days` keys so charts can render zero-filled days.
fn calendar_day_keys(days: u32) -> Vec<String> {
    use chrono::{Duration, Utc};
    let today = Utc::now().date_naive();
    let days = days.max(1) as i64;
    (0..days)
        .rev()
        .map(|offset| {
            (today - Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect()
}

/// Prefer llm_usage payload.agent_type (execute_task 行内字段 / 嵌套摄取写入), then session.agent_type.
fn resolved_usage_agent_sql() -> &'static str {
    "COALESCE(NULLIF(TRIM(json_extract(e.payload_json, '$.agent_type')), ''), NULLIF(TRIM(s.agent_type), ''), 'unknown')"
}

/// by_agent token 指标：嵌套子代理 usage（recorder 摄取子任务 output.log 写入，
/// payload.nested=true）归并到其 agent_type 下，并单列 nested 贡献。
async fn usage_by_agent(
    db: &DashboardDb,
    days: Option<u32>,
    session_id: Option<&str>,
    project_id: Option<&str>,
) -> Result<Vec<crate::schema::AgentUsageRow>> {
    use crate::schema::AgentUsageRow;
    use sqlx::Row;
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let mut sql = format!(
        r#"
        SELECT
          {resolved_agent} AS usage_agent,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CASE WHEN json_extract(e.payload_json, '$.nested') = 1
                            THEN CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER) ELSE 0 END), 0) AS nested_input_tokens,
          COALESCE(SUM(CASE WHEN json_extract(e.payload_json, '$.nested') = 1
                            THEN CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER) ELSE 0 END), 0) AS nested_output_tokens
        FROM project_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          {mock_filter}
        "#,
        resolved_agent = resolved_usage_agent_sql(),
        event_filter = event_filter,
        mock_filter = mock_filter,
    );
    let mut binds: Vec<String> = Vec::new();
    if let Some(days) = days {
        sql.push_str(" AND datetime(e.occurred_at) >= datetime('now', ?)");
        binds.push(format!("-{} days", days.clamp(1, 90)));
    }
    if let Some(sid) = session_id.filter(|s| !s.is_empty()) {
        sql.push_str(" AND e.session_id = ?");
        binds.push(sid.to_string());
    }
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        sql.push_str(" AND e.project_id = ?");
        binds.push(pid.to_string());
    }
    // 别名必须区别于 sessions.agent_type 列名：SQLite GROUP BY 同名时优先解析为表列。
    sql.push_str(" GROUP BY usage_agent ORDER BY input_tokens + output_tokens DESC LIMIT 50");
    let mut q = sqlx::query(&sql);
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(db.pool()).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let input_tokens: i64 = r.get("input_tokens");
            let output_tokens: i64 = r.get("output_tokens");
            AgentUsageRow {
                agent_type: r.get("usage_agent"),
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                nested_input_tokens: r.get("nested_input_tokens"),
                nested_output_tokens: r.get("nested_output_tokens"),
            }
        })
        .collect())
}

pub async fn global_readiness(db: &DashboardDb) -> Result<DeliveryReadiness> {
    let overview = db.overview_stats().await?;
    let blocked = overview.sessions_blocked;
    let failed_gates = overview.gates_failed;
    let running = overview.sessions_running;

    let unverified: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM artifacts WHERE trust_level IN ('unknown', 'needs_verify', 'unverified')"#,
    )
    .fetch_one(db.pool())
    .await?;

    let stale_running: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM sessions
        WHERE status = 'running'
          AND datetime(started_at) < datetime('now', '-24 hours')
        "#,
    )
    .fetch_one(db.pool())
    .await?;

    let mut project_items = Vec::new();
    let projects = db.list_projects().await?;
    for p in projects.iter().take(20) {
        if let Ok(m) = project_metrics(db, &p.id).await {
            let score = readiness_score(
                m.blocked_sessions,
                m.failed_required_gates,
                m.unverified_artifacts,
                m.stale_running_sessions,
            );
            if score < 100 {
                project_items.push(ProjectReadinessItem {
                    project_id: p.id.clone(),
                    project_name: p.name.clone(),
                    readiness_score: score,
                    blocked_sessions: m.blocked_sessions,
                    failed_gates: m.failed_required_gates,
                    unverified_artifacts: m.unverified_artifacts,
                });
            }
        }
    }
    project_items.sort_by_key(|i| i.readiness_score);

    let status = if blocked > 0 || failed_gates > 0 {
        "warn"
    } else if stale_running > 0 || unverified > 0 {
        "warn"
    } else {
        "ok"
    };

    Ok(DeliveryReadiness {
        status: status.into(),
        blocked_sessions: blocked,
        failed_required_gates: failed_gates,
        unverified_artifacts: unverified,
        stale_running_sessions: stale_running,
        running_sessions: running,
        projects: project_items,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn project_metrics(db: &DashboardDb, project_id: &str) -> Result<ProjectMetrics> {
    let trust_inputs = db.fetch_project_trust_inputs(project_id).await?;
    let sessions_total = trust_inputs.sessions_total;
    let blocked_sessions = trust_inputs.blocked_sessions;
    let failed_required_gates = trust_inputs.failed_required_gates;
    let unverified_artifacts = trust_inputs.unverified_artifacts;
    let stale_running_sessions = trust_inputs.stale_running_sessions;

    let sessions_completed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sessions WHERE project_id = ? AND status IN ('completed', 'done')",
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await?;

    let events_7d: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM project_events WHERE project_id = ? AND datetime(occurred_at) >= datetime('now', '-7 days')"#,
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await?;

    let gates_passed: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM gates WHERE project_id = ? AND status = 'passed'"#,
    )
    .bind(project_id)
    .fetch_one(db.pool())
    .await?;

    let gates_total = trust_inputs.gates_total;

    let gate_pass_rate = if gates_total > 0 {
        (gates_passed as f64) / (gates_total as f64)
    } else {
        1.0
    };

    let success_rate = if sessions_total > 0 {
        (sessions_completed as f64) / (sessions_total as f64)
    } else {
        0.0
    };

    let readiness_score = readiness_from_inputs(&trust_inputs);

    Ok(ProjectMetrics {
        project_id: project_id.into(),
        sessions_total,
        sessions_completed,
        blocked_sessions,
        failed_required_gates,
        unverified_artifacts,
        stale_running_sessions,
        events_7d,
        gate_pass_rate,
        session_success_rate: success_rate,
        readiness_score,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Rolling daily aggregates for the home timeline chart (computed live from SQLite).
pub async fn global_timeline(
    db: &DashboardDb,
    days: u32,
) -> Result<crate::schema::GlobalTimelineMetrics> {
    use crate::schema::TimelineMetricPoint;
    let days = days.clamp(1, 30);
    let span = format!("-{days} days");

    let session_rows = sqlx::query(
        r#"
        SELECT date(started_at) AS d, COUNT(*) AS c
        FROM sessions
        WHERE datetime(started_at) >= datetime('now', ?)
        GROUP BY date(started_at)
        "#,
    )
    .bind(span.clone())
    .fetch_all(db.pool())
    .await?;

    let event_rows = sqlx::query(
        r#"
        SELECT date(occurred_at) AS d, COUNT(*) AS c
        FROM project_events
        WHERE datetime(occurred_at) >= datetime('now', ?)
        GROUP BY date(occurred_at)
        "#,
    )
    .bind(span.clone())
    .fetch_all(db.pool())
    .await?;

    let gate_rows = sqlx::query(
        r#"
        SELECT date(COALESCE(ended_at, started_at)) AS d, COUNT(*) AS c
        FROM gates
        WHERE status = 'failed'
          AND datetime(COALESCE(ended_at, started_at)) >= datetime('now', ?)
        GROUP BY date(COALESCE(ended_at, started_at))
        "#,
    )
    .bind(span)
    .fetch_all(db.pool())
    .await?;

    use sqlx::Row;
    use std::collections::BTreeMap;

    let mut map: BTreeMap<String, TimelineMetricPoint> = BTreeMap::new();

    for r in session_rows {
        let d: String = r.get("d");
        map.entry(d.clone())
            .or_insert(TimelineMetricPoint {
                date: d,
                sessions_count: 0,
                events_count: 0,
                gates_failed: 0,
            })
            .sessions_count = r.get::<i64, _>("c");
    }
    for r in event_rows {
        let d: String = r.get("d");
        map.entry(d.clone())
            .or_insert(TimelineMetricPoint {
                date: d,
                sessions_count: 0,
                events_count: 0,
                gates_failed: 0,
            })
            .events_count = r.get::<i64, _>("c");
    }
    for r in gate_rows {
        let d: String = r.get("d");
        map.entry(d.clone())
            .or_insert(TimelineMetricPoint {
                date: d,
                sessions_count: 0,
                events_count: 0,
                gates_failed: 0,
            })
            .gates_failed = r.get::<i64, _>("c");
    }

    let points: Vec<TimelineMetricPoint> = calendar_day_keys(days)
        .into_iter()
        .map(|date| {
            map.get(&date).cloned().unwrap_or(TimelineMetricPoint {
                date,
                sessions_count: 0,
                events_count: 0,
                gates_failed: 0,
            })
        })
        .collect();
    let trust_trend_pct = if points.iter().all(|p| p.sessions_count == 0) {
        0.0
    } else if points.len() >= 2 {
        let first = points.first().map(|p| p.sessions_count).unwrap_or(0).max(1) as f64;
        let last = points.last().map(|p| p.sessions_count).unwrap_or(0) as f64;
        ((last - first) / first) * 100.0
    } else {
        0.0
    };

    Ok(crate::schema::GlobalTimelineMetrics {
        days,
        points,
        trust_trend_pct,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Aggregate LLM token usage from `llm_response_end` event payloads.
pub async fn global_token_usage(
    db: &DashboardDb,
    days: u32,
) -> Result<crate::schema::TokenUsageStats> {
    Ok(global_token_usage_detail(db, days).await?.usage)
}

/// Global usage with per-model breakdown (V3).
pub async fn global_token_usage_detail(
    db: &DashboardDb,
    days: u32,
) -> Result<crate::schema::TokenUsageDetail> {
    usage_detail(db, days, None).await
}

/// Per-project usage with model breakdown (V3).
pub async fn project_token_usage_detail(
    db: &DashboardDb,
    project_id: &str,
    days: u32,
) -> Result<crate::schema::TokenUsageDetail> {
    usage_detail(db, days, Some(project_id)).await
}

async fn usage_detail(
    db: &DashboardDb,
    days: u32,
    project_id: Option<&str>,
) -> Result<crate::schema::TokenUsageDetail> {
    use crate::schema::{TokenUsageDetail, TokenUsageStats};
    let days = days.clamp(1, 90);
    let by_model = usage_by_model(db, days, project_id).await?;
    let by_project = usage_by_project(db, days, project_id).await?;
    let by_day = usage_by_day(db, days, project_id).await?;
    let by_agent = usage_by_agent(db, Some(days), None, project_id).await?;
    let llm_calls: i64 = by_model.iter().map(|r| r.llm_calls).sum();
    let input_tokens: i64 = by_model.iter().map(|r| r.input_tokens).sum();
    let output_tokens: i64 = by_model.iter().map(|r| r.output_tokens).sum();
    let estimated_cost_cny: f64 = by_model.iter().map(|r| r.estimated_cost_cny).sum();
    let cache_read_tokens: i64 = by_model.iter().map(|r| r.cache_read_tokens).sum();
    let cache_creation_tokens: i64 = by_model.iter().map(|r| r.cache_creation_tokens).sum();
    Ok(TokenUsageDetail {
        usage: TokenUsageStats {
            days,
            llm_calls,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            estimated_cost_cny,
            cache_read_tokens,
            cache_creation_tokens,
            generated_at: chrono::Utc::now().to_rfc3339(),
        },
        by_model,
        by_project,
        by_day,
        by_agent,
    })
}

async fn usage_by_model(
    db: &DashboardDb,
    days: u32,
    project_id: Option<&str>,
) -> Result<Vec<crate::schema::ModelUsageRow>> {
    use crate::schema::ModelUsageRow;
    use sqlx::Row;
    let days = days.clamp(1, 90);
    let span = format!("-{days} days");
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let mut sql = format!(
        r#"
        SELECT
          {resolved_model} AS model,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_read_tokens') AS INTEGER)), 0) AS cache_read_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_creation_tokens') AS INTEGER)), 0) AS cache_creation_tokens
        FROM project_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          AND datetime(e.occurred_at) >= datetime('now', ?)
          {mock_filter}
        "#,
        resolved_model = resolved_usage_model_sql(),
        event_filter = event_filter,
        mock_filter = mock_filter,
    );
    if project_id.filter(|s| !s.is_empty()).is_some() {
        sql.push_str(" AND e.project_id = ?");
    }
    sql.push_str(" GROUP BY model ORDER BY input_tokens + output_tokens DESC LIMIT 50");
    let mut q = sqlx::query(&sql).bind(&span);
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        q = q.bind(pid);
    }
    let rows = q.fetch_all(db.pool()).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let model: String = r.get("model");
            let input_tokens: i64 = r.get("input_tokens");
            let output_tokens: i64 = r.get("output_tokens");
            let cache_read_tokens: i64 = r.get("cache_read_tokens");
            ModelUsageRow {
                provider: infer_provider(&model).into(),
                model: model.clone(),
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                estimated_cost_cny: estimate_model_cost_with_cache_cny(
                    &model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                ),
                cache_read_tokens,
                cache_creation_tokens: r.get("cache_creation_tokens"),
            }
        })
        .collect())
}

async fn usage_by_project(
    db: &DashboardDb,
    days: u32,
    project_id: Option<&str>,
) -> Result<Vec<crate::schema::ProjectUsageRow>> {
    use crate::schema::ProjectUsageRow;
    use sqlx::Row;
    let days = days.clamp(1, 90);
    let span = format!("-{days} days");
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let mut sql = format!(
        r#"
        SELECT
          e.project_id,
          COALESCE(p.name, e.project_id) AS project_name,
          COALESCE(p.root_path, '') AS root_path,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_read_tokens') AS INTEGER)), 0) AS cache_read_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_creation_tokens') AS INTEGER)), 0) AS cache_creation_tokens
        FROM project_events e
        LEFT JOIN projects p ON p.id = e.project_id
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          AND datetime(e.occurred_at) >= datetime('now', ?)
          {mock_filter}
        "#,
    );
    if project_id.filter(|s| !s.is_empty()).is_some() {
        sql.push_str(" AND e.project_id = ?");
    }
    sql.push_str(" GROUP BY e.project_id ORDER BY input_tokens + output_tokens DESC LIMIT 50");
    let mut q = sqlx::query(&sql).bind(&span);
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        q = q.bind(pid);
    }
    let rows = q.fetch_all(db.pool()).await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let input_tokens: i64 = r.get("input_tokens");
            let output_tokens: i64 = r.get("output_tokens");
            let cache_read_tokens: i64 = r.get("cache_read_tokens");
            let model = "unknown";
            ProjectUsageRow {
                project_id: r.get("project_id"),
                project_name: r.get("project_name"),
                root_path: r.get("root_path"),
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                estimated_cost_cny: estimate_model_cost_with_cache_cny(
                    model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                ),
                cache_read_tokens,
                cache_creation_tokens: r.get("cache_creation_tokens"),
            }
        })
        .collect())
}

async fn usage_by_day(
    db: &DashboardDb,
    days: u32,
    project_id: Option<&str>,
) -> Result<Vec<crate::schema::TokenTimelinePoint>> {
    use crate::schema::TokenTimelinePoint;
    use sqlx::Row;
    let days = days.clamp(1, 90);
    let span = format!("-{days} days");
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let mut sql = format!(
        r#"
        SELECT
          date(e.occurred_at) AS d,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_read_tokens') AS INTEGER)), 0) AS cache_read_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_creation_tokens') AS INTEGER)), 0) AS cache_creation_tokens
        FROM project_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          AND datetime(e.occurred_at) >= datetime('now', ?)
          {mock_filter}
        "#,
    );
    if project_id.filter(|s| !s.is_empty()).is_some() {
        sql.push_str(" AND e.project_id = ?");
    }
    sql.push_str(" GROUP BY date(e.occurred_at) ORDER BY d ASC");
    let mut q = sqlx::query(&sql).bind(&span);
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        q = q.bind(pid);
    }
    let rows = q.fetch_all(db.pool()).await?;
    let mut by_date = std::collections::BTreeMap::<String, TokenTimelinePoint>::new();
    for r in rows {
        let input_tokens: i64 = r.get("input_tokens");
        let output_tokens: i64 = r.get("output_tokens");
        let cache_read_tokens: i64 = r.get("cache_read_tokens");
        let date: String = r.get("d");
        by_date.insert(
            date.clone(),
            TokenTimelinePoint {
                date,
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                estimated_cost_cny: estimate_model_cost_with_cache_cny(
                    "unknown",
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                ),
                cache_read_tokens,
                cache_creation_tokens: r.get("cache_creation_tokens"),
            },
        );
    }
    Ok(calendar_day_keys(days)
        .into_iter()
        .map(|date| {
            by_date.remove(&date).unwrap_or(TokenTimelinePoint {
                date,
                llm_calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                estimated_cost_cny: 0.0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            })
        })
        .collect())
}

/// Saved-hours KPI: compares completed session wall time vs manual baseline (V3).
pub async fn saved_hours_kpi(db: &DashboardDb, days: u32) -> Result<crate::schema::SavedHoursKpi> {
    use crate::schema::SavedHoursKpi;
    use sqlx::Row;
    let days = days.clamp(1, 90);
    let span = format!("-{days} days");
    let row = sqlx::query(
        r#"
        SELECT
          COUNT(*) AS sessions_completed,
          CAST(COALESCE(SUM(
            MAX(0.0, (julianday(COALESCE(NULLIF(TRIM(ended_at), ''), datetime('now')))
              - julianday(started_at)) * 24.0)
          ), 0) AS REAL) AS automation_hours
        FROM sessions
        WHERE status IN ('completed', 'done')
          AND started_at IS NOT NULL AND TRIM(started_at) != ''
          AND datetime(started_at) >= datetime('now', ?)
        "#,
    )
    .bind(&span)
    .fetch_one(db.pool())
    .await?;
    let sessions_completed: i64 = row.get("sessions_completed");
    let automation_hours: f64 = row.get::<f64, _>("automation_hours");
    let baseline_hours_per_session = baseline_session_hours();
    let estimated_manual_hours = sessions_completed as f64 * baseline_hours_per_session;
    let estimated_saved_hours = (estimated_manual_hours - automation_hours).max(0.0);
    let hourly_rate_cny = hourly_rate_cny();
    Ok(SavedHoursKpi {
        days,
        sessions_completed,
        automation_hours,
        baseline_hours_per_session,
        estimated_manual_hours,
        estimated_saved_hours,
        hourly_rate_cny,
        estimated_value_cny: estimated_saved_hours * hourly_rate_cny,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[must_use]
pub fn infer_provider(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") || m.starts_with("anthropic") {
        "anthropic"
    } else if m.contains("gpt")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.contains("openai")
    {
        "openai"
    } else if m.contains("gemini") || m.contains("google") {
        "google"
    } else if m.contains("deepseek") {
        "deepseek"
    } else if m.contains("glm") || m.contains("zhipu") {
        "z.ai"
    } else if m.contains("qwen") || m.contains("dashscope") {
        "alibaba"
    } else if m.contains("llama") || m.contains("meta") {
        "meta"
    } else if m == "unknown" || m.is_empty() {
        "unknown"
    } else {
        "other"
    }
}

fn model_token_rates_cny(model: &str) -> (f64, f64) {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        (108.0, 540.0)
    } else if m.contains("sonnet") || m.contains("claude") {
        (21.6, 108.0)
    } else if m.contains("haiku") {
        (1.8, 9.0)
    } else if m.contains("gpt-4o-mini") || m.contains("mini") {
        (1.08, 4.32)
    } else if m.contains("gpt-4") || m.starts_with("o1") || m.starts_with("o3") {
        (18.0, 72.0)
    } else if m.contains("gemini") {
        (9.0, 36.0)
    } else if m.contains("deepseek") {
        (1.944, 7.92)
    } else {
        (
            std::env::var("ANYCODE_DASHBOARD_INPUT_CNY_PER_M")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(21.6),
            std::env::var("ANYCODE_DASHBOARD_OUTPUT_CNY_PER_M")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(108.0),
        )
    }
}

fn estimate_model_cost_cny(model: &str, input_tokens: i64, output_tokens: i64) -> f64 {
    let (input_rate, output_rate) = model_token_rates_cny(model);
    (input_tokens as f64 / 1_000_000.0) * input_rate
        + (output_tokens as f64 / 1_000_000.0) * output_rate
}

/// Fraction of the list input price charged for a cache-hit token (DeepSeek
/// context-cache style discount; override with `ANYCODE_DASHBOARD_CACHE_HIT_RATIO`).
fn cache_hit_price_ratio() -> f64 {
    std::env::var("ANYCODE_DASHBOARD_CACHE_HIT_RATIO")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|r| (0.0..=1.0).contains(r))
        .unwrap_or(0.1)
}

/// Effective cost when the provider reports cache hits: hit tokens are billed at
/// the discounted ratio, the remaining input at list price. `cache_read_tokens`
/// is a subset of `input_tokens` on OpenAI-compatible usage schemas.
fn estimate_model_cost_with_cache_cny(
    model: &str,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
) -> f64 {
    let (input_rate, output_rate) = model_token_rates_cny(model);
    let cached = cache_read_tokens.clamp(0, input_tokens);
    let uncached = input_tokens - cached;
    (uncached as f64 / 1_000_000.0) * input_rate
        + (cached as f64 / 1_000_000.0) * input_rate * cache_hit_price_ratio()
        + (output_tokens as f64 / 1_000_000.0) * output_rate
}

fn baseline_session_hours() -> f64 {
    std::env::var("ANYCODE_DASHBOARD_BASELINE_SESSION_MINUTES")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(45.0)
        / 60.0
}

fn hourly_rate_cny() -> f64 {
    std::env::var("ANYCODE_DASHBOARD_HOURLY_RATE_CNY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(360.0)
}

/// Rough CNY estimate, configurable with `*_CNY_PER_M` environment variables.
fn estimate_token_cost_cny(input_tokens: i64, output_tokens: i64) -> f64 {
    estimate_model_cost_cny("unknown", input_tokens, output_tokens)
}

/// Per-project LLM token usage.
pub async fn project_token_usage(
    db: &DashboardDb,
    project_id: &str,
    days: u32,
) -> Result<crate::schema::TokenUsageStats> {
    Ok(project_token_usage_detail(db, project_id, days)
        .await?
        .usage)
}

/// Per-session LLM token usage (all events for session).
pub async fn session_token_usage_detail(
    db: &DashboardDb,
    session_id: &str,
) -> Result<crate::schema::TokenUsageDetail> {
    use crate::schema::{TokenUsageDetail, TokenUsageStats};
    let by_model = usage_by_model_session(db, session_id).await?;
    let by_day = usage_by_day_session(db, session_id).await?;
    let by_agent = usage_by_agent(db, None, Some(session_id), None).await?;
    let llm_calls: i64 = by_model.iter().map(|r| r.llm_calls).sum();
    let input_tokens: i64 = by_model.iter().map(|r| r.input_tokens).sum();
    let output_tokens: i64 = by_model.iter().map(|r| r.output_tokens).sum();
    let estimated_cost_cny: f64 = by_model.iter().map(|r| r.estimated_cost_cny).sum();
    let cache_read_tokens: i64 = by_model.iter().map(|r| r.cache_read_tokens).sum();
    let cache_creation_tokens: i64 = by_model.iter().map(|r| r.cache_creation_tokens).sum();
    Ok(TokenUsageDetail {
        usage: TokenUsageStats {
            days: 0,
            llm_calls,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
            estimated_cost_cny,
            cache_read_tokens,
            cache_creation_tokens,
            generated_at: chrono::Utc::now().to_rfc3339(),
        },
        by_model,
        by_project: Vec::new(),
        by_day,
        by_agent,
    })
}

async fn usage_by_model_session(
    db: &DashboardDb,
    session_id: &str,
) -> Result<Vec<crate::schema::ModelUsageRow>> {
    use crate::schema::ModelUsageRow;
    use sqlx::Row;
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let sql = format!(
        r#"
        SELECT
          {resolved_model} AS model,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_read_tokens') AS INTEGER)), 0) AS cache_read_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_creation_tokens') AS INTEGER)), 0) AS cache_creation_tokens
        FROM project_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          AND e.session_id = ?
          {mock_filter}
        GROUP BY model ORDER BY input_tokens + output_tokens DESC LIMIT 20
        "#,
        resolved_model = resolved_usage_model_sql(),
        event_filter = event_filter,
        mock_filter = mock_filter,
    );
    let rows = sqlx::query(&sql)
        .bind(session_id)
        .fetch_all(db.pool())
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let model: String = r.get("model");
            let input_tokens: i64 = r.get("input_tokens");
            let output_tokens: i64 = r.get("output_tokens");
            let cache_read_tokens: i64 = r.get("cache_read_tokens");
            ModelUsageRow {
                provider: infer_provider(&model).into(),
                model: model.clone(),
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                estimated_cost_cny: estimate_model_cost_with_cache_cny(
                    &model,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                ),
                cache_read_tokens,
                cache_creation_tokens: r.get("cache_creation_tokens"),
            }
        })
        .collect())
}

async fn usage_by_day_session(
    db: &DashboardDb,
    session_id: &str,
) -> Result<Vec<crate::schema::TokenTimelinePoint>> {
    use crate::schema::TokenTimelinePoint;
    use sqlx::Row;
    let event_filter = usage_event_filter_sql();
    let mock_filter = mock_model_filter_sql();
    let sql = format!(
        r#"
        SELECT
          date(e.occurred_at) AS d,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_read_tokens') AS INTEGER)), 0) AS cache_read_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.cache_creation_tokens') AS INTEGER)), 0) AS cache_creation_tokens
        FROM project_events e
        LEFT JOIN sessions s ON s.id = e.session_id
        WHERE {event_filter}
          AND e.session_id = ?
          {mock_filter}
        GROUP BY date(e.occurred_at) ORDER BY d ASC
        "#,
    );
    let rows = sqlx::query(&sql)
        .bind(session_id)
        .fetch_all(db.pool())
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let input_tokens: i64 = r.get("input_tokens");
            let output_tokens: i64 = r.get("output_tokens");
            let cache_read_tokens: i64 = r.get("cache_read_tokens");
            TokenTimelinePoint {
                date: r.get("d"),
                llm_calls: r.get("llm_calls"),
                input_tokens,
                output_tokens,
                total_tokens: input_tokens + output_tokens,
                estimated_cost_cny: estimate_model_cost_with_cache_cny(
                    "unknown",
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                ),
                cache_read_tokens,
                cache_creation_tokens: r.get("cache_creation_tokens"),
            }
        })
        .collect())
}

/// CSV export for global or per-project usage (by project row).
pub async fn usage_export_csv(
    db: &DashboardDb,
    days: u32,
    project_id: Option<&str>,
) -> Result<String> {
    use sqlx::Row;
    let days = days.clamp(1, 90);
    let span = format!("-{days} days");
    let mut out = String::from("project_id,project_name,llm_calls,input_tokens,output_tokens,total_tokens,estimated_cost_cny,currency\n");
    if let Some(pid) = project_id.filter(|s| !s.is_empty()) {
        let usage = project_token_usage(db, pid, days).await?;
        let name = db
            .get_project(pid)
            .await?
            .map(|p| p.name)
            .unwrap_or_else(|| pid.to_string());
        out.push_str(&format!(
            "{},{},{},{},{},{},{:.4},CNY\n",
            pid,
            csv_escape(&name),
            usage.llm_calls,
            usage.input_tokens,
            usage.output_tokens,
            usage.total_tokens,
            usage.estimated_cost_cny
        ));
        return Ok(out);
    }
    let rows = sqlx::query(&format!(
        r#"
        SELECT
          e.project_id,
          p.name AS project_name,
          COUNT(*) AS llm_calls,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.input_tokens') AS INTEGER)), 0) AS input_tokens,
          COALESCE(SUM(CAST(json_extract(e.payload_json, '$.output_tokens') AS INTEGER)), 0) AS output_tokens
        FROM project_events e
        LEFT JOIN projects p ON p.id = e.project_id
        WHERE {}
          AND datetime(e.occurred_at) >= datetime('now', ?)
        GROUP BY e.project_id
        ORDER BY input_tokens + output_tokens DESC
        "#,
        usage_event_filter_sql()
    ))
    .bind(span)
    .fetch_all(db.pool())
    .await?;
    for r in rows {
        let input_tokens: i64 = r.get("input_tokens");
        let output_tokens: i64 = r.get("output_tokens");
        let total = input_tokens + output_tokens;
        let cost = estimate_token_cost_cny(input_tokens, output_tokens);
        out.push_str(&format!(
            "{},{},{},{},{},{},{:.4},CNY\n",
            r.get::<String, _>("project_id"),
            csv_escape(
                &r.get::<Option<String>, _>("project_name")
                    .unwrap_or_default()
            ),
            r.get::<i64, _>("llm_calls"),
            input_tokens,
            output_tokens,
            total,
            cost
        ));
    }
    Ok(out)
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn blocked_alert_threshold() -> Option<i64> {
    std::env::var("ANYCODE_DASHBOARD_BLOCKED_ALERT_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&t| t >= 0)
}

/// Emit at most one `blocked_threshold_exceeded` notification per hour when blocked sessions exceed threshold.
pub async fn maybe_emit_blocked_threshold_alert(db: &DashboardDb) -> Result<()> {
    let Some(threshold) = blocked_alert_threshold() else {
        return Ok(());
    };
    let stats = db.overview_stats().await?;
    if stats.sessions_blocked <= threshold {
        return Ok(());
    }
    let recent: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*) FROM auth_events
        WHERE event_type = 'blocked_threshold_exceeded'
          AND datetime(created_at) >= datetime('now', '-1 hour')
        "#,
    )
    .fetch_one(db.pool())
    .await?;
    if recent > 0 {
        return Ok(());
    }
    let detail = serde_json::json!({
        "title": "Blocked sessions exceeded threshold",
        "blocked_sessions": stats.sessions_blocked,
        "threshold": threshold,
    });
    crate::audit::record_audit(
        db,
        crate::audit::AuditEventInput {
            project_id: None,
            session_id: None,
            action: "blocked_threshold_exceeded".into(),
            risk: "medium".into(),
            detail: detail.clone(),
        },
    )
    .await?;
    crate::notifications::emit_local_log(db, None, None, "blocked_threshold_exceeded", detail)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CreateSessionRequest, UpsertProjectRequest};
    use tempfile::tempdir;

    #[test]
    fn token_cost_estimate() {
        assert!((super::estimate_token_cost_cny(1_000_000, 0) - 21.6).abs() < 0.01);
        assert!((super::estimate_token_cost_cny(0, 1_000_000) - 108.0).abs() < 0.01);
    }

    #[test]
    fn infer_provider_variants() {
        assert_eq!(infer_provider("claude-sonnet-4"), "anthropic");
        assert_eq!(infer_provider("gpt-4o"), "openai");
        assert_eq!(infer_provider("gemini-2.0-flash"), "google");
        assert_eq!(infer_provider("unknown"), "unknown");
        assert_eq!(infer_provider(""), "unknown");
    }

    #[test]
    fn model_cost_sonnet() {
        let cost = estimate_model_cost_cny("claude-sonnet-4", 1_000_000, 1_000_000);
        assert!((cost - 129.6).abs() < 0.01);
    }

    #[test]
    fn deepseek_cache_hit_discounts_effective_cost() {
        // DeepSeek list rates: 1.944 / 7.92 CNY per M. 800k of the 1M input
        // tokens are cache hits billed at the 0.1 ratio default.
        let cost = estimate_model_cost_with_cache_cny("deepseek-v4-flash", 1_000_000, 0, 800_000);
        let expected = 0.2 * 1.944 + 0.8 * 1.944 * 0.1;
        assert!(
            (cost - expected).abs() < 0.001,
            "cost={cost} expected={expected}"
        );
        // No cache → identical to the plain estimate.
        let plain = estimate_model_cost_cny("deepseek-v4-flash", 1_000_000, 1_000_000);
        let with_zero =
            estimate_model_cost_with_cache_cny("deepseek-v4-flash", 1_000_000, 1_000_000, 0);
        assert!((plain - with_zero).abs() < 0.0001);
        // Cache claims exceeding input are clamped, never negative.
        let clamped = estimate_model_cost_with_cache_cny("deepseek-v4-flash", 100, 0, 500);
        assert!(clamped >= 0.0);
    }

    #[tokio::test]
    async fn usage_by_agent_groups_payload_agent_type_and_marks_nested() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("ba.db")).await.unwrap();
        let project = db
            .upsert_project(UpsertProjectRequest {
                root_path: "/tmp/ba".into(),
                name: Some("BA".into()),
                description: None,
                create_root: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let session = db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "run".into(),
                task_id: Some("parent-task".into()),
                title: "t".into(),
                prompt_preview: None,
                agent_type: Some("general-purpose".into()),
                model: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let insert_usage = |task_id: &str, payload: serde_json::Value| {
            let db = db.clone();
            let pid = project.id.clone();
            let sid = session.id.clone();
            let tid = task_id.to_string();
            async move {
                db.insert_event(crate::schema::InsertEventRequest {
                    project_id: pid,
                    session_id: Some(sid),
                    task_id: Some(tid),
                    agent_id: None,
                    event_type: "llm_usage".into(),
                    severity: Some("info".into()),
                    title: "u".into(),
                    body: None,
                    payload: Some(payload),
                })
                .await
                .unwrap();
            }
        };
        // 父任务 usage：无 payload.agent_type → session.agent_type 兜底
        insert_usage(
            "parent-task",
            serde_json::json!({"turn": "1", "input_tokens": 1000, "output_tokens": 200}),
        )
        .await;
        // 嵌套子代理 usage：payload.agent_type=explore + nested 标记
        insert_usage(
            "nested-task",
            serde_json::json!({"turn": "1", "input_tokens": 500, "output_tokens": 100, "agent_type": "explore", "nested": true}),
        )
        .await;
        insert_usage(
            "nested-task",
            serde_json::json!({"turn": "2", "input_tokens": 300, "output_tokens": 50, "agent_type": "explore", "nested": true}),
        )
        .await;

        let detail = global_token_usage_detail(&db, 7).await.unwrap();
        let explore = detail
            .by_agent
            .iter()
            .find(|r| r.agent_type == "explore")
            .expect("explore row");
        assert_eq!(explore.llm_calls, 2);
        assert_eq!(explore.input_tokens, 800);
        assert_eq!(explore.output_tokens, 150);
        assert_eq!(explore.nested_input_tokens, 800);
        assert_eq!(explore.nested_output_tokens, 150);
        let parent = detail
            .by_agent
            .iter()
            .find(|r| r.agent_type == "general-purpose")
            .expect("parent row via session fallback");
        assert_eq!(parent.llm_calls, 1);
        assert_eq!(parent.nested_input_tokens, 0);
        // 总计含嵌套（token 指标不再缺失子代理消耗）
        assert_eq!(detail.usage.input_tokens, 1800);
    }

    #[tokio::test]
    async fn readiness_empty_db_ok() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("m.db")).await.unwrap();
        let r = global_readiness(&db).await.unwrap();
        assert_eq!(r.status, "ok");
    }

    #[tokio::test]
    async fn project_metrics_after_gate_failure() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("g.db")).await.unwrap();
        let project = db
            .upsert_project(UpsertProjectRequest {
                root_path: "/tmp/m".into(),
                name: Some("M".into()),
                description: None,
                create_root: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let session = db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "run".into(),
                task_id: None,
                title: "t".into(),
                prompt_preview: None,
                agent_type: None,
                model: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        db.upsert_gate(
            &project.id,
            &session.id,
            "test",
            "test",
            "failed",
            true,
            "fail",
        )
        .await
        .unwrap();
        let m = project_metrics(&db, &project.id).await.unwrap();
        assert_eq!(m.failed_required_gates, 1);
        assert!(m.readiness_score < 100);
    }

    #[tokio::test]
    async fn global_timeline_empty_db() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("tl.db")).await.unwrap();
        let tl = global_timeline(&db, 7).await.unwrap();
        assert_eq!(tl.days, 7);
        assert_eq!(tl.points.len(), 7, "empty window still returns 7 zero days");
        assert!(tl
            .points
            .iter()
            .all(|p| { p.sessions_count == 0 && p.events_count == 0 && p.gates_failed == 0 }));
        assert_eq!(tl.trust_trend_pct, 0.0);
    }

    #[tokio::test]
    async fn usage_by_day_fills_zero_days() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("usage-days.db"))
            .await
            .unwrap();
        let detail = global_token_usage_detail(&db, 7).await.unwrap();
        assert_eq!(detail.by_day.len(), 7);
        assert!(detail
            .by_day
            .iter()
            .all(|p| { p.llm_calls == 0 && p.total_tokens == 0 && p.estimated_cost_cny == 0.0 }));
        let keys = calendar_day_keys(7);
        assert_eq!(
            detail
                .by_day
                .iter()
                .map(|p| p.date.as_str())
                .collect::<Vec<_>>(),
            keys.iter().map(String::as_str).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn usage_by_model_prefers_session_then_payload() {
        use crate::schema::InsertEventRequest;
        use serde_json::json;

        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("usage-model.db"))
            .await
            .unwrap();
        let project = db
            .upsert_project(UpsertProjectRequest {
                root_path: "/tmp/usage-model".into(),
                name: Some("UM".into()),
                description: None,
                create_root: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let session_empty = db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "run".into(),
                task_id: None,
                title: "empty-model".into(),
                prompt_preview: None,
                agent_type: None,
                model: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        let session_named = db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "run".into(),
                task_id: None,
                title: "named-model".into(),
                prompt_preview: None,
                agent_type: None,
                model: Some("gpt-4o".into()),
                metadata_json: None,
            })
            .await
            .unwrap();

        db.insert_event(InsertEventRequest {
            project_id: project.id.clone(),
            session_id: Some(session_empty.id),
            task_id: None,
            agent_id: None,
            event_type: LLM_USAGE_EVENT.into(),
            severity: Some("info".into()),
            title: "payload model".into(),
            body: None,
            payload: Some(json!({
                "turn": "1",
                "input_tokens": 10,
                "output_tokens": 2,
                "model": "claude-sonnet-4",
            })),
        })
        .await
        .unwrap();
        db.insert_event(InsertEventRequest {
            project_id: project.id,
            session_id: Some(session_named.id),
            task_id: None,
            agent_id: None,
            event_type: LLM_USAGE_EVENT.into(),
            severity: Some("info".into()),
            title: "session model wins".into(),
            body: None,
            payload: Some(json!({
                "turn": "1",
                "input_tokens": 5,
                "output_tokens": 1,
                "model": "claude-sonnet-4",
            })),
        })
        .await
        .unwrap();

        let detail = global_token_usage_detail(&db, 7).await.unwrap();
        let models: Vec<_> = detail.by_model.iter().map(|r| r.model.as_str()).collect();
        assert!(models.contains(&"claude-sonnet-4"));
        assert!(models.contains(&"gpt-4o"));
        let claude = detail
            .by_model
            .iter()
            .find(|r| r.model == "claude-sonnet-4")
            .unwrap();
        assert_eq!(claude.provider, "anthropic");
        assert_eq!(claude.llm_calls, 1);
        let gpt = detail
            .by_model
            .iter()
            .find(|r| r.model == "gpt-4o")
            .unwrap();
        assert_eq!(gpt.provider, "openai");
        assert_eq!(gpt.llm_calls, 1);
    }

    #[tokio::test]
    async fn blocked_threshold_alert_when_exceeded() {
        let dir = tempdir().unwrap();
        let db = DashboardDb::open(dir.path().join("blocked.db"))
            .await
            .unwrap();
        std::env::set_var("ANYCODE_DASHBOARD_BLOCKED_ALERT_THRESHOLD", "0");
        let project = db
            .upsert_project(UpsertProjectRequest {
                root_path: "/tmp/blocked".into(),
                name: Some("B".into()),
                description: None,
                create_root: None,
                ..Default::default()
            })
            .await
            .unwrap();
        let session = db
            .create_session(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: "run".into(),
                task_id: Some("t1".into()),
                title: "blocked".into(),
                prompt_preview: None,
                agent_type: None,
                model: None,
                metadata_json: None,
            })
            .await
            .unwrap();
        db.upsert_gate(
            &project.id,
            &session.id,
            "test gate",
            "echo fail",
            "failed",
            true,
            "failed",
        )
        .await
        .unwrap();
        db.refresh_session_trusted_status(&session.id)
            .await
            .unwrap();
        maybe_emit_blocked_threshold_alert(&db).await.unwrap();
        let recent = crate::audit::list_recent_notifications(&db, 5)
            .await
            .unwrap();
        assert!(recent
            .iter()
            .any(|n| n.action == "blocked_threshold_exceeded"));
        std::env::remove_var("ANYCODE_DASHBOARD_BLOCKED_ALERT_THRESHOLD");
    }
}
