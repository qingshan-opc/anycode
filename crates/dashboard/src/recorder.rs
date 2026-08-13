//! Bridge `output.log` tailing to SQLite sessions and events.

use crate::db::DashboardDb;
use crate::log_parser::{parse_line, task_end_status};
use crate::notify;
use crate::observability::event_tier::is_index_event_type;
use crate::observability::llm_usage::{self, EVENT_TYPE as LLM_USAGE_EVENT};
use crate::schema::{CreateSessionRequest, InsertEventRequest, ProjectEvent, UpsertProjectRequest};
use crate::server::default_db_path;
use anycode_core::{DiskTaskOutput, GoalProgress, Task, TaskId};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunSessionKind {
    Run,
    Goal,
    Workflow,
    Repl,
    Cron,
}

impl RunSessionKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Goal => "goal",
            Self::Workflow => "workflow",
            Self::Repl => "repl",
            Self::Cron => "cron",
        }
    }
}

/// Records task runs into `projects.db` (best-effort; never fails the agent loop).
const ARTIFACT_TOOLS: &[&str] = &["FileWrite", "Edit", "NotebookEdit", "Bash"];

#[derive(Clone)]
pub struct DashboardRecorder {
    db: Arc<DashboardDb>,
    session_id: String,
    project_id: String,
    project_root: PathBuf,
    task_id: String,
    log_offset: u64,
    pending_tool_name: Option<String>,
    pending_tool_json: Option<String>,
    pending_artifact_rel: Option<String>,
    last_model: Option<String>,
    started_at: SystemTime,
    /// 最近一次 ingest 用的磁盘句柄（嵌套子代理 output.log 摄取复用同一 root）。
    disk: Option<DiskTaskOutput>,
    /// 已摄取的嵌套子代理 task_id（实例内去重；跨实例幂等靠 DB 存在性检查）。
    nested_ingested: HashSet<String>,
}

impl DashboardRecorder {
    #[must_use]
    pub fn enabled() -> bool {
        !matches!(
            std::env::var("ANYCODE_DASHBOARD_RECORD").as_deref(),
            Ok("0") | Ok("false") | Ok("off")
        )
    }

