//! Tool dispatch: safe execution, cancel scopes, read-only batches, pairing invariant.

use super::agentic_turn::{
    MessageAppendSink, TurnToolBatchOutcome, TurnToolCancel, TurnToolCancelOutcome, TurnToolCtx,
    TurnToolState,
};
use super::artifacts::extract_artifacts;
use super::budget::{tick_budget, tool_blocked_under_degrade};
use super::evidence;
use super::live_trace_emit::{emit_artifacts_ready, emit_tool_call_progress};
use super::logging::RunLogger;
use super::session_activity::{ActivityReason, SessionActivityGuard};
use super::tool_result_injection;
use super::AgentRuntime;
use anycode_core::prelude::*;
use futures::future::join_all;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_READONLY_TOOL_CONCURRENCY: usize = 10;
const TOOL_PROGRESS_INTERVAL: Duration = Duration::from_secs(2);

fn spawn_tool_progress(
    live_trace_tx: &Option<tokio::sync::mpsc::UnboundedSender<LiveTraceEvent>>,
    turn: usize,
    tool_idx: usize,
    tool_call: &ToolCall,
    t0: Instant,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (done_tx, mut done_rx) = tokio::sync::oneshot::channel::<()>();
    let live_tx = live_trace_tx.clone();
    let tool_call = tool_call.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TOOL_PROGRESS_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    emit_tool_call_progress(
                        &live_tx,
                        turn,
                        tool_idx,
                        &tool_call,
                        t0.elapsed().as_millis(),
                    );
                }
                _ = &mut done_rx => break,
            }
        }
    });
    (done_tx, handle)
}

fn stop_tool_progress(
    done_tx: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
) {
    let _ = done_tx.send(());
    handle.abort();
}

/// How a tool responds to cooperative cancel while running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolCancelPolicy {
    Cancel,
    Block,
}

pub(super) fn tool_cancel_policy(name: &str) -> ToolCancelPolicy {
    match name {
        "Bash" | "PowerShell" | "Task" | "Agent" | "Sleep" => ToolCancelPolicy::Cancel,
        _ => ToolCancelPolicy::Block,
    }
}

pub(super) fn is_readonly_tool(name: &str) -> bool {
    matches!(
        name,
        "Glob" | "Grep" | "Read" | "FileRead" | "WebFetch" | "WebSearch" | "SemanticSearch"
    )
}

struct ToolBatch {
    concurrent: bool,
    calls: Vec<ToolCall>,
}

fn partition_tool_calls(calls: Vec<ToolCall>) -> Vec<ToolBatch> {
    let mut batches: Vec<ToolBatch> = Vec::new();
    for call in calls {
        let concurrent = is_readonly_tool(&call.name);
        if let Some(last) = batches.last_mut() {
            if last.concurrent && concurrent {
                last.calls.push(call);
                continue;
            }
        }
        batches.push(ToolBatch {
            concurrent,
            calls: vec![call],
        });
    }
    batches
}

impl AgentRuntime {
    pub(super) async fn dispatch_turn_tool_calls(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        cancel: &TurnToolCancel<'_>,
        sink: &mut MessageAppendSink<'_>,
        tool_calls: Vec<ToolCall>,
        record_evidence: bool,
        cancel_outcome: TurnToolCancelOutcome,
    ) -> Result<TurnToolBatchOutcome, CoreError> {
        let batches = partition_tool_calls(tool_calls.clone());
        // 配对不变量(CORE-13):每个已记录的 tool_use 都必须有 tool_result,
        // 否则下一轮请求 Anthropic 会以 400 拒绝 dangling tool_use。
        // `pending` 始终等于「尚未产生 result 的调用」——任何提前出口
        // (cancel / max_tool_calls / budget)都必须先排空它再返回。
        let mut pending: Vec<ToolCall> = tool_calls;
        for batch in batches {
            if cancel.cancelled() {
                self.emit_synthetic_for_remaining(
                    logger,
                    ctx,
                    state,
                    sink,
                    &mut pending,
                    "cooperative_cancel",
                )
                .await;
                return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
            }
            if batch.concurrent && batch.calls.len() > 1 {
                let outcome = self
                    .dispatch_readonly_batch(
                        logger,
                        ctx,
                        state,
                        cancel,
                        sink,
                        batch.calls,
                        record_evidence,
                        cancel_outcome,
                        &mut pending,
                    )
                    .await?;
                if !matches!(outcome, TurnToolBatchOutcome::Ok) {
                    self.emit_synthetic_for_remaining(
                        logger,
                        ctx,
                        state,
                        sink,
                        &mut pending,
                        drain_reason(&outcome),
                    )
                    .await;
                    return Ok(outcome);
                }
            } else {
                for tool_call in batch.calls {
                    let outcome = self
                        .dispatch_single_tool_call(
                            logger,
                            ctx,
                            state,
                            cancel,
                            sink,
                            tool_call,
                            record_evidence,
                            cancel_outcome,
                            &mut pending,
                        )
                        .await?;
                    if !matches!(outcome, TurnToolBatchOutcome::Ok) {
                        self.emit_synthetic_for_remaining(
                            logger,
                            ctx,
                            state,
                            sink,
                            &mut pending,
                            drain_reason(&outcome),
                        )
                        .await;
                        return Ok(outcome);
                    }
                }
            }
        }
        Ok(TurnToolBatchOutcome::Ok)
    }

