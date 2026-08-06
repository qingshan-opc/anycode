//! Shared agentic loop helpers (Template Method + Bridge for `execute_task` / `execute_turn`).

use super::agentic_loop::{nested_coop_cancelled, opt_coop_cancelled, task_cancelled_failure};
use super::discoverable_verification::SessionVerificationState;
use super::logging::RunLogger;
use super::AgentRuntime;
use anycode_core::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Per-turn context shared by task and TUI turn paths.
pub(super) struct TurnToolCtx<'a> {
    pub task_id: TaskId,
    pub agent_type: &'a AgentType,
    pub working_directory: &'a str,
    pub session_label: &'a str,
    pub turn: usize,
    pub loop_limits: AgentLoopLimits,
    pub live_trace_tx: Option<tokio::sync::mpsc::UnboundedSender<LiveTraceEvent>>,
    pub verification: Option<Arc<std::sync::Mutex<SessionVerificationState>>>,
}

/// Mutable counters/state updated while dispatching tool calls in a turn.
pub(super) struct TurnToolState {
    pub total_tool_calls: usize,
    pub artifacts: Vec<anycode_core::Artifact>,
    pub budget_state: Option<super::budget::RuntimeBudgetState>,
    pub progress_seq: u32,
}

pub(super) enum TurnToolCancel<'a> {
    /// Nested task cooperative cancel (`execute_task`).
    Nested(&'a anycode_core::TaskContext),
    /// TUI / channel cooperative cancel flag.
    Coop(Option<Arc<AtomicBool>>),
}

impl TurnToolCancel<'_> {
    pub(super) fn cancelled(&self) -> bool {
        match self {
            Self::Nested(ctx) => nested_coop_cancelled(ctx),
            Self::Coop(flag) => opt_coop_cancelled(flag),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnToolCancelOutcome {
    TaskCancelled,
    TurnCancelled,
}

impl TurnToolCancelOutcome {
    pub fn into_task_result(self) -> Option<anycode_core::TaskResult> {
        match self {
            Self::TaskCancelled => Some(task_cancelled_failure()),
            _ => None,
        }
    }

    pub fn into_core_error(self) -> Option<CoreError> {
        match self {
            Self::TurnCancelled => Some(CoreError::CooperativeCancel),
            _ => None,
        }
    }
}

/// Where tool_result messages are appended (Bridge: Vec vs shared mutex history).
pub(super) enum MessageAppendSink<'a> {
    Vec(&'a mut Vec<Message>),
    Shared(&'a Arc<Mutex<Vec<Message>>>),
}

impl MessageAppendSink<'_> {
    pub(super) async fn push(&mut self, message: Message) {
        match self {
            Self::Vec(v) => v.push(message),
            Self::Shared(m) => {
                let mut g = m.lock().await;
                g.push(message);
            }
        }
    }

    /// 历史末尾不是 assistant 时补插该消息（tool-recovery 的首跳响应可能尚未入史）。
    pub(super) async fn push_assistant_if_tail_missing(&mut self, message: &Message) {
        match self {
            Self::Vec(v) => {
                if v.last().is_none_or(|m| m.role != MessageRole::Assistant) {
                    v.push(message.clone());
                }
            }
            Self::Shared(m) => {
                let mut g = m.lock().await;
                if g.last()
                    .is_none_or(|last| last.role != MessageRole::Assistant)
                {
                    g.push(message.clone());
                }
            }
        }
    }

    /// 当前历史的完整快照（LLM 请求入参；两种容器语义一致）。
    pub(super) async fn snapshot(&self) -> Vec<Message> {
        match self {
            Self::Vec(v) => (*v).clone(),
            Self::Shared(m) => m.lock().await.clone(),
        }
    }
}

/// 弱本地模型首轮无 tool_call 的恢复结果；各主循环映射到自己的出口类型。
pub(super) enum NoToolRecovery {
    /// 某一跳重试产出了 tool_calls，继续主循环。
    Recovered(LLMResponse),
    /// 预算 hard-stop（调用方走各自的预算出口）。
    BudgetExceeded,
    /// 恢复用的 LLM 调用失败。
    LlmFailed(CoreError),
    /// 两次 nudge 后仍无 tool_call；负载为末次 assistant 正文（task 侧忽略）。
    Exhausted(String),
}

impl AgentRuntime {
    /// 弱本地模型首轮无 tool_call 的共享恢复流程：记录用量 → 注入 nudge → 重试，至多 2 次。
    /// 出口差异（`TaskResult` vs `TurnOutput`）由调用方映射；日志文本两侧一致。
    /// `partial_usage` 仅 turn 侧传入：恢复循环内只累加 input 最大值与 output 求和（不含 cache）。
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn recover_no_tool_response(
        &self,
        logger: &RunLogger,
        task_id: TaskId,
        initial: LLMResponse,
        turn_tool_schemas: &[ToolSchema],
        llm_config: &ModelConfig,
        budget_state: &mut Option<super::budget::RuntimeBudgetState>,
        sink: &mut MessageAppendSink<'_>,
        mut partial_usage: Option<&mut TurnTokenUsage>,
    ) -> NoToolRecovery {
        let mut response = initial;
        for attempt in 1..=2u8 {
            logger.line(
                task_id,
                &format!(
                    "[tool_recovery] turn=1 attempt={} reason=no_tool_response",
                    attempt
                ),
            );
            if let Some(usage) = partial_usage.as_deref_mut() {
                usage.max_input_tokens = usage.max_input_tokens.max(response.usage.input_tokens);
                usage.total_output_tokens += response.usage.output_tokens;
            }
            if super::budget::record_llm_usage(logger, task_id, budget_state, &response.usage) {
                return NoToolRecovery::BudgetExceeded;
            }
            sink.push_assistant_if_tail_missing(&response.message).await;
            sink.push(Message {
                id: Uuid::new_v4(),
                role: MessageRole::User,
                content: MessageContent::Text(if attempt == 1 {
                    anycode_llm::TOOL_RECOVERY_NUDGE.to_string()
                } else {
                    anycode_llm::TOOL_RECOVERY_NUDGE_FORCE_GLOB.to_string()
                }),
                timestamp: chrono::Utc::now(),
                metadata: HashMap::new(),
            })
            .await;
            let snapshot = sink.snapshot().await;
            match self
                .chat_with_failover(
                    &snapshot,
                    turn_tool_schemas.to_vec(),
                    llm_config,
                    task_id,
                    logger,
                )
                .await
            {
                Ok(r) => {
                    response = r;
                    sink.push(response.message.clone()).await;
                    if !response.tool_calls.is_empty() {
                        break;
                    }
                }
                Err(e) => return NoToolRecovery::LlmFailed(e),
            }
        }
        if response.tool_calls.is_empty() {
            let refusal = match &response.message.content {
                MessageContent::Text(text) => text.clone(),
                _ => String::new(),
            };
            NoToolRecovery::Exhausted(refusal)
        } else {
            NoToolRecovery::Recovered(response)
        }
    }
}

/// Outcome after processing a batch of tool calls for one assistant turn.
pub(super) enum TurnToolBatchOutcome {
    Ok,
    Cancelled(TurnToolCancelOutcome),
    MaxToolCalls,
    BudgetExceeded,
}
