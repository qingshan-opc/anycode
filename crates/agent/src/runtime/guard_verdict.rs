//! Shared CompletionGuard verdict handling for `execute_task` / `execute_turn`.
//!
//! 两条主循环在「无 tool_call 的回合」跑同一套守卫决策骨架（evidence repair 检查、
//! repair 注入、verification 标记、memory/notify 钩子），仅出口类型不同
//! （`TaskResult` vs `TurnOutput` 终止原因）。本模块收敛骨架为单一方法，
//! 调用方按 [`GuardVerdict`] 映射各自的 break / continue / return ——
//! Strategy 模式作用于出口，模板方法作用于骨架。

use super::agentic_turn::MessageAppendSink;
use super::completion_guard::GuardDecision;
use super::discoverable_verification::{self, SessionVerificationState};
use super::live_trace_emit;
use super::logging::RunLogger;
use super::{automem, context_user_message, AgentRuntime};
use anycode_core::prelude::*;
use anycode_core::{Artifact, ExpectedArtifact, GatePlan, TaskFamily};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// 守卫评估的只读入参（参数对象，两条主循环共用）。
pub(super) struct GuardEvalInput<'a> {
    pub task_id: TaskId,
    pub agent_type: &'a AgentType,
    pub working_directory: &'a str,
    pub session_label: &'a str,
    pub turn: usize,
    pub task_family: Option<TaskFamily>,
    pub gate_plan: Option<&'a GatePlan>,
    pub expected_artifacts: &'a [ExpectedArtifact],
    pub artifacts: &'a [Artifact],
    /// 参与 evidence repair 判定与钩子的 assistant 正文
    /// （task 侧为本轮末条正文，turn 侧为最后一条非空正文）。
    pub assistant_text: &'a str,
    pub live_trace_tx: &'a Option<UnboundedSender<LiveTraceEvent>>,
    pub verification: &'a Arc<std::sync::Mutex<SessionVerificationState>>,
    pub progress_seq: u32,
    /// turn 侧为 `true`：Complete 打 `[verification_finished] passed=1`，
    /// 且 evidence/repair/complete 三分支发 verify 阶段 live trace 事件；
    /// task 侧为 `false`：不发事件，Repair 分支改打
    /// `[verification_finished] passed={all_passed}`（report 存在时）。
    pub turn_style_verify_markers: bool,
}

/// 跨轮可变的守卫状态（修复计数与诊断）。
#[derive(Default)]
pub(super) struct GuardLoopState {
    pub repairs_used: u32,
    pub evidence_repairs_used: u32,
    pub last_repair_diagnostics: Option<String>,
}

/// 守卫结论；控制流出口（break / continue / return）由调用方决定。
pub(super) enum GuardVerdict {
    /// 验证通过，memory/notify 钩子已触发；调用方结束主循环。
    Completed,
    /// evidence repair 或 gate repair 消息已注入历史；调用方进入下一轮。
    RepairInjected,
    Partial {
        repair_message: Option<String>,
    },
    Failed {
        repair_message: Option<String>,
    },
}