    async fn dispatch_readonly_batch(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        cancel: &TurnToolCancel<'_>,
        sink: &mut MessageAppendSink<'_>,
        tool_calls: Vec<ToolCall>,
        record_evidence: bool,
        cancel_outcome: TurnToolCancelOutcome,
        pending: &mut Vec<ToolCall>,
    ) -> Result<TurnToolBatchOutcome, CoreError> {
        let mut planned: Vec<(ToolCall, usize)> = Vec::new();
        for tool_call in tool_calls {
            if cancel.cancelled() {
                self.emit_synthetic_for_planned(
                    logger,
                    ctx,
                    state,
                    sink,
                    &planned,
                    "cooperative_cancel",
                )
                .await;
                for (done, _) in &planned {
                    mark_tool_result_emitted(pending, &done.id);
                }
                return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
            }
            let outcome = self
                .prepare_tool_dispatch(logger, ctx, state, cancel, cancel_outcome)
                .await?;
            if !matches!(outcome, TurnToolBatchOutcome::Ok) {
                return Ok(outcome);
            }
            if budget_degrade_blocks(state, &tool_call) {
                let idx = state.total_tool_calls;
                self.finalize_budget_denied(logger, ctx, state, sink, &tool_call, idx)
                    .await;
                mark_tool_result_emitted(pending, &tool_call.id);
                continue;
            }
            planned.push((tool_call, state.total_tool_calls));
        }

        let chunk_size = MAX_READONLY_TOOL_CONCURRENCY.max(1);
        for chunk in planned.chunks(chunk_size) {
            let _activity =
                SessionActivityGuard::start(logger.clone(), ctx.task_id, ActivityReason::ToolExec);
            let futures = chunk
                .iter()
                .map(|(tool_call, tool_idx)| {
                    self.execute_tool_call_with_policy(logger, ctx, *tool_idx, tool_call, cancel)
                })
                .collect::<Vec<_>>();
            let results = join_all(futures).await;
            for ((tool_call, tool_idx), (tool_result, elapsed_ms)) in
                chunk.iter().zip(results.into_iter())
            {
                self.finalize_tool_call(
                    logger,
                    ctx,
                    state,
                    sink,
                    tool_call,
                    *tool_idx,
                    tool_result,
                    elapsed_ms,
                    record_evidence,
                )
                .await;
                mark_tool_result_emitted(pending, &tool_call.id);
            }
            if cancel.cancelled() {
                return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
            }
        }
        Ok(TurnToolBatchOutcome::Ok)
    }

    async fn dispatch_single_tool_call(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        cancel: &TurnToolCancel<'_>,
        sink: &mut MessageAppendSink<'_>,
        tool_call: ToolCall,
        record_evidence: bool,
        cancel_outcome: TurnToolCancelOutcome,
        pending: &mut Vec<ToolCall>,
    ) -> Result<TurnToolBatchOutcome, CoreError> {
        if cancel.cancelled() {
            return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
        }
        let outcome = self
            .prepare_tool_dispatch(logger, ctx, state, cancel, cancel_outcome)
            .await?;
        if !matches!(outcome, TurnToolBatchOutcome::Ok) {
            return Ok(outcome);
        }
        let tool_idx = state.total_tool_calls;
        if budget_degrade_blocks(state, &tool_call) {
            self.finalize_budget_denied(logger, ctx, state, sink, &tool_call, tool_idx)
                .await;
            mark_tool_result_emitted(pending, &tool_call.id);
            return Ok(TurnToolBatchOutcome::Ok);
        }
        let _activity =
            SessionActivityGuard::start(logger.clone(), ctx.task_id, ActivityReason::ToolExec);
        let (tool_result, elapsed_ms) = self
            .execute_tool_call_with_policy(logger, ctx, tool_idx, &tool_call, cancel)
            .await;
        self.finalize_tool_call(
            logger,
            ctx,
            state,
            sink,
            &tool_call,
            tool_idx,
            tool_result,
            elapsed_ms,
            record_evidence,
        )
        .await;
        mark_tool_result_emitted(pending, &tool_call.id);
        if cancel.cancelled() {
            return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
        }
        Ok(TurnToolBatchOutcome::Ok)
    }

