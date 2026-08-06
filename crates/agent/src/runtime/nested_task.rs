//! Nested sub-agent execution (`SubAgentExecutor`).

use super::nested_worktree;
use super::AgentRuntime;
use anycode_core::prelude::*;
use async_trait::async_trait;
use std::collections::HashMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

/// 子代理默认追加提示词（D6：对齐 Claude Code `appendSubagentSystemPrompt`）。
/// 提示子代理聚焦任务、避免重复上下文与冗长回执。
/// 该常量同时作为「是否为嵌套子代理任务」的判定标记（见 `execute_task` 子代理工具过滤）。
pub(crate) const SUBAGENT_SYSTEM_APPEND: &str = "\
你是被父任务派出的子代理，只完成父任务交给你的这个任务。\
请保持专注：不要重复描述父任务已提供的上下文，不要主动扩大任务范围。\
输出只包含任务要求的结果；完成后用简短一句话总结即可。";

#[async_trait]
impl SubAgentExecutor for AgentRuntime {
    async fn run_nested_task(&self, invoke: NestedTaskInvoke) -> Result<NestedTaskRun, CoreError> {
        let mut wd = invoke.working_directory;
        let wt_roots = {
            let iso = invoke
                .isolation
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if iso.is_some_and(|s| s.eq_ignore_ascii_case("worktree")) {
                let (repo, wt) = nested_worktree::create_nested_worktree(&wd).await?;
                wd = wt.clone();
                Some((repo, wt))
            } else {
                None
            }
        };

        let task_id = invoke.task_id.unwrap_or_else(Uuid::new_v4);
        let agent_type_str = invoke.agent_type.as_str().to_string();
        // Step 3b：子任务事件经 Subagent 包装转发到父 live trace 通道；
        // 父侧无通道时保持 None（嵌套运行不可见的旧行为）。
        let nested_trace_tx = invoke.live_trace_tx.as_ref().map(|parent_tx| {
            spawn_nested_trace_forwarder(
                parent_tx.clone(),
                task_id,
                agent_type_str,
                invoke.parent_task_id,
            )
        });

        let task = Task {
            id: task_id,
            agent_type: invoke.agent_type,
            prompt: invoke.prompt,
            context: TaskContext {
                session_id: Uuid::new_v4(),
                working_directory: wd,
                environment: HashMap::new(),
                user_id: None,
                system_prompt_append: Some(SUBAGENT_SYSTEM_APPEND.to_string()),
                context_injections: invoke.context_injections.clone(),
                nested_model_override: invoke.model.clone(),
                nested_worktree_repo_root: wt_roots.as_ref().map(|(r, _)| r.clone()),
                nested_worktree_path: wt_roots.as_ref().map(|(_, p)| p.clone()),
                nested_cancel: invoke.cancel.clone(),
                channel_progress_tx: None,
                live_trace_tx: nested_trace_tx,
                tool_deny_names: invoke.tool_deny_names.clone(),
                tool_deny_prefixes: invoke.tool_deny_prefixes.clone(),
                user_vision_images: vec![],
                budget: nested_budget_from_env(),
                loop_limits: anycode_core::resolve_agent_loop_limits(None, None),
                chat_turn: anycode_core::current_chat_turn(),
            },
            created_at: chrono::Utc::now(),
        };
        let result = self.execute_task(task).await?;
        Ok(NestedTaskRun { task_id, result })
    }

    fn agent_catalog(&self) -> Vec<(String, String)> {
        // 同步上下文（工具 schema 构建路径）：`try_read` 繁忙时返回空目录；
        // schema 每轮重建，窗口期错过会自愈。
        let Ok(agents) = self.agents.try_read() else {
            return Vec::new();
        };
        super::summaries_from_agents(&agents)
    }
}

