//! 将 MCP `tools/list` 中的条目注册为独立 API 工具名 `mcp__<server>__<tool>`。

use crate::mcp_connected::McpConnected;
use anycode_core::prelude::*;
use anycode_core::ExecutionTraceEvent;
use async_trait::async_trait;
use serde_json::json;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};

/// Runtime MCP governance from `config.json` → `mcp.governance` (installed at bootstrap).
#[derive(Debug, Clone, Default)]
pub struct McpGovernanceRuntime {
    pub strict: bool,
    pub max_calls_per_server: Option<usize>,
    pub allowed_tools: HashSet<String>,
}

static MCP_GOVERNANCE: OnceLock<McpGovernanceRuntime> = OnceLock::new();

/// Called once from bootstrap after config load.
pub fn install_mcp_governance(governance: McpGovernanceRuntime) {
    let _ = MCP_GOVERNANCE.set(governance);
}

fn configured_governance() -> Option<&'static McpGovernanceRuntime> {
    MCP_GOVERNANCE.get()
}

static MCP_SERVER_COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

pub struct McpProxiedTool {
    session: Arc<dyn McpConnected>,
    server_slug: String,
    logical_name: String,
    mcp_tool_name: String,
    description: String,
    schema: Value,
    policy: SecurityPolicy,
}

impl McpProxiedTool {
    pub fn new(
        session: Arc<dyn McpConnected>,
        server_slug: String,
        logical_name: String,
        mcp_tool_name: String,
        description: String,
        schema: Value,
    ) -> Self {
        Self {
            session,
            server_slug,
            logical_name,
            mcp_tool_name,
            description,
            schema,
            policy: SecurityPolicy::sensitive_mutation(),
        }
    }

    fn governance_check(&self) -> Result<(), CoreError> {
        mcp_governance_check(&self.server_slug, &self.logical_name, &self.mcp_tool_name)
    }
}

#[async_trait]
impl Tool for McpProxiedTool {
    fn name(&self) -> &str {
        &self.logical_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        Some(&self.policy)
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        self.governance_check()?;
        self.session
            .call_tool_named(&self.mcp_tool_name, input.input.clone())
            .await
    }
}

pub(crate) fn mcp_governance_check(
    server: &str,
    logical_name: &str,
    mcp_tool_name: &str,
) -> Result<(), CoreError> {
    if mcp_strict_enabled() && !mcp_tool_allowed(server, logical_name, mcp_tool_name) {
        let detail = format!(
            "MCP strict mode denied {logical_name} (server={server}, tool={mcp_tool_name})"
        );
        emit_mcp_governance_event("mcp_denied", server, logical_name, mcp_tool_name, &detail);
        return Err(CoreError::PermissionDenied(detail));
    }
    if let Some(max) = mcp_max_calls_per_server() {
        let counts = MCP_SERVER_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut counts = counts.lock().expect("mcp server call counts");
        let count = counts.entry(server.to_string()).or_insert(0);
        if *count >= max {
            let detail = format!(
                "MCP server {server} exceeded call quota {max} (set ANYCODE_MCP_MAX_CALLS_PER_SERVER to adjust)"
            );
            emit_mcp_governance_event(
                "mcp_quota_exceeded",
                server,
                logical_name,
                mcp_tool_name,
                &detail,
            );
            return Err(CoreError::PermissionDenied(detail));
        }
        *count += 1;
    }
    Ok(())
}

fn mcp_strict_enabled() -> bool {
    if matches!(
        std::env::var("ANYCODE_MCP_STRICT").as_deref(),
        Ok("1") | Ok("true") | Ok("yes") | Ok("on")
    ) {
        return true;
    }
    configured_governance().is_some_and(|g| g.strict)
}

fn mcp_max_calls_per_server() -> Option<usize> {
    std::env::var("ANYCODE_MCP_MAX_CALLS_PER_SERVER")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .or_else(|| {
            configured_governance()
                .and_then(|g| g.max_calls_per_server)
                .filter(|v| *v > 0)
        })
}