    async fn prepare_tool_dispatch(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        cancel: &TurnToolCancel<'_>,
        cancel_outcome: TurnToolCancelOutcome,
    ) -> Result<TurnToolBatchOutcome, CoreError> {
        if cancel.cancelled() {
            return Ok(TurnToolBatchOutcome::Cancelled(cancel_outcome));
        }
        if tick_budget(logger, ctx.task_id, &mut state.budget_state) {
            logger.line(
                ctx.task_id,
                "[task_end] status=failed reason=budget_exceeded",
            );
            return Ok(TurnToolBatchOutcome::BudgetExceeded);
        }
        state.total_tool_calls += 1;
        if state.total_tool_calls > ctx.loop_limits.max_tool_calls {
            logger.line(
                ctx.task_id,
                &format!(
                    "[task_end] status=failed reason=max_tool_calls({})",
                    ctx.loop_limits.max_tool_calls
                ),
            );
            return Ok(TurnToolBatchOutcome::MaxToolCalls);
        }
        Ok(TurnToolBatchOutcome::Ok)
    }

    async fn execute_tool_call_with_policy(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        tool_idx: usize,
        tool_call: &ToolCall,
        cancel: &TurnToolCancel<'_>,
    ) -> (ToolOutput, u128) {
        tool_result_injection::log_tool_call_input(
            logger,
            &ctx.live_trace_tx,
            ctx.task_id,
            ctx.turn,
            tool_idx,
            tool_call,
        );
        tool_result_injection::log_tool_call_start(
            logger,
            &ctx.live_trace_tx,
            ctx.task_id,
            ctx.turn,
            tool_idx,
            tool_call,
        );

        let policy = tool_cancel_policy(&tool_call.name);
        let t0 = Instant::now();
        let (progress_done, progress_handle) =
            spawn_tool_progress(&ctx.live_trace_tx, ctx.turn, tool_idx, tool_call, t0);

        let tool_result = if policy == ToolCancelPolicy::Cancel {
            tokio::select! {
                biased;
                _ = wait_cancel_flag(cancel.clone_flag()) => {
                    stop_tool_progress(progress_done, progress_handle);
                    synthetic_tool_output("cooperative_cancel")
                }
                result = self.execute_tool_call(
                    ctx.task_id,
                    ctx.agent_type,
                    ctx.working_directory,
                    tool_call,
                ) => {
                    stop_tool_progress(progress_done, progress_handle);
                    match result {
                    Ok(out) => out,
                    Err(e) => {
                        logger.turn_error(ctx.task_id, ctx.turn, &tool_call.name, &e.to_string());
                        synthetic_tool_error(&e)
                    }
                }
                }
            }
        } else {
            let result = self
                .execute_tool_call(
                    ctx.task_id,
                    ctx.agent_type,
                    ctx.working_directory,
                    tool_call,
                )
                .await;
            stop_tool_progress(progress_done, progress_handle);
            match result {
                Ok(out) => out,
                Err(e) => {
                    logger.turn_error(ctx.task_id, ctx.turn, &tool_call.name, &e.to_string());
                    synthetic_tool_error(&e)
                }
            }
        };

        (tool_result, t0.elapsed().as_millis())
    }

    async fn finalize_budget_denied(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        sink: &mut MessageAppendSink<'_>,
        tool_call: &ToolCall,
        tool_idx: usize,
    ) {
        logger.line(
            ctx.task_id,
            &format!(
                "[tool_denied] name={} reason=budget_degrade",
                tool_call.name
            ),
        );
        let tool_result = ToolOutput {
            result: serde_json::json!({ "error": "tool blocked under budget degradation" }),
            error: Some("tool blocked under budget degradation".into()),
            duration_ms: 0,
        };
        self.finalize_tool_call(
            logger,
            ctx,
            state,
            sink,
            tool_call,
            tool_idx,
            tool_result,
            0,
            false,
        )
        .await;
    }

