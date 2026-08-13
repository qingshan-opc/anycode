//! Shared CompletionGuard verdict handling for `execute_task` / `execute_turn`.
//!
//! 两条主循环在「无 tool_call 的回合」跑同一套守卫决策骨架(evidence repair 检查、
//! repair 注入、verification 标记、memory/notify 钩子),仅出口类型不同
//! (`TaskResult` vs `TurnOutput` 终止原因)。本模块收敛骨架为单一方法,
//! 调用方按 [`GuardVerdict`] 映射各自的 break / continue / return ——
//! Strategy 模式作用于出口,模板方法作用于骨架。
//!
//! Complete 分支内的判定次序(全部先于放行):
//! 1. P2.9 子任务结果门禁——failed/partial 子任务不得作为完成证据;
//! 2. evidence repair——写了可验证文件但未跑栈相关验证(P1.5);
//! 3. P2.8 逃逸度量——以上预算耗尽后的放行打点;
//! 4. P0.1/P0.3 LLM grader——rubric 语义验收 + critic 对抗复核(default-to-refuted)。

use super::agentic_turn::MessageAppendSink;
use super::completion_guard::GuardDecision;
use super::discoverable_verification::{self, SessionVerificationState};
use super::grader::{self, GraderVerdict};
use super::live_trace_emit;
use super::logging::RunLogger;
use super::{automem, context_user_message, AgentRuntime};
use anycode_core::prelude::*;
use anycode_core::{Artifact, ExpectedArtifact, GatePlan, RubricItem, TaskFamily};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// grader 返修预算:语义判定给一次返修机会;再 refuted 则 Partial 放行并明示理由
/// (语义判定不应把任务钉死为 Failed)。
const MAX_GRADER_REPAIRS: u32 = 1;

/// 守卫评估的只读入参(参数对象,两条主循环共用)。
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
    /// (task 侧为本轮末条正文,turn 侧为最后一条非空正文)。
    pub assistant_text: &'a str,
    /// 原始用户意图,供 grader 生成/判定 rubric。
    pub task_prompt: &'a str,
    pub live_trace_tx: &'a Option<UnboundedSender<LiveTraceEvent>>,
    pub verification: &'a Arc<std::sync::Mutex<SessionVerificationState>>,
    pub progress_seq: u32,
    /// turn 侧为 `true`:Complete 打 `[verification_finished] passed=1`,
    /// 且 evidence/repair/complete 三分支发 verify 阶段 live trace 事件;
    /// task 侧为 `false`:不发事件,Repair 分支改打
    /// `[verification_finished] passed={all_passed}`(report 存在时)。
    pub turn_style_verify_markers: bool,
}

/// 跨轮可变的守卫状态(修复计数与诊断)。
#[derive(Default)]
pub(super) struct GuardLoopState {
    pub repairs_used: u32,
    pub evidence_repairs_used: u32,
    pub last_repair_diagnostics: Option<String>,
    /// P1.6:上轮失败 gate 集合,用于无进展熔断。
    pub last_failed_gates: Vec<String>,
    /// P2.9:子任务失败返修计数(上限 1)。
    pub subtask_repairs_used: u32,
    /// P0.1/P0.3:grader 返修计数与本会话缓存的 rubric。
    pub grader_repairs_used: u32,
    pub rubric: Option<Vec<RubricItem>>,
    /// rubric 生成已尝试过(失败不重复打 LLM)。
    pub rubric_attempted: bool,
}

/// 守卫结论;控制流出口(break / continue / return)由调用方决定。
pub(super) enum GuardVerdict {
    /// 验证通过,memory/notify 钩子已触发;调用方结束主循环。
    Completed,
    /// evidence repair 或 gate repair 消息已注入历史;调用方进入下一轮。
    RepairInjected,
    Partial {
        repair_message: Option<String>,
    },
    Failed {
        repair_message: Option<String>,
    },
}

