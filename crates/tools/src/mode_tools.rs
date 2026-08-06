//! Plan / Worktree / ToolSearch / Sleep / StructuredOutput

use crate::services::ToolServices;
use anycode_core::prelude::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub struct EnterPlanModeTool {
    services: Arc<ToolServices>,
}

impl EnterPlanModeTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }
    fn description(&self) -> &str {
        "Mark session as plan mode (stored in ToolServices)."
    }
    fn schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{}})
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }
    async fn execute(&self, _input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        self.services.set_plan_mode(true);
        Ok(ToolOutput {
            result: json!({ "plan_mode": true }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub struct ExitPlanModeTool {
    services: Arc<ToolServices>,
}

impl ExitPlanModeTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }
    fn description(&self) -> &str {
        "Leave plan mode."
    }
    fn schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{}})
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }
    async fn execute(&self, _input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        self.services.set_plan_mode(false);
        Ok(ToolOutput {
            result: json!({ "plan_mode": false }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Deserialize)]
struct EwIn {
    #[serde(default)]
    name: Option<String>,
}

pub struct EnterWorktreeTool {
    services: Arc<ToolServices>,
    policy: SecurityPolicy,
}

impl EnterWorktreeTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self {
            services,
            policy: SecurityPolicy::sensitive_mutation(),
        }
    }
}

