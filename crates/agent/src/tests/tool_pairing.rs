//! CORE-13 配对不变量回归:取消/MaxToolCalls 中断工具批次时,未执行的
//! tool_use 必须补合成 tool_result(否则下一轮请求 Anthropic 400)。

use super::support::{msg_text, DummyMemoryStore, EchoTool, MockLLM};
use crate::{
    AgentClaudeToolGating, AgentRuntime, RuntimeCoreDeps, RuntimeMemoryOptions,
    RuntimePromptConfig, RuntimeToolPolicy,
};
use anycode_core::prelude::*;
use anycode_core::AgentLoopLimits;
use anycode_security::SecurityLayer;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

/// 执行时置位取消旗标的工具,用于模拟「批次中途被取消」。
struct SetCancelTool(Arc<AtomicBool>);

#[async_trait]
impl Tool for SetCancelTool {
    fn name(&self) -> &str {
        "SetCancel"
    }

    fn description(&self) -> &str {
        "set the cooperative cancel flag"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Auto
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }

    async fn execute(&self, _input: ToolInput) -> Result<ToolOutput, CoreError> {
        self.0.store(true, Ordering::Relaxed);
        Ok(ToolOutput {
            result: serde_json::json!({ "cancel": true }),
            error: None,
            duration_ms: 1,
        })
    }
}

fn tool_call_response(calls: Vec<ToolCall>) -> LLMResponse {
    llm_response("calling tools", calls)
}

fn text_response(text: &str) -> LLMResponse {
    llm_response(text, vec![])
}

fn llm_response(text: &str, calls: Vec<ToolCall>) -> LLMResponse {
    LLMResponse {
        message: msg_text(MessageRole::Assistant, text),
        tool_calls: calls,
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
            cache_creation_tokens: None,
            cache_read_tokens: None,
        },
    }
}

fn make_runtime(llm: Arc<MockLLM>, disk: DiskTaskOutput) -> AgentRuntime {
    make_runtime_with(llm, disk, HashMap::new())
}

fn make_runtime_with_cancel_tool(
    llm: Arc<MockLLM>,
    disk: DiskTaskOutput,
    flag: Arc<AtomicBool>,
) -> AgentRuntime {
    let mut extra: HashMap<ToolName, Box<dyn Tool>> = HashMap::new();
    extra.insert("SetCancel".to_string(), Box::new(SetCancelTool(flag)));
    make_runtime_with(llm, disk, extra)
}

fn make_runtime_with(
    llm: Arc<MockLLM>,
    disk: DiskTaskOutput,
    extra: HashMap<ToolName, Box<dyn Tool>>,
) -> AgentRuntime {
    let mut tools: HashMap<ToolName, Box<dyn Tool>> = HashMap::new();
    tools.insert("Echo".to_string(), Box::new(EchoTool));
    tools.extend(extra);
    AgentRuntime::new(
        RuntimeCoreDeps {
            llm_client: llm,
            tools,
            memory_store: Arc::new(DummyMemoryStore),
            default_model_config: ModelConfig {
                provider: LLMProvider::Custom("mock".to_string()),
                model: "mock".to_string(),
                base_url: None,
                temperature: None,
                max_tokens: None,
                api_key: None,
                ..Default::default()
            },
            model_overrides: HashMap::new(),
            failover_chain: vec![],
            disk_output: Some(disk),
            security: Arc::new(SecurityLayer::new(PermissionMode::BypassPermissions)),
            sandbox_mode: false,
            prompt_config: RuntimePromptConfig::default(),
        },
        RuntimeMemoryOptions {
            memory_pipeline: None,
            memory_pipeline_settings: None,
            memory_project_autosave_enabled: false,
            session_notifications: None,
            automem: None,
            automem_base_path: None,
        },
        RuntimeToolPolicy {
            tool_name_deny: vec![],
            claude_gating: AgentClaudeToolGating::default(),
            expose_skill_on_explore_plan: false,
        },
    )
}