impl AgentRuntime {
    /// 评估完成守卫并执行公共后续动作(标记日志、live trace、修复注入、钩子)。
    /// 日志与事件文本同两条主循环的历史输出逐字一致。
    pub(super) async fn evaluate_completion_guard(
        &self,
        logger: &RunLogger,
        input: &GuardEvalInput<'_>,
        state: &mut GuardLoopState,
        sink: &mut MessageAppendSink<'_>,
    ) -> GuardVerdict {
        let verification_snapshot = input
            .verification
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
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
                &state.last_failed_gates,
                &verification_snapshot.written_paths,
            )
            .await;
        super::delivery_metrics::record_guard_verdict(
            &input.task_id,
            input.session_label,
            match guard_out.decision {
                GuardDecision::Complete => "complete",
                GuardDecision::Repair => "repair",
                GuardDecision::Partial => "partial",
                GuardDecision::Failed => "failed",
            },
            state.repairs_used,
            guard_out.report.as_ref(),
        );
        // P2.8 逃逸度量:兜底触发(family 误判)与跳过(应跑未跑的分母)。
        if let Some(f) = guard_out.fallback_family {
            super::delivery_metrics::record_guard_fallback(
                &input.task_id,
                input.session_label,
                f.as_str(),
                guard_out
                    .report
                    .as_ref()
                    .map(|r| r.results.len())
                    .unwrap_or(0),
            );
        }
        if let Some(reason) = guard_out.skipped_reason {
            super::delivery_metrics::record_guard_skipped(
                &input.task_id,
                input.session_label,
                reason,
            );
        }
        match guard_out.decision {
            GuardDecision::Complete => {
                // 确定性门禁已有真实通过项时,守卫报告本身就是验证证据,
                // 不再要求 agent 额外跑命令(Info 级占位通过不算)。
                let gates_verified = guard_out.report.as_ref().is_some_and(|r| {
                    r.results.iter().any(|x| {
                        x.outcome == anycode_core::VerificationOutcome::Passed
                            && x.severity != anycode_core::GateSeverity::Info
                    })
                });
                // P2.9:failed/partial 子任务不得作为完成证据。
                if let Some(msg) = discoverable_verification::maybe_subtask_repair(
                    &verification_snapshot,
                    input.assistant_text,
                    state.subtask_repairs_used,
                ) {
                    state.subtask_repairs_used += 1;
                    let marker = format!(
                        "[subtask_repair_requested] repairs_used={}",
                        state.subtask_repairs_used
                    );
                    logger.line(input.task_id, &marker);
                    sink.push(context_user_message(msg)).await;
                    return GuardVerdict::RepairInjected;
                }
                if !gates_verified {
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
                    // P2.8:预算耗尽仍无验证的放行,记逃逸。
                    if discoverable_verification::verification_escape(
                        &verification_snapshot,
                        state.evidence_repairs_used,
                    ) {
                        super::delivery_metrics::record_verification_escape(
                            &input.task_id,
                            input.session_label,
                        );
                    }
                }
                // P0.1/P0.3:LLM grader(rubric 语义验收 + critic 对抗复核)。
                // 仅在守卫真的跑过门禁(report 存在)且有原始意图时启用。
                if self.completion_guard.grader_enabled()
                    && guard_out.report.is_some()
                    && !input.task_prompt.trim().is_empty()
                {
                    if let Some(verdict) = self.run_grader(input, state, &guard_out).await {
                        match verdict {
                            GraderVerdict::Pass => {
                                super::delivery_metrics::record_grader_verdict(
                                    &input.task_id,
                                    input.session_label,
                                    "pass",
                                    state.rubric.as_ref().map(|r| r.len()).unwrap_or(0),
                                    0,
                                );
                            }
                            GraderVerdict::Unavailable => {
                                super::delivery_metrics::record_grader_verdict(
                                    &input.task_id,
                                    input.session_label,
                                    "unavailable",
                                    0,
                                    0,
                                );
                            }
                            GraderVerdict::Refuted(msg) => {
                                super::delivery_metrics::record_grader_verdict(
                                    &input.task_id,
                                    input.session_label,
                                    "refuted",
                                    state.rubric.as_ref().map(|r| r.len()).unwrap_or(0),
                                    1,
                                );
                                logger.line(input.task_id, "[grader_verdict] verdict=refuted");
                                if state.grader_repairs_used < MAX_GRADER_REPAIRS {
                                    state.grader_repairs_used += 1;
                                    sink.push(context_user_message(msg)).await;
                                    return GuardVerdict::RepairInjected;
                                }
                                // 语义判定不钉死任务:预算耗尽后 Partial 放行并明示理由。
                                return GuardVerdict::Partial {
                                    repair_message: Some(msg),
                                };
                            }
                        }
                    }
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
                // P1.6:记录本轮失败 gate 集合,供下一轮无进展判定。
                state.last_failed_gates = guard_out
                    .report
                    .as_ref()
                    .map(|r| {
                        r.results
                            .iter()
                            .filter(|x| {
                                x.outcome == anycode_core::VerificationOutcome::TaskFailed
                                    && x.severity != anycode_core::GateSeverity::Info
                            })
                            .map(|x| x.gate_id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
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

    /// P0.1/P0.3:生成 rubric(每会话一次)并做对抗判定。LLM 全程不可用返回 None
    /// (rubric 生成即失败,不再尝试判定)。
    async fn run_grader(
        &self,
        input: &GuardEvalInput<'_>,
        state: &mut GuardLoopState,
        guard_out: &super::completion_guard::GuardOutcome,
    ) -> Option<GraderVerdict> {
        let config = self.model_for_task(input.agent_type).clone();
        if !state.rubric_attempted {
            state.rubric_attempted = true;
            state.rubric = grader::generate_rubric(
                &self.llm_client,
                &config,
                input.task_prompt,
                input.task_family,
            )
            .await;
            if state.rubric.is_none() {
                super::delivery_metrics::record_grader_verdict(
                    &input.task_id,
                    input.session_label,
                    "unavailable",
                    0,
                    0,
                );
                return None;
            }
        }
        let rubric = state.rubric.as_deref().unwrap_or(&[]);
        Some(
            grader::grade_completion(
                &self.llm_client,
                &config,
                rubric,
                input.task_prompt,
                input.assistant_text,
                input.artifacts,
                guard_out.report.as_ref(),
            )
            .await,
        )
    }
}