fn mcp_allowed_tools_set() -> HashSet<String> {
    let from_env = std::env::var("ANYCODE_MCP_ALLOWED_TOOLS").unwrap_or_default();
    if !from_env.trim().is_empty() {
        return from_env
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    configured_governance()
        .map(|g| g.allowed_tools.clone())
        .unwrap_or_default()
}

fn mcp_tool_allowed(server: &str, logical_name: &str, mcp_tool_name: &str) -> bool {
    let allow = mcp_allowed_tools_set();
    if allow.is_empty() {
        return false;
    }
    allow.contains(logical_name)
        || allow.contains(mcp_tool_name)
        || allow.contains(&format!("{server}:{mcp_tool_name}"))
}

fn audit_events_path() -> Option<PathBuf> {
    Some(anycode_core::user_home_dir()?.join(".anycode/audit/events.jsonl"))
}

/// Best-effort governance trace for Dashboard / doctor (`mcp_denied` / `mcp_quota_exceeded`).
fn emit_mcp_governance_event(
    event_type: &str,
    server: &str,
    logical_name: &str,
    mcp_tool_name: &str,
    detail: &str,
) {
    let Some(path) = audit_events_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let evt = ExecutionTraceEvent::new(
        event_type,
        "warn",
        format!("MCP {event_type}: {logical_name}"),
        detail,
        json!({
            "server": server,
            "tool": mcp_tool_name,
            "logical_name": logical_name,
        }),
    );
    let Ok(line) = serde_json::to_string(&evt) else {
        return;
    };
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
pub(crate) fn reset_mcp_call_counts_for_tests() {
    if let Some(counts) = MCP_SERVER_COUNTS.get() {
        counts.lock().expect("mcp counts").clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static ENV_LOCK: StdMutex<()> = StdMutex::new(());

    fn with_env_lock<R>(f: impl FnOnce() -> R) -> R {
        let _g = ENV_LOCK.lock().expect("env lock");
        f()
    }

    #[test]
    fn allowlist_accepts_logical_and_server_tool_names() {
        with_env_lock(|| {
            std::env::set_var("ANYCODE_MCP_ALLOWED_TOOLS", "mcp__s__search,api:list");
            assert!(mcp_tool_allowed("s", "mcp__s__search", "search"));
            assert!(mcp_tool_allowed("api", "mcp__api__list", "list"));
            assert!(!mcp_tool_allowed("api", "mcp__api__write", "write"));
            std::env::remove_var("ANYCODE_MCP_ALLOWED_TOOLS");
        });
    }

    #[test]
    fn strict_mode_denies_tools_outside_allowlist_and_emits_event() {
        with_env_lock(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            std::env::set_var("HOME", tmp.path());
            std::env::set_var("ANYCODE_MCP_STRICT", "1");
            std::env::set_var("ANYCODE_MCP_ALLOWED_TOOLS", "mcp__ok__ping");
            std::env::remove_var("ANYCODE_MCP_MAX_CALLS_PER_SERVER");
            reset_mcp_call_counts_for_tests();

            let err = mcp_governance_check("evil", "mcp__evil__rm", "rm").expect_err("denied");
            assert!(
                matches!(err, CoreError::PermissionDenied(ref m) if m.contains("strict")),
                "{err:?}"
            );

            let path = tmp.path().join(".anycode/audit/events.jsonl");
            let body = std::fs::read_to_string(&path).expect("events written");
            assert!(
                body.contains("\"event_type\":\"mcp_denied\""),
                "missing mcp_denied in {body}"
            );

            assert!(mcp_governance_check("ok", "mcp__ok__ping", "ping").is_ok());

            std::env::remove_var("ANYCODE_MCP_STRICT");
            std::env::remove_var("ANYCODE_MCP_ALLOWED_TOOLS");
        });
    }

    #[test]
    fn quota_exceeded_emits_event_and_blocks_further_calls() {
        with_env_lock(|| {
            let tmp = tempfile::tempdir().expect("tempdir");
            std::env::set_var("HOME", tmp.path());
            std::env::remove_var("ANYCODE_MCP_STRICT");
            std::env::remove_var("ANYCODE_MCP_ALLOWED_TOOLS");
            std::env::set_var("ANYCODE_MCP_MAX_CALLS_PER_SERVER", "2");
            reset_mcp_call_counts_for_tests();

            assert!(mcp_governance_check("s", "mcp__s__a", "a").is_ok());
            assert!(mcp_governance_check("s", "mcp__s__b", "b").is_ok());
            let err = mcp_governance_check("s", "mcp__s__c", "c").expect_err("quota");
            assert!(
                matches!(err, CoreError::PermissionDenied(ref m) if m.contains("quota")),
                "{err:?}"
            );

            let path = tmp.path().join(".anycode/audit/events.jsonl");
            let body = std::fs::read_to_string(&path).expect("events written");
            assert!(
                body.contains("\"event_type\":\"mcp_quota_exceeded\""),
                "missing mcp_quota_exceeded in {body}"
            );

            // Other servers still allowed under their own counter.
            assert!(mcp_governance_check("other", "mcp__other__x", "x").is_ok());

            std::env::remove_var("ANYCODE_MCP_MAX_CALLS_PER_SERVER");
        });
    }
}