impl AgentRuntime {
    /// 评估完成守卫并执行公共后续动作（标记日志、live trace、修复注入、钩子）。
    /// 日志与事件文本同两条主循环的历史输出逐字一致。
    pub(super) async fn evaluate_completion_guard(
        &self,
        logger: &RunLogger,
        input: &GuardEvalInput<'_>,
        state: &mut GuardLoopState,
        sink: &mut MessageAppendSink<'_>,
    ) -> GuardVerdict {
        let guard_out = self
            .completion_guard
            .evaluate(
                &input.task_id.to_string(),
                input.task_family,
                input.gate_plan,
                input.expected_artifacts,
                input.artifacts,
                Path::new(input.working_directory),
                state.repairs_used,
                state.last_repair_diagnostics.as_deref(),
            )
            .await;
        match guard_out.decision {
            GuardDecision::Complete => {
                let verification_snapshot = input
                    .verification
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                if let Some(msg) = discoverable_verification::maybe_evidence_repair(
                    &verification_snapshot,
                    input.assistant_text,
                    state.evidence_repairs_used,
                ) {
                    state.evidence_repairs_used += 1;
                    state.last_repair_diagnostics = Some(msg.clone());
                    let marker = format!(
                        "[evidence_repair_requested] repairs_used={}",
                        state.evidence_repairs_used
                    );
                    logger.line(input.task_id, &marker);
                    if input.turn_style_verify_markers {
                        live_trace_emit::try_emit(
                            input.live_trace_tx,
                            LiveTraceEvent::ProgressUpdate {
                                turn: input.turn as u32,
                                seq: input.progress_seq.saturating_add(1),
                                phase: "verify".into(),
                                work_stage: Some("discover".into()),
                                summary: marker,
                                next: Some("discover and run official verification".into()),
                                discovery: None,
                                evidence_refs: vec![],
                            },
                        );
                    }
                    sink.push(context_user_message(msg)).await;
                    return GuardVerdict::RepairInjected;
                }
                if input.turn_style_verify_markers {
                    let marker = format!(
                        "[verification_finished] passed=1 results={}",
                        guard_out
                            .report
                            .as_ref()
                            .map(|r| r.results.len())
                            .unwrap_or(0)
                    );
                    logger.line(input.task_id, &marker);
                    live_trace_emit::try_emit(
                        input.live_trace_tx,
                        LiveTraceEvent::ProgressUpdate {
                            turn: input.turn as u32,
                            seq: input.progress_seq.saturating_add(1),
                            phase: "verify".into(),
                            work_stage: Some("complete".into()),
                            summary: marker,
                            next: None,
                            discovery: None,
                            evidence_refs: vec![],
                        },
                    );
                }
                if !automem::is_automem_agent_type(input.agent_type.as_str()) {
                    self.pipeline_memory_hook_agent_turn(
                        input.session_label,
                        input.task_id,
                        input.turn,
                        input.assistant_text,
                    )
                    .await;
                }
                self.maybe_session_notify_agent_turn(
                    input.session_label,
                    input.task_id,
                    input.turn,
                    input.assistant_text,
                    Some(input.working_directory),
                );
                logger.line(
                    input.task_id,
                    &format!("[turn_end] turn={} tool_calls=0", input.turn),
                );
                GuardVerdict::Completed
            }
            GuardDecision::Repair => {
                let msg = guard_out.repair_message.unwrap_or_default();
                state.last_repair_diagnostics = Some(msg.clone());
                state.repairs_used += 1;
                let marker = format!(
                    "[repair_requested] repairs_used={} verification_started=1",
                    state.repairs_used
                );
                logger.line(input.task_id, &marker);
                if input.turn_style_verify_markers {
                    live_trace_emit::try_emit(
                        input.live_trace_tx,
                        LiveTraceEvent::ProgressUpdate {
                            turn: input.turn as u32,
                            seq: input.progress_seq.saturating_add(1),
                            phase: "verify".into(),
                            work_stage: Some("repair".into()),
                            summary: marker,
                            next: Some("fix gate failures then re-check".into()),
                            discovery: None,
                            evidence_refs: vec![],
                        },
                    );
                } else if let Some(report) = &guard_out.report {
                    logger.line(
                        input.task_id,
                        &format!(
                            "[verification_finished] passed={} results={}",
                            report.all_passed(),
                            report.results.len()
                        ),
                    );
                }
                sink.push(context_user_message(msg)).await;
                GuardVerdict::RepairInjected
            }
            GuardDecision::Partial => GuardVerdict::Partial {
                repair_message: guard_out.repair_message,
            },
            GuardDecision::Failed => GuardVerdict::Failed {
                repair_message: guard_out.repair_message,
            },
        }
    }
}