    async fn finalize_tool_call(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        sink: &mut MessageAppendSink<'_>,
        tool_call: &ToolCall,
        tool_idx: usize,
        tool_result: ToolOutput,
        elapsed_ms: u128,
        record_evidence: bool,
    ) {
        tool_result_injection::log_tool_call_end(
            logger,
            &ctx.live_trace_tx,
            ctx.task_id,
            ctx.turn,
            tool_idx,
            tool_call,
            &tool_result,
            elapsed_ms,
            &mut state.progress_seq,
        );
        let prepared = tool_result_injection::prepare_tool_result_message(
            ctx.task_id,
            tool_call,
            &tool_result,
            logger,
        );
        if record_evidence {
            evidence::append_tool_evidence(ctx.task_id, &tool_call.name, &prepared.for_hook);
        }
        if let Some(v) = &ctx.verification {
            if let Ok(mut g) = v.lock() {
                g.note_tool(&tool_call.name, &tool_call.input, &prepared.for_hook);
            }
        }
        if tool_call.name == "Bash" {
            let command = tool_call
                .input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            self.maybe_record_verify_recipe(
                ctx.session_label,
                ctx.task_id,
                command,
                &prepared.for_hook,
                ctx.working_directory,
            )
            .await;
        }
        sink.push(prepared.message).await;
        // automem fork 的工具轨迹不进入规则管线 episode（避免记忆代理自我污染）。
        if !super::automem::is_automem_agent_type(ctx.agent_type.as_str()) {
            self.pipeline_memory_hook_tool_result(
                ctx.session_label,
                ctx.task_id,
                &tool_call.name,
                &prepared.for_hook,
            )
            .await;
        }
        self.maybe_session_notify_tool_result(
            ctx.session_label,
            ctx.task_id,
            ctx.turn,
            &tool_call.name,
            &prepared.for_hook,
            Some(ctx.working_directory),
        );
        let extracted = extract_artifacts(tool_call, &tool_result);
        if !extracted.is_empty() {
            // P1 申报点验收:交付物一经显式申报立即按类型验收,不通过当场返修。
            let acceptance = self
                .check_declared_deliverables(
                    &extracted,
                    &mut state.checked_deliverables,
                    std::path::Path::new(ctx.working_directory),
                )
                .await;
            if acceptance.checked > 0 {
                super::delivery_metrics::record_declaration_checks(
                    &ctx.task_id,
                    ctx.session_label,
                    &acceptance.results,
                );
                let failed = acceptance
                    .results
                    .iter()
                    .filter(|r| {
                        r.outcome != anycode_core::VerificationOutcome::Passed
                            && r.severity != anycode_core::GateSeverity::Info
                    })
                    .count();
                logger.line(
                    ctx.task_id,
                    &format!(
                        "[delivery_acceptance] checked={} failed={}",
                        acceptance.checked, failed
                    ),
                );
                if let Some(msg) = acceptance.repair_message() {
                    sink.push(super::context_user_message(msg)).await;
                }
            }
        }
        emit_artifacts_ready(
            &ctx.live_trace_tx,
            ctx.turn,
            tool_idx,
            tool_call,
            &extracted,
        );
        state.artifacts.extend(extracted);
    }

    /// 提前出口前排空 pending:为每个尚未产生 result 的调用补合成
    /// tool_result,保住 pairing 不变量(否则下一轮请求 400)。
    async fn emit_synthetic_for_remaining(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        sink: &mut MessageAppendSink<'_>,
        pending: &mut Vec<ToolCall>,
        reason: &str,
    ) {
        if pending.is_empty() {
            return;
        }
        let base = state.total_tool_calls;
        let planned: Vec<(ToolCall, usize)> = pending
            .drain(..)
            .enumerate()
            .map(|(i, c)| (c, base + i + 1))
            .collect();
        logger.line(
            ctx.task_id,
            &format!(
                "[tool_pairing_drain] count={} reason={}",
                planned.len(),
                reason
            ),
        );
        self.emit_synthetic_for_planned(logger, ctx, state, sink, &planned, reason)
            .await;
    }

    async fn emit_synthetic_for_planned(
        &self,
        logger: &RunLogger,
        ctx: &TurnToolCtx<'_>,
        state: &mut TurnToolState,
        sink: &mut MessageAppendSink<'_>,
        planned: &[(ToolCall, usize)],
        reason: &str,
    ) {
        for (tool_call, tool_idx) in planned {
            logger.tool_synthetic_result(ctx.task_id, ctx.turn, *tool_idx, &tool_call.name, reason);
            let output = synthetic_tool_output(reason);
            tool_result_injection::log_tool_call_end(
                logger,
                &ctx.live_trace_tx,
                ctx.task_id,
                ctx.turn,
                *tool_idx,
                tool_call,
                &output,
                0,
                &mut state.progress_seq,
            );
            let prepared = tool_result_injection::prepare_tool_result_message(
                ctx.task_id,
                tool_call,
                &output,
                logger,
            );
            sink.push(prepared.message).await;
        }
    }
}