fn make_task(loop_limits: AgentLoopLimits) -> Task {
    Task {
        id: Uuid::new_v4(),
        agent_type: AgentType::new("general-purpose"),
        prompt: "test".to_string(),
        context: TaskContext {
            session_id: Uuid::new_v4(),
            working_directory: ".".to_string(),
            environment: HashMap::new(),
            user_id: None,
            system_prompt_append: None,
            context_injections: vec![],
            nested_model_override: None,
            nested_worktree_path: None,
            nested_worktree_repo_root: None,
            nested_cancel: None,
            channel_progress_tx: None,
            live_trace_tx: None,
            tool_deny_names: vec![],
            tool_deny_prefixes: vec![],
            user_vision_images: vec![],
            budget: TaskBudget::default(),
            loop_limits,
            chat_turn: None,
        },
        created_at: chrono::Utc::now(),
    }
}

fn echo_call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "Echo".into(),
        input: serde_json::json!({ "text": id }),
    }
}

#[tokio::test]
async fn max_tool_calls_drains_synthetic_results_for_unexecuted_calls() {
    let temp = TempDir::new().unwrap();
    let disk = DiskTaskOutput::new(temp.path().to_path_buf());
    // 3 个 Echo 调用,max_tool_calls=1:第 2 个触发上限,第 2/3 个必须补合成 result。
    let first = tool_call_response(vec![echo_call("c1"), echo_call("c2"), echo_call("c3")]);
    let llm = Arc::new(MockLLM::new(vec![first, text_response("done")]));
    let runtime = make_runtime(llm, disk.clone());
    let limits = AgentLoopLimits {
        max_tool_calls: 1,
        ..Default::default()
    };
    let task = make_task(limits);
    let task_id = task.id;
    let _ = runtime.execute_task(task).await;

    let log = disk.tail(task_id, 64 * 1024).unwrap();
    assert!(
        log.contains("reason=max_tool_calls"),
        "应记录 max_tool_calls 终止:{log}"
    );
    assert!(
        log.contains("[tool_pairing_drain] count=2 reason=max_tool_calls"),
        "应为 2 个未执行调用排空合成 result:{log}"
    );
    let synthetic_count = log.matches("[tool_synthetic_result]").count();
    assert_eq!(synthetic_count, 2, "c2/c3 各应有一条合成 result 日志:{log}");
    // c1 真实执行过
    assert!(log.contains("[tool_call_end]"), "c1 应真实执行:{log}");
}

#[tokio::test]
async fn cooperative_cancel_drains_remaining_tool_calls() {
    let temp = TempDir::new().unwrap();
    let disk = DiskTaskOutput::new(temp.path().to_path_buf());
    // 第一个工具在执行中置位取消旗标:后续 2 个 tool_call 未执行,必须补合成 result。
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let first = tool_call_response(vec![
        ToolCall {
            id: "cancel".into(),
            name: "SetCancel".into(),
            input: serde_json::json!({}),
        },
        echo_call("c2"),
        echo_call("c3"),
    ]);
    let llm = Arc::new(MockLLM::new(vec![first, text_response("done")]));
    let runtime = make_runtime_with_cancel_tool(llm, disk.clone(), Arc::clone(&flag));
    let mut task = make_task(AgentLoopLimits::default());
    task.context.nested_cancel = Some(flag);
    let task_id = task.id;
    let _ = runtime.execute_task(task).await;

    let log = disk.tail(task_id, 64 * 1024).unwrap();
    assert!(
        log.contains("[tool_pairing_drain] count=2 reason=cooperative_cancel"),
        "取消时应为 2 个未执行调用补合成 result:{log}"
    );
    let real_starts = log.matches("[tool_call_start]").count();
    assert_eq!(real_starts, 1, "旗标置位后只有 SetCancel 应真实执行:{log}");
}