#[async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }
    fn description(&self) -> &str {
        "Create a git worktree under the repo and record its path."
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Optional worktree directory name segment" }
            }
        })
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        Some(&self.policy)
    }
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let e: EwIn = serde_json::from_value(input.input).unwrap_or(EwIn { name: None });
        let cwd = input
            .working_directory
            .clone()
            .unwrap_or_else(|| ".".to_string());
        let slug = e.name.unwrap_or_else(|| {
            format!(
                "anycode-{}",
                uuid::Uuid::new_v4().to_string().split('-').next().unwrap()
            )
        });
        let path = format!("../wt-{}", slug);
        let cwd_b = cwd.clone();
        let path_b = path.clone();
        let status = tokio::task::spawn_blocking(move || {
            Command::new("git")
                .args(["worktree", "add", &path_b, "HEAD"])
                .current_dir(&cwd_b)
                .status()
        })
        .await
        .map_err(|e| CoreError::Other(anyhow::anyhow!("join: {}", e)))?
        .map_err(CoreError::IoError)?;

        if !status.success() {
            return Ok(ToolOutput {
                result: json!({
                    "error": "git worktree add failed",
                    "path": path,
                    "cwd": cwd
                }),
                error: Some("git failed".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }

        let abs = std::path::Path::new(&cwd).join(&path);
        let abs_s = abs.to_string_lossy().to_string();
        self.services.set_worktree(Some(abs_s.clone()));

        Ok(ToolOutput {
            result: json!({
                "worktreePath": abs_s,
                "message": "worktree created"
            }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

pub struct ExitWorktreeTool {
    services: Arc<ToolServices>,
    policy: SecurityPolicy,
}

impl ExitWorktreeTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self {
            services,
            policy: SecurityPolicy::sensitive_mutation(),
        }
    }
}

#[async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }
    fn description(&self) -> &str {
        "Clear recorded worktree path (does not remove git worktree on disk)."
    }
    fn schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{}})
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        Some(&self.policy)
    }
    async fn execute(&self, _input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let prev = self.services.worktree_path();
        self.services.set_worktree(None);
        Ok(ToolOutput {
            result: json!({ "previous": prev, "cleared": true }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Deserialize)]
struct TsIn {
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    name: String,
    /// 支持 `select:a,b,c`（对齐 Claude ToolSearch 多选登记）。
    #[serde(default)]
    query: String,
}

fn toolsearch_deferred_names(t: &TsIn) -> Vec<String> {
    let q = t.query.trim();
    if let Some(rest) = q
        .strip_prefix("select:")
        .or_else(|| q.strip_prefix("select :"))
        .map(str::trim_start)
    {
        return rest
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    let n = if !t.tool_name.trim().is_empty() {
        t.tool_name.trim().to_string()
    } else {
        t.name.trim().to_string()
    };
    if n.is_empty() {
        vec![]
    } else {
        vec![n]
    }
}

pub struct ToolSearchTool {
    services: Arc<ToolServices>,
}

impl ToolSearchTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }
    fn description(&self) -> &str {
        "Defer discovery of tools: tool_name or name, or query \"select:a,b\" for multiple. Unlocks deferred MCP tools when defer_mcp_tools is enabled."
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "name": { "type": "string" },
                "query": { "type": "string", "description": "e.g. select:mcp__srv__tool_a,mcp__srv__tool_b" }
            }
        })
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let t: TsIn = serde_json::from_value(input.input).unwrap_or(TsIn {
            tool_name: String::new(),
            name: String::new(),
            query: String::new(),
        });
        let names = toolsearch_deferred_names(&t);
        if names.is_empty() {
            return Ok(ToolOutput {
                result: json!({ "error": "missing tool_name, name, or query select:..." }),
                error: Some("missing tool name".to_string()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        for n in &names {
            self.services.defer_tool(n.clone());
            self.services.register_mcp_tool_for_llm_session(n);
        }
        Ok(ToolOutput {
            result: json!({ "deferred": names, "all": self.services.deferred_tools() }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Deserialize)]
struct SleepIn {
    #[serde(default = "default_ms")]
    duration_ms: u64,
}

fn default_ms() -> u64 {
    1000
}

pub struct SleepTool;

#[async_trait]
impl Tool for SleepTool {
    fn name(&self) -> &str {
        "Sleep"
    }
    fn description(&self) -> &str {
        "Async sleep (capped) for proactive pacing."
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "duration_ms": { "type": "number", "description": "Milliseconds to wait (max 60s)" }
            }
        })
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let s: SleepIn =
            serde_json::from_value(input.input).unwrap_or(SleepIn { duration_ms: 1000 });
        let ms = s.duration_ms.clamp(1, 60_000);
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        Ok(ToolOutput {
            result: json!({ "slept_ms": ms }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

/// 轻量 JSON schema 校验（顶层 type / required / 逐属性 type），不引新依赖。
/// 允许多余键；失败返回人类可读清单，供模型自愈重试。
fn validate_structured_against_schema(
    value: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<(), Vec<String>> {
    fn type_ok(v: &serde_json::Value, ty: &str) -> bool {
        match ty {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "number" => v.is_number(),
            "integer" => v.is_i64() || v.is_u64(),
            "null" => v.is_null(),
            _ => true,
        }
    }
    let mut issues = Vec::new();
    if let Some(ty) = schema.get("type").and_then(|t| t.as_str()) {
        if !type_ok(value, ty) {
            issues.push(format!("top-level value must be of type `{ty}`"));
        }
    }
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        if let Some(obj) = value.as_object() {
            for req in required.iter().filter_map(|r| r.as_str()) {
                if !obj.contains_key(req) {
                    issues.push(format!("missing required key `{req}`"));
                }
            }
        }
    }
    if let (Some(obj), Some(props)) = (
        value.as_object(),
        schema.get("properties").and_then(|p| p.as_object()),
    ) {
        for (key, prop_schema) in props {
            let Some(v) = obj.get(key) else { continue };
            let Some(ty) = prop_schema.get("type").and_then(|t| t.as_str()) else {
                continue;
            };
            if !type_ok(v, ty) {
                issues.push(format!("property `{key}` must be of type `{ty}`"));
            }
        }
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

pub struct StructuredOutputTool {
    services: Arc<ToolServices>,
}

impl StructuredOutputTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for StructuredOutputTool {
    fn name(&self) -> &str {
        "StructuredOutput"
    }
    fn description(&self) -> &str {
        "Return structured JSON output. When the parent agent attached an output schema to this task, the JSON must match it (top-level type, required keys, declared property types); mismatches return an error so you can correct and retry. Call exactly once as the final action."
    }
    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "additionalProperties": true
        })
    }
    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }
    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }
    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let value = input.input.clone();
        if let Some(task_id) = input.task_id {
            if let Some(schema) = self.services.structured_output_schema(task_id) {
                if let Err(issues) = validate_structured_against_schema(&value, &schema) {
                    return Ok(ToolOutput {
                        result: json!({ "recorded": false, "issues": issues }),
                        error: Some(format!(
                            "structured output does not match schema: {}",
                            issues.join("; ")
                        )),
                        duration_ms: start.elapsed().as_millis() as u64,
                    });
                }
            }
            self.services
                .record_structured_output(task_id, value.clone());
        }
        Ok(ToolOutput {
            result: json!({ "recorded": true, "value": value }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod structured_output_tests {
    use super::*;
    use crate::services::ToolServices;

    fn schema_obj() -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["verdict"],
            "properties": {
                "verdict": { "type": "string" },
                "count": { "type": "integer" },
                "score": { "type": "number" }
            }
        })
    }

    #[test]
    fn validate_accepts_valid_and_extra_keys() {
        let v = json!({ "verdict": "ok", "count": 3, "score": 1.5, "extra": [1, 2] });
        assert!(validate_structured_against_schema(&v, &schema_obj()).is_ok());
    }

    #[test]
    fn validate_missing_required() {
        let v = json!({ "count": 1 });
        let err = validate_structured_against_schema(&v, &schema_obj()).unwrap_err();
        assert!(
            err.iter()
                .any(|i| i.contains("missing required key `verdict`")),
            "{err:?}"
        );
    }

    #[test]
    fn validate_wrong_property_type() {
        let v = json!({ "verdict": 42 });
        let err = validate_structured_against_schema(&v, &schema_obj()).unwrap_err();
        assert!(
            err.iter()
                .any(|i| i.contains("`verdict`") && i.contains("string")),
            "{err:?}"
        );
    }

    #[test]
    fn validate_integer_vs_number() {
        // integer 拒绝浮点
        let v = json!({ "verdict": "ok", "count": 1.5 });
        assert!(validate_structured_against_schema(&v, &schema_obj()).is_err());
        // number 接受整数与浮点
        let v2 = json!({ "verdict": "ok", "score": 5 });
        assert!(validate_structured_against_schema(&v2, &schema_obj()).is_ok());
    }

    #[test]
    fn validate_top_level_type_mismatch() {
        let v = json!(["not", "an", "object"]);
        let err = validate_structured_against_schema(&v, &schema_obj()).unwrap_err();
        assert!(err.iter().any(|i| i.contains("top-level")), "{err:?}");
    }

    fn ti(value: serde_json::Value, task_id: Option<uuid::Uuid>) -> ToolInput {
        ToolInput {
            name: "StructuredOutput".into(),
            input: value,
            working_directory: None,
            sandbox_mode: false,
            dashboard_session_id: None,
            task_id,
        }
    }

    #[tokio::test]
    async fn execute_records_without_schema() {
        let services = Arc::new(ToolServices::default());
        let tool = StructuredOutputTool::new(services.clone());
        let tid = uuid::Uuid::new_v4();
        let out = tool
            .execute(ti(json!({ "a": 1 }), Some(tid)))
            .await
            .unwrap();
        assert!(out.error.is_none());
        assert_eq!(out.result["recorded"], true);
        assert_eq!(
            services.take_structured_output(tid),
            Some(json!({ "a": 1 }))
        );
    }

    #[tokio::test]
    async fn execute_rejects_mismatch_and_does_not_record() {
        let services = Arc::new(ToolServices::default());
        let tid = uuid::Uuid::new_v4();
        services.set_structured_output_schema(tid, schema_obj());
        let tool = StructuredOutputTool::new(services.clone());
        let out = tool
            .execute(ti(json!({ "count": "no" }), Some(tid)))
            .await
            .unwrap();
        assert!(
            out.error.is_some(),
            "mismatch should surface as error for self-heal retry"
        );
        assert_eq!(out.result["recorded"], false);
        assert!(
            services.take_structured_output(tid).is_none(),
            "rejected value must not be captured"
        );
    }

    #[tokio::test]
    async fn execute_accepts_match_and_records() {
        let services = Arc::new(ToolServices::default());
        let tid = uuid::Uuid::new_v4();
        services.set_structured_output_schema(tid, schema_obj());
        let tool = StructuredOutputTool::new(services.clone());
        let out = tool
            .execute(ti(json!({ "verdict": "ok" }), Some(tid)))
            .await
            .unwrap();
        assert!(out.error.is_none());
        assert_eq!(
            services.take_structured_output(tid),
            Some(json!({ "verdict": "ok" }))
        );
    }

    #[tokio::test]
    async fn execute_without_task_id_is_passthrough() {
        let services = Arc::new(ToolServices::default());
        let tool = StructuredOutputTool::new(services.clone());
        let out = tool.execute(ti(json!({ "x": true }), None)).await.unwrap();
        assert!(out.error.is_none());
        assert_eq!(out.result["recorded"], true);
    }
}