    pub async fn open() -> Option<Arc<DashboardDb>> {
        if !Self::enabled() {
            return None;
        }
        let path = std::env::var("ANYCODE_DASHBOARD_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_db_path());
        match DashboardDb::open(&path).await {
            Ok(db) => Some(Arc::new(db)),
            Err(e) => {
                tracing::debug!(error = %e, "dashboard recorder: db open skipped");
                None
            }
        }
    }

    pub async fn begin(
        db: Arc<DashboardDb>,
        kind: RunSessionKind,
        task: &Task,
        title_hint: &str,
    ) -> Result<Self> {
        let root = std::fs::canonicalize(&task.context.working_directory)
            .unwrap_or_else(|_| PathBuf::from(&task.context.working_directory));
        let root_str = root.to_string_lossy().to_string();
        let project = db
            .upsert_project(UpsertProjectRequest {
                root_path: root_str,
                name: None,
                description: None,
                create_root: None,
                ..Default::default()
            })
            .await?;
        let metadata_json = session_metadata_json(kind, task);
        let prompt_preview = truncate(&task.prompt, 240);
        let agent_type = task.agent_type.as_str().to_string();

        // Pre-created session id: task-local chat turn context first (embedded
        // chat / triggers); SESSION_ENV only as legacy fallback for headless
        // single-task CLI processes.
        let pre_session_id = anycode_core::current_dashboard_session_id()
            .or_else(|| std::env::var(crate::ipc::approval_ipc::SESSION_ENV).ok());
        let session = if let Some(pre_id) = pre_session_id {
            let pre_id = pre_id.trim().to_string();
            if !pre_id.is_empty() {
                if let Some(existing) = db.get_session(&pre_id).await? {
                    db.attach_task_to_session(
                        &pre_id,
                        &task.id.to_string(),
                        Some(agent_type.as_str()),
                        Some(prompt_preview.as_str()),
                    )
                    .await?;
                    existing
                } else {
                    db.create_or_get_session_by_task_id(CreateSessionRequest {
                        project_id: project.id.clone(),
                        kind: kind.as_str().into(),
                        task_id: Some(task.id.to_string()),
                        title: truncate(title_hint, 120),
                        prompt_preview: Some(prompt_preview.clone()),
                        agent_type: Some(agent_type.clone()),
                        model: None,
                        metadata_json: metadata_json.clone(),
                    })
                    .await?
                }
            } else {
                db.create_or_get_session_by_task_id(CreateSessionRequest {
                    project_id: project.id.clone(),
                    kind: kind.as_str().into(),
                    task_id: Some(task.id.to_string()),
                    title: truncate(title_hint, 120),
                    prompt_preview: Some(prompt_preview.clone()),
                    agent_type: Some(agent_type.clone()),
                    model: None,
                    metadata_json: metadata_json.clone(),
                })
                .await?
            }
        } else {
            db.create_or_get_session_by_task_id(CreateSessionRequest {
                project_id: project.id.clone(),
                kind: kind.as_str().into(),
                task_id: Some(task.id.to_string()),
                title: truncate(title_hint, 120),
                prompt_preview: Some(prompt_preview.clone()),
                agent_type: Some(agent_type.clone()),
                model: None,
                metadata_json,
            })
            .await?
        };

        if !task.prompt.trim().is_empty() {
            let existing_prompt = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT COUNT(*) FROM project_events
                WHERE session_id = ?
                  AND event_type = 'user_prompt'
                  AND (task_id = ? OR body = ?)
                "#,
            )
            .bind(&session.id)
            .bind(task.id.to_string())
            .bind(truncate(&task.prompt, 8000))
            .fetch_one(db.pool())
            .await
            .unwrap_or(0);
            if existing_prompt == 0 {
                if let Ok(evt) = db
                    .insert_event(InsertEventRequest {
                        project_id: project.id.clone(),
                        session_id: Some(session.id.clone()),
                        task_id: Some(task.id.to_string()),
                        agent_id: None,
                        event_type: "user_prompt".into(),
                        severity: Some("info".into()),
                        title: "User prompt".into(),
                        body: Some(truncate(&task.prompt, 8000)),
                        payload: None,
                    })
                    .await
                {
                    Self::notify_sse(evt);
                }
            }
        }
        if let Err(e) = crate::cancel_ipc::register_active(&session.id, &task.id.to_string()) {
            tracing::debug!(error = %e, "dashboard cancel_ipc register skipped");
        }
        Ok(Self {
            db,
            session_id: session.id,
            project_id: project.id,
            project_root: root,
            task_id: task.id.to_string(),
            log_offset: 0,
            pending_tool_name: None,
            pending_tool_json: None,
            pending_artifact_rel: None,
            last_model: None,
            started_at: SystemTime::now(),
            disk: None,
            nested_ingested: HashSet::new(),
        })
    }

    pub(crate) async fn scan_workspace_artifacts(&self) {
        if let Err(e) = crate::workspace_scan::scan_and_register_artifacts(
            &self.db,
            &self.project_id,
            &self.session_id,
            &self.project_root,
            self.started_at,
        )
        .await
        {
            tracing::debug!(error = %e, session_id = %self.session_id, "workspace artifact scan");
        }
    }

    pub async fn ingest_delta(&mut self, disk: &DiskTaskOutput, task_id: TaskId) {
        self.disk = Some(disk.clone());
        let Ok((delta, new_offset)) = disk.read_delta(task_id, self.log_offset, 64 * 1024) else {
            return;
        };
        if delta.is_empty() {
            return;
        }
        self.log_offset = new_offset;
        if let Err(e) = self.ingest_text(&delta).await {
            tracing::debug!(error = %e, "dashboard ingest_delta");
        }
    }

    pub async fn ingest_full_log(&mut self, disk: &DiskTaskOutput, task_id: TaskId) {
        // Drain whatever is left after the last delta — do NOT re-read from
        // offset 0: `ingest_text`'s dedup set is per-call, so re-reading the
        // whole log would insert every earlier event a second time.
        loop {
            let before = self.log_offset;
            self.ingest_delta(disk, task_id).await;
            if self.log_offset == before {
                break;
            }
        }
    }