impl TurnToolCancel<'_> {
    fn clone_flag(&self) -> Option<Arc<AtomicBool>> {
        match self {
            TurnToolCancel::Coop(flag) => flag.clone(),
            // 嵌套任务内 Cancel-policy 工具可中途抢占（ADR 010 协作式取消）。
            TurnToolCancel::Nested(ctx) => ctx.nested_cancel.clone(),
        }
    }
}

async fn wait_cancel_flag(flag: Option<Arc<AtomicBool>>) {
    let Some(flag) = flag else {
        std::future::pending::<()>().await;
        return;
    };
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// 调用产生 result 后从 pending 移除(id 唯一;位置无关,保持顺序)。
fn mark_tool_result_emitted(pending: &mut Vec<ToolCall>, id: &str) {
    if let Some(pos) = pending.iter().position(|c| c.id == id) {
        pending.remove(pos);
    }
}

/// 非 Ok 出口对应的合成 result 原因串(供 drain 与日志使用)。
fn drain_reason(outcome: &TurnToolBatchOutcome) -> &'static str {
    match outcome {
        TurnToolBatchOutcome::Cancelled(_) => "cooperative_cancel",
        TurnToolBatchOutcome::MaxToolCalls => "max_tool_calls",
        TurnToolBatchOutcome::BudgetExceeded => "budget_exceeded",
        TurnToolBatchOutcome::Ok => "unknown",
    }
}

fn synthetic_tool_output(reason: &str) -> ToolOutput {
    ToolOutput {
        result: serde_json::json!({ "cancelled": true, "reason": reason }),
        error: Some(format!("cancelled: {reason}")),
        duration_ms: 0,
    }
}

fn synthetic_tool_error(err: &CoreError) -> ToolOutput {
    ToolOutput {
        result: serde_json::json!({ "error": err.to_string() }),
        error: Some(err.to_string()),
        duration_ms: 0,
    }
}

fn budget_degrade_blocks(state: &TurnToolState, tool_call: &ToolCall) -> bool {
    state
        .budget_state
        .as_ref()
        .is_some_and(|s| tool_blocked_under_degrade(s, &tool_call.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_consecutive_readonly_tools() {
        let calls = vec![
            ToolCall {
                id: "1".into(),
                name: "Glob".into(),
                input: serde_json::json!({}),
            },
            ToolCall {
                id: "2".into(),
                name: "Grep".into(),
                input: serde_json::json!({}),
            },
            ToolCall {
                id: "3".into(),
                name: "Bash".into(),
                input: serde_json::json!({}),
            },
        ];
        let batches = partition_tool_calls(calls);
        assert_eq!(batches.len(), 2);
        assert!(batches[0].concurrent);
        assert_eq!(batches[0].calls.len(), 2);
        assert!(!batches[1].concurrent);
    }

    #[test]
    fn bash_tools_are_cancellable() {
        assert_eq!(tool_cancel_policy("Bash"), ToolCancelPolicy::Cancel);
        assert_eq!(tool_cancel_policy("Edit"), ToolCancelPolicy::Block);
    }

    #[test]
    fn nested_cancel_clone_flag_returns_context_flag() {
        let flag = Arc::new(AtomicBool::new(false));
        let ctx = anycode_core::TaskContext {
            session_id: uuid::Uuid::new_v4(),
            working_directory: ".".into(),
            environment: Default::default(),
            user_id: None,
            system_prompt_append: None,
            context_injections: vec![],
            nested_model_override: None,
            nested_worktree_path: None,
            nested_worktree_repo_root: None,
            nested_cancel: Some(Arc::clone(&flag)),
            channel_progress_tx: None,
            live_trace_tx: None,
            tool_deny_names: vec![],
            tool_deny_prefixes: vec![],
            user_vision_images: vec![],
            budget: anycode_core::TaskBudget::default(),
            loop_limits: Default::default(),
            chat_turn: None,
        };
        let cancel = TurnToolCancel::Nested(&ctx);
        let cloned = cancel.clone_flag().expect("nested flag");
        flag.store(true, Ordering::SeqCst);
        assert!(cloned.load(Ordering::SeqCst));
    }

    #[test]
    fn coop_cancel_clone_flag_passthrough() {
        let flag = Arc::new(AtomicBool::new(false));
        let cancel = TurnToolCancel::Coop(Some(Arc::clone(&flag)));
        let cloned = cancel.clone_flag().expect("coop flag");
        flag.store(true, Ordering::SeqCst);
        assert!(cloned.load(Ordering::SeqCst));
        assert!(TurnToolCancel::Coop(None).clone_flag().is_none());
    }
}