/// 唯一打标点：把嵌套任务的内部 live trace 事件包成 `Subagent` 转发给父通道。
/// 发送端全部 drop 后转发任务退出；父 bridge 已关闭（send 失败）时停止转发，
/// 嵌套任务本身不受影响（事件仅丢失，不写父时间线）。
fn spawn_nested_trace_forwarder(
    parent_tx: UnboundedSender<LiveTraceEvent>,
    task_id: Uuid,
    agent_type: String,
    parent_task_id: Option<Uuid>,
) -> UnboundedSender<LiveTraceEvent> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<LiveTraceEvent>();
    tokio::spawn(async move {
        while let Some(evt) = rx.recv().await {
            let wrapped = LiveTraceEvent::Subagent {
                task_id,
                agent_type: agent_type.clone(),
                parent_task_id,
                event: Box::new(evt),
            };
            if parent_tx.send(wrapped).is_err() {
                break;
            }
        }
    });
    tx
}

fn nested_budget_from_env() -> TaskBudget {
    fn env_or<T: std::str::FromStr>(primary: &str, fallback: &str) -> Option<T> {
        std::env::var(primary)
            .or_else(|_| std::env::var(fallback))
            .ok()
            .and_then(|v| v.parse::<T>().ok())
    }
    TaskBudget {
        token_budget_total: env_or("ANYCODE_NESTED_TOKEN_BUDGET", "ANYCODE_TASK_TOKEN_BUDGET"),
        cost_budget_cny: env_or(
            "ANYCODE_NESTED_COST_BUDGET_CNY",
            "ANYCODE_TASK_COST_BUDGET_CNY",
        ),
        max_duration_secs: env_or(
            "ANYCODE_NESTED_MAX_DURATION_SECS",
            "ANYCODE_TASK_MAX_DURATION_SECS",
        ),
        ..TaskBudget::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn nested_trace_forwarder_wraps_inner_events_with_identity() {
        let (parent_tx, mut parent_rx) = tokio::sync::mpsc::unbounded_channel::<LiveTraceEvent>();
        let task_id = Uuid::new_v4();
        let parent_task_id = Uuid::new_v4();
        let child_tx = spawn_nested_trace_forwarder(
            parent_tx,
            task_id,
            "explore".to_string(),
            Some(parent_task_id),
        );

        child_tx
            .send(LiveTraceEvent::TurnStart { turn: 1 })
            .unwrap();
        let wrapped = parent_rx.recv().await.expect("forwarded event");
        match wrapped {
            LiveTraceEvent::Subagent {
                task_id: tid,
                agent_type,
                parent_task_id: pid,
                event,
            } => {
                assert_eq!(tid, task_id);
                assert_eq!(agent_type, "explore");
                assert_eq!(pid, Some(parent_task_id));
                assert_eq!(*event, LiveTraceEvent::TurnStart { turn: 1 });
            }
            other => panic!("expected Subagent wrapper, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn nested_trace_forwarder_exits_when_child_senders_drop() {
        let (parent_tx, mut parent_rx) = tokio::sync::mpsc::unbounded_channel::<LiveTraceEvent>();
        let child_tx =
            spawn_nested_trace_forwarder(parent_tx, Uuid::new_v4(), "plan".to_string(), None);
        child_tx
            .send(LiveTraceEvent::TurnStart { turn: 1 })
            .unwrap();
        assert!(parent_rx.recv().await.is_some());
        // 子侧全部 sender drop 后转发任务退出、父通道随之关闭。
        drop(child_tx);
        let drained: Vec<_> = std::iter::from_fn(|| parent_rx.try_recv().ok()).collect();
        assert!(drained.is_empty());
        assert!(parent_rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn nested_trace_forwarder_stops_when_parent_channel_closed() {
        let (parent_tx, parent_rx) = tokio::sync::mpsc::unbounded_channel::<LiveTraceEvent>();
        let child_tx =
            spawn_nested_trace_forwarder(parent_tx, Uuid::new_v4(), "plan".to_string(), None);
        drop(parent_rx);
        // 父通道关闭后转发失败退出；子侧发送不 panic（事件仅丢失）。
        for _ in 0..8 {
            let _ = child_tx.send(LiveTraceEvent::TurnStart { turn: 1 });
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let _ = child_tx.send(LiveTraceEvent::TurnDone {
            status: "completed".to_string(),
        });
    }
}