    async fn ingest_text(&mut self, text: &str) -> Result<()> {
        let mut dedup = HashSet::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("artifact_path=") {
                let path = rest.trim();
                if !path.is_empty() {
                    self.pending_artifact_rel = Some(path.to_string());
                }
                continue;
            }
            if line.starts_with('{')
                && parse_line(line).is_none()
                && self.pending_tool_name.is_some()
            {
                self.pending_tool_json = Some(line.to_string());
                continue;
            }
            let Some(parsed) = parse_line(line) else {
                continue;
            };
            if parsed.event_type == "tool_call_input" {
                self.pending_tool_name = parsed
                    .payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                self.pending_tool_json = None;
                self.pending_artifact_rel = None;
            }
            if parsed.event_type == "tool_call_end" {
                self.maybe_record_artifact(&parsed).await;
                self.maybe_record_skill_run(&parsed).await;
                self.maybe_record_skill_artifacts(&parsed).await;
                self.pending_tool_name = None;
                self.pending_tool_json = None;
                self.pending_artifact_rel = None;
            }
            if parsed.event_type == "llm_request_start" {
                if let Some(model) = parsed.payload.get("model").and_then(|v| v.as_str()) {
                    let model = model.trim();
                    if !model.is_empty() {
                        self.last_model = Some(model.to_string());
                        let _ = self.db.update_session_model(&self.session_id, model).await;
                    }
                }
            }
            let is_gate = parsed.event_type == "gate";
            if is_gate {
                let name = parsed
                    .payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("gate");
                let status = parsed
                    .payload
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let cmd = parsed
                    .payload
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let _ = self
                    .db
                    .upsert_gate(
                        &self.project_id,
                        &self.session_id,
                        name,
                        cmd,
                        status,
                        true,
                        &parsed.body,
                    )
                    .await;
            }
            let key = format!("{}:{}", parsed.event_type, line);
            if !dedup.insert(key) {
                continue;
            }
            // Gate rows live in `gates`; skip duplicate timeline events.
            if is_gate {
                continue;
            }
            if parsed.event_type == "llm_response_end" {
                self.maybe_record_llm_usage(&parsed, &mut dedup).await;
                continue;
            }
            if parsed.event_type == "nested_task_end" {
                // 嵌套子代理完成标记：子任务 output.log 此时已完整（标记由 agent runtime
                // 在子任务 execute_task 返回后写入父日志），立即摄取其 token usage。
                let nested_id = parsed
                    .payload
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let agent_type = parsed
                    .payload
                    .get("agent_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !nested_id.is_empty() {
                    self.record_nested_task_end_event(&parsed, &nested_id).await;
                    self.ingest_nested_log(&nested_id, &agent_type).await;
                }
                continue;
            }
            if !is_index_event_type(&parsed.event_type) {
                continue;
            }
            if let Ok(evt) = self
                .db
                .insert_event(InsertEventRequest {
                    project_id: self.project_id.clone(),
                    session_id: Some(self.session_id.clone()),
                    task_id: Some(self.task_id.clone()),
                    agent_id: None,
                    event_type: parsed.event_type,
                    severity: Some(parsed.severity),
                    title: parsed.title,
                    body: Some(parsed.body),
                    payload: Some(parsed.payload),
                })
                .await
            {
                Self::notify_sse(evt);
            }
        }
        Ok(())
    }

    async fn maybe_record_skill_run(&self, event: &crate::log_parser::ParsedLine) {
        if self.pending_tool_name.as_deref() != Some("Skill") {
            return;
        }
        let Some(input) = self
            .pending_tool_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        else {
            return;
        };
        let Some(skill_id) = input.get("name").and_then(Value::as_str) else {
            return;
        };
        let status = if event.severity == "error" {
            "failed"
        } else {
            "ok"
        };
        let detail = serde_json::json!({
            "duration_ms": event.payload.get("elapsed_ms"),
            "error": event.payload.get("error"),
        });
        let _ = crate::skills_governance::record_skill_run(
            &self.db,
            skill_id,
            Some(&self.project_id),
            Some(&self.session_id),
            status,
            &detail,
        )
        .await;
    }

    fn notify_sse(evt: ProjectEvent) {
        notify::spawn_publish_event(evt);
    }

    async fn maybe_record_llm_usage(
        &self,
        parsed: &crate::log_parser::ParsedLine,
        dedup: &mut HashSet<String>,
    ) {
        let Some(payload) =
            llm_usage::usage_payload_from_parsed_with_model(parsed, self.last_model.as_deref())
        else {
            return;
        };
        let turn = payload.get("turn").and_then(|v| v.as_str()).unwrap_or("0");
        let key = llm_usage::usage_dedup_key(turn);
        if !dedup.insert(key) {
            return;
        }
        let input = payload
            .get("input_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let output = payload
            .get("output_tokens")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let title = format!("LLM usage ({input} in / {output} out tokens)");
        if let Ok(evt) = self
            .db
            .insert_event(InsertEventRequest {
                project_id: self.project_id.clone(),
                session_id: Some(self.session_id.clone()),
                task_id: Some(self.task_id.clone()),
                agent_id: None,
                event_type: LLM_USAGE_EVENT.into(),
                severity: Some("info".into()),
                title,
                body: None,
                payload: Some(payload),
            })
            .await
        {
            Self::notify_sse(evt);
        }
    }

    /// 嵌套子代理完成的时间线事件（task_id 键控为子任务 id，DB 幂等——recorder 每轮
    /// 重建会从 offset 0 重读父日志重见标记）。
    async fn record_nested_task_end_event(
        &self,
        parsed: &crate::log_parser::ParsedLine,
        nested_task_id: &str,
    ) {
        let exists: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*) FROM project_events
            WHERE session_id = ? AND event_type = 'nested_task_end' AND task_id = ?
            "#,
        )
        .bind(&self.session_id)
        .bind(nested_task_id)
        .fetch_one(self.db.pool())
        .await
        .unwrap_or(1);
        if exists > 0 {
            return;
        }
        // agent_id 是 agents 表 FK——recorder 一律置 None，agent_type 走 payload。
        if let Ok(evt) = self
            .db
            .insert_event(InsertEventRequest {
                project_id: self.project_id.clone(),
                session_id: Some(self.session_id.clone()),
                task_id: Some(nested_task_id.to_string()),
                agent_id: None,
                event_type: "nested_task_end".into(),
                severity: Some(parsed.severity.clone()),
                title: parsed.title.clone(),
                body: Some(parsed.body.clone()),
                payload: Some(parsed.payload.clone()),
            })
            .await
        {
            Self::notify_sse(evt);
        }
    }

    /// 摄取嵌套子代理 output.log 的 `llm_response_end` → `llm_usage` 事件
    ///（task_id = 子任务 id，payload 带 `agent_type` / `nested` / `parent_task_id`），
    /// 解锁 metrics 的 by_agent token 分组。实例内 `nested_ingested` 去重 +
    /// DB（session, task_id, turn）存在性检查兜底跨实例幂等。
    async fn ingest_nested_log(&mut self, nested_task_id: &str, agent_type: &str) {
        if !self.nested_ingested.insert(nested_task_id.to_string()) {
            return;
        }
        let (Some(disk), Ok(nested_uuid)) =
            (self.disk.clone(), uuid::Uuid::parse_str(nested_task_id))
        else {
            return;
        };
        let content = std::fs::read_to_string(disk.output_path(nested_uuid)).unwrap_or_default();
        if content.is_empty() {
            return;
        }
        let mut last_model: Option<String> = None;
        for line in content.lines() {
            let Some(parsed) = parse_line(line) else {
                continue;
            };
            if parsed.event_type == "llm_request_start" {
                if let Some(model) = parsed
                    .payload
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    last_model = Some(model.to_string());
                }
                continue;
            }
            if parsed.event_type != "llm_response_end" {
                continue;
            }
            let Some(mut payload) =
                llm_usage::usage_payload_from_parsed_with_model(&parsed, last_model.as_deref())
            else {
                continue;
            };
            // 子代理身份以标记为准（行内 agent_type 缺失时兜底，如旧日志）。
            if payload.get("agent_type").is_none() && !agent_type.is_empty() {
                payload["agent_type"] = Value::String(agent_type.to_string());
            }
            payload["nested"] = Value::Bool(true);
            payload["parent_task_id"] = Value::String(self.task_id.clone());
            let turn = payload
                .get("turn")
                .and_then(|v| v.as_str())
                .unwrap_or("0")
                .to_string();
            let exists: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM project_events
                WHERE session_id = ?
                  AND event_type = ?
                  AND task_id = ?
                  AND json_extract(payload_json, '$.turn') = ?
                "#,
            )
            .bind(&self.session_id)
            .bind(LLM_USAGE_EVENT)
            .bind(nested_task_id)
            .bind(&turn)
            .fetch_one(self.db.pool())
            .await
            .unwrap_or(1);
            if exists > 0 {
                continue;
            }
            let input = payload
                .get("input_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let output = payload
                .get("output_tokens")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let title =
                format!("LLM usage ({input} in / {output} out tokens, subagent {agent_type})");
            // agent_id 是 agents 表 FK——置 None，agent_type 已在 payload。
            if let Ok(evt) = self
                .db
                .insert_event(InsertEventRequest {
                    project_id: self.project_id.clone(),
                    session_id: Some(self.session_id.clone()),
                    task_id: Some(nested_task_id.to_string()),
                    agent_id: None,
                    event_type: LLM_USAGE_EVENT.into(),
                    severity: Some("info".into()),
                    title,
                    body: None,
                    payload: Some(payload),
                })
                .await
            {
                Self::notify_sse(evt);
            }
        }
    }

    async fn maybe_record_artifact(&self, _parsed: &crate::log_parser::ParsedLine) {
        let Some(tool) = self.pending_tool_name.as_deref() else {
            return;
        };
        if tool == "Skill" {
            return;
        }
        if !ARTIFACT_TOOLS.contains(&tool) {
            return;
        }
        let Some(json) = self.pending_tool_json.as_deref() else {
            if let Some(rel) = self.pending_artifact_rel.as_deref() {
                let kind = if tool == "NotebookEdit" {
                    "notebook"
                } else {
                    "file"
                };
                self.record_artifact_rel(rel, kind).await;
            }
            return;
        };
        if tool == "Bash" {
            for rel in extract_bash_output_paths(json) {
                self.record_artifact_rel(&rel, "file").await;
            }
            return;
        }
        let rel = self
            .pending_artifact_rel
            .as_deref()
            .map(str::to_string)
            .or_else(|| extract_artifact_path(json));
        let Some(rel) = rel else {
            return;
        };
        let kind = if tool == "NotebookEdit" {
            "notebook"
        } else {
            "file"
        };
        self.record_artifact_rel(&rel, kind).await;
    }

    async fn maybe_record_skill_artifacts(&self, event: &crate::log_parser::ParsedLine) {
        if self.pending_tool_name.as_deref() != Some("Skill") {
            return;
        }
        let mut paths = extract_artifact_paths_from_text(&event.body);
        if let Some(raw) = self.pending_tool_json.as_deref() {
            paths.extend(extract_artifact_paths_from_text(raw));
        }
        for rel in paths {
            let kind = skill_artifact_kind(&rel);
            self.record_artifact_rel(&rel, kind).await;
        }
    }

    async fn record_artifact_rel(&self, rel: &str, kind: &str) {
        let path = self.project_root.join(rel).to_string_lossy().to_string();
        let title = Path::new(rel)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(rel)
            .to_string();
        let _ = self
            .db
            .upsert_artifact(&self.project_id, &self.session_id, &path, kind, &title)
            .await;
    }

    pub async fn finish_with_status(&self, status: &str, summary: Option<&str>) {
        self.scan_workspace_artifacts().await;
        if let Err(e) = self
            .db
            .finish_session(&self.session_id, status, summary)
            .await
        {
            tracing::debug!(error = %e, "dashboard finish_with_status");
        }
        crate::cancel_ipc::unregister_active(&self.session_id);
    }

    pub async fn finish_run(
        &mut self,
        disk: &DiskTaskOutput,
        task_id: TaskId,
        summary: Option<&str>,
    ) {
        // 补读尾部 delta：channel-bridge 路径直接调 finish_run（无前置 ingest_full_log），
        // 尾段里的嵌套 `nested_task_end` 标记与 llm_usage 需在此落库（offset 幂等）。
        self.ingest_full_log(disk, task_id).await;
        let path = disk.output_path(task_id);
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let lines: Vec<&str> = content.lines().collect();
        let status = if content.contains("[task_end] status=failed") {
            "failed".to_string()
        } else {
            task_end_status(&lines).unwrap_or_else(|| "completed".into())
        };
        self.scan_workspace_artifacts().await;
        if let Err(e) = self
            .db
            .finish_session(&self.session_id, &status, summary)
            .await
        {
            tracing::debug!(error = %e, "dashboard finish_run");
        }
        crate::cancel_ipc::unregister_active(&self.session_id);
    }

    pub async fn finish_goal(
        &mut self,
        disk: &DiskTaskOutput,
        task_id: TaskId,
        progress: &GoalProgress,
        done_when: Option<&str>,
        working_dir: &Path,
    ) {
        self.ingest_full_log(disk, task_id).await;
        self.record_goal_gates(progress, done_when, working_dir)
            .await;
        let status = if progress.completed {
            "completed"
        } else {
            "failed"
        };
        let summary = progress
            .last_error
            .as_deref()
            .or(progress.last_output.as_deref());
        if let Err(e) = self
            .db
            .merge_session_metadata(
                &self.session_id,
                &serde_json::json!({ "goal_attempts": progress.attempts }),
            )
            .await
        {
            tracing::debug!(error = %e, "dashboard goal_attempts metadata");
        }
        self.scan_workspace_artifacts().await;
        if let Err(e) = self
            .db
            .finish_session(&self.session_id, status, summary)
            .await
        {
            tracing::debug!(error = %e, "dashboard finish_goal");
        }
        crate::cancel_ipc::unregister_active(&self.session_id);
    }

    async fn record_goal_gates(
        &self,
        progress: &GoalProgress,
        done_when: Option<&str>,
        working_dir: &Path,
    ) {
        if let Some(marker) = done_when.filter(|m| !m.is_empty()) {
            let (status, excerpt) = if progress.completed {
                ("passed", "")
            } else {
                (
                    "failed",
                    progress
                        .last_error
                        .as_deref()
                        .unwrap_or("README marker or objective not satisfied"),
                )
            };
            let _ = self
                .db
                .upsert_gate(
                    &self.project_id,
                    &self.session_id,
                    &format!("README `{marker}`"),
                    "readme_marker",
                    status,
                    true,
                    excerpt,
                )
                .await;
        }

        let flutter_scope =
            working_dir.join("pubspec.yaml").is_file() || working_dir.join("test").is_dir();
        if flutter_scope {
            let err = progress.last_error.as_deref().unwrap_or("");
            if progress.completed {
                for (name, cmd) in [
                    ("flutter analyze", "flutter analyze"),
                    ("flutter test", "flutter test"),
                ] {
                    let _ = self
                        .db
                        .upsert_gate(
                            &self.project_id,
                            &self.session_id,
                            name,
                            cmd,
                            "passed",
                            true,
                            "",
                        )
                        .await;
                }
            } else {
                if err.contains("flutter analyze") {
                    let _ = self
                        .db
                        .upsert_gate(
                            &self.project_id,
                            &self.session_id,
                            "flutter analyze",
                            "flutter analyze",
                            "failed",
                            true,
                            &truncate(err, 2000),
                        )
                        .await;
                }
                if err.contains("flutter test") {
                    let _ = self
                        .db
                        .upsert_gate(
                            &self.project_id,
                            &self.session_id,
                            "flutter test",
                            "flutter test",
                            "failed",
                            true,
                            &truncate(err, 2000),
                        )
                        .await;
                }
            }
        }

        if let Some(marker) = done_when.filter(|m| {
            let lower = m.to_lowercase();
            lower.contains("browser")
                || lower.contains("manual")
                || lower.contains("visual")
                || lower.contains("screenshot")
        }) {
            let (status, excerpt) = if progress.completed {
                ("passed", "Manual/browser verification recorded")
            } else {
                (
                    "failed",
                    progress
                        .last_error
                        .as_deref()
                        .unwrap_or("Manual verification not satisfied"),
                )
            };
            let _ = self
                .db
                .upsert_gate(
                    &self.project_id,
                    &self.session_id,
                    "Manual / browser verification",
                    "manual_verification",
                    status,
                    true,
                    &truncate(excerpt, 2000),
                )
                .await;
            let _ = marker;
        }
    }

    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    #[must_use]
    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// Workflow step marker for the session timeline (not a verification gate).
    pub async fn log_workflow_step(&self, step_id: &str, title: &str, status: &str) {
        let payload = serde_json::json!({ "step_id": step_id, "status": status });
        if let Ok(evt) = self
            .db
            .insert_event(InsertEventRequest {
                project_id: self.project_id.clone(),
                session_id: Some(self.session_id.clone()),
                task_id: Some(self.task_id.clone()),
                agent_id: None,
                event_type: "workflow_step".into(),
                severity: Some(if status == "failed" {
                    "error".into()
                } else {
                    "info".into()
                }),
                title: title.to_string(),
                body: None,
                payload: Some(payload),
            })
            .await
        {
            Self::notify_sse(evt);
        }
    }
}

fn extract_artifact_path(json: &str) -> Option<String> {
    let v: Value = serde_json::from_str(json).ok()?;
    for key in ["file_path", "path", "notebook_path", "target_file"] {
        if let Some(p) = v.get(key).and_then(|x| x.as_str()) {
            if !p.is_empty() {
                return Some(p.to_string());
            }
        }
    }
    None
}

fn extract_artifact_paths_from_text(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for token in text.split_whitespace() {
        let cleaned = token.trim_matches(|c| {
            c == '"' || c == '\'' || c == ',' || c == ';' || c == '(' || c == ')'
        });
        if cleaned.is_empty() {
            continue;
        }
        let lower = cleaned.to_lowercase();
        let is_artifact = lower.ends_with(".pptx")
            || lower.ends_with(".pdf")
            || lower.ends_with(".md")
            || lower.ends_with(".png")
            || lower.ends_with(".docx")
            || lower.ends_with(".xlsx");
        if !is_artifact {
            continue;
        }
        if !paths.iter().any(|p| p == cleaned) {
            paths.push(cleaned.to_string());
        }
    }
    paths
}

fn skill_artifact_kind(rel: &str) -> &'static str {
    anycode_core::artifact_kind_for_path(rel)
}

fn extract_bash_output_paths(json: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let cmd = v
        .get("command")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .trim();
    if cmd.is_empty() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    if let Some(idx) = cmd.rfind(">> ") {
        push_shell_path(&mut paths, cmd[idx + 3..].trim());
    } else if let Some(idx) = cmd.rfind("> ") {
        push_shell_path(&mut paths, cmd[idx + 2..].trim());
    }
    for prefix in ["touch ", "tee ", "cp ", "mv "] {
        if let Some(rest) = cmd.strip_prefix(prefix) {
            if let Some(last) = rest.split_whitespace().last() {
                push_shell_path(&mut paths, last);
            }
        }
    }
    paths
}

fn push_shell_path(out: &mut Vec<String>, raw: &str) {
    let path = raw
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches(';')
        .trim();
    if path.is_empty() || path.starts_with('-') || path.contains('$') {
        return;
    }
    if !out.iter().any(|p| p == path) {
        out.push(path.to_string());
    }
}

fn session_metadata_json(kind: RunSessionKind, task: &Task) -> Option<String> {
    if kind == RunSessionKind::Cron {
        let mut meta = serde_json::json!({
            "correlation_id": task.context.session_id.to_string(),
            "source": "cron",
        });
        if let Some(job_id) = task.prompt.strip_prefix("Cron ") {
            let job_id = job_id.split_whitespace().next().unwrap_or(job_id);
            meta["cron_job_id"] = serde_json::Value::String(job_id.to_string());
        }
        return serde_json::to_string(&meta).ok();
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::extract_artifact_path;

    #[test]
    fn extracts_file_path_from_tool_json() {
        let json = r#"{"file_path":"lib/main.dart"}"#;
        assert_eq!(
            extract_artifact_path(json).as_deref(),
            Some("lib/main.dart")
        );
    }

    #[test]
    fn extracts_bash_redirect_path() {
        let paths = super::extract_bash_output_paths(r#"{"command":"echo hi >> out/report.md"}"#);
        assert!(paths.iter().any(|p| p.contains("report.md")));
    }
}
