//! Bridge in-process [`LiveTraceEvent`] → dashboard `chat_event` SSE.

use crate::db::DashboardDb;
use crate::events::EventBus;
use crate::observability::chat_events::{
    apply_subagent_scope, chat_event_from_live_trace, subagent_done_event, subagent_header_event,
    turn_phase_event, SubagentScope,
};
use crate::observability::chat_turn_log::persist_and_enrich;
use crate::schema::ChatStreamEvent;
use anycode_core::LiveTraceEvent;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedReceiver;
use uuid::Uuid;

const DELTA_FLUSH: Duration = Duration::from_millis(50);

struct BridgeState {
    assistant_raw_buffers: HashMap<u32, String>,
    assistant_display_buffers: HashMap<u32, String>,
    thinking_buffers: HashMap<u32, String>,
    pending_delta_turn: Option<u32>,
    last_delta_flush: HashMap<u32, Instant>,
    streaming_phases: HashSet<u32>,
    tool_phases: HashSet<u32>,
    narration_turns: HashSet<u32>,
}

impl BridgeState {
    fn new() -> Self {
        Self {
            assistant_raw_buffers: HashMap::new(),
            assistant_display_buffers: HashMap::new(),
            thinking_buffers: HashMap::new(),
            pending_delta_turn: None,
            last_delta_flush: HashMap::new(),
            streaming_phases: HashSet::new(),
            tool_phases: HashSet::new(),
            narration_turns: HashSet::new(),
        }
    }

    fn should_flush_delta(&mut self, turn: u32) -> bool {
        let now = Instant::now();
        let last = self.last_delta_flush.entry(turn).or_insert(now);
        if now.duration_since(*last) >= DELTA_FLUSH {
            *last = now;
            true
        } else {
            false
        }
    }

    async fn flush_pending_delta(
        &mut self,
        db: &DashboardDb,
        events: &EventBus,
        session_id: &str,
        project_id: &str,
        user_turn_id: u32,
        scope: Option<&SubagentScope>,
    ) {
        let Some(turn) = self.pending_delta_turn.take() else {
            return;
        };
        let full = self
            .assistant_display_buffers
            .get(&turn)
            .cloned()
            .unwrap_or_default();
        if full.is_empty() {
            return;
        }
        let mut chat_evt = crate::observability::chat_events::assistant_delta_event(
            session_id,
            project_id,
            user_turn_id,
            turn,
            "",
            &full,
            self.narration_turns.contains(&turn),
        );
        if let Some(scope) = scope {
            apply_subagent_scope(&mut chat_evt, scope);
        }
        publish_persisted(db, events, chat_evt, user_turn_id).await;
    }

    fn phase_for_event(&mut self, evt: &LiveTraceEvent) -> Option<&'static str> {
        match evt {
            LiveTraceEvent::LlmRequestStart { .. } => Some("waiting_first_token"),
            LiveTraceEvent::AssistantDelta { turn, .. } => {
                if self.streaming_phases.insert(*turn) {
                    Some("streaming")
                } else {
                    None
                }
            }
            LiveTraceEvent::ToolCallStart { turn, .. } => {
                if self.tool_phases.insert(*turn) {
                    Some("running_tools")
                } else {
                    None
                }
            }
            LiveTraceEvent::TurnDone { .. } => {
                self.streaming_phases.clear();
                self.tool_phases.clear();
                self.narration_turns.clear();
                self.thinking_buffers.clear();
                None
            }
            _ => None,
        }
    }
}

async fn publish_persisted(
    db: &DashboardDb,
    events: &EventBus,
    chat_evt: ChatStreamEvent,
    conversation_turn_id: u32,
) {
    match persist_and_enrich(db, chat_evt, conversation_turn_id).await {
        Ok(enriched) => events.publish_chat(enriched),
        Err(error) => {
            tracing::warn!(%error, "chat turn event persist failed");
        }
    }
}

async fn publish_turn_phase(
    db: &DashboardDb,
    events: &EventBus,
    session_id: &str,
    project_id: &str,
    user_turn_id: u32,
    turn: u32,
    phase: &str,
) {
    let chat_evt = turn_phase_event(session_id, project_id, user_turn_id, turn, phase);
    publish_persisted(db, events, chat_evt, user_turn_id).await;
}

/// 子代理的独立时间线状态：身份（scope）+ 隔离缓冲。
struct ChildBridge {
    scope: SubagentScope,
    state: BridgeState,
}

/// Live bridge 处理上下文：父时间线状态 + 各子代理的独立缓冲（按 task_id 隔离）。
struct LiveBridge {
    db: DashboardDb,
    events: Arc<EventBus>,
    session_id: String,
    project_id: String,
    user_turn_id: u32,
    state: BridgeState,
    /// 子代理缓冲按 task_id 与父时间线隔离（同一 turn 号互不污染）。
    subagents: HashMap<Uuid, ChildBridge>,
    /// 已发布组头的子代理 task_id（首次见到时发 `subagent_start`）。
    announced: HashSet<Uuid>,
}

impl LiveBridge {
    fn new(
        db: DashboardDb,
        events: Arc<EventBus>,
        session_id: String,
        project_id: String,
        user_turn_id: u32,
    ) -> Self {
        Self {
            db,
            events,
            session_id,
            project_id,
            user_turn_id,
            state: BridgeState::new(),
            subagents: HashMap::new(),
            announced: HashSet::new(),
        }
    }

    async fn flush_all_pending(&mut self) {
        self.state
            .flush_pending_delta(
                &self.db,
                &self.events,
                &self.session_id,
                &self.project_id,
                self.user_turn_id,
                None,
            )
            .await;
        let keys: Vec<Uuid> = self.subagents.keys().copied().collect();
        for key in keys {
            let Some(child) = self.subagents.get_mut(&key) else {
                continue;
            };
            let scope = child.scope.clone();
            child
                .state
                .flush_pending_delta(
                    &self.db,
                    &self.events,
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    Some(&scope),
                )
                .await;
        }
    }

    /// 递归处理事件：`Subagent` 包装解包后进入子作用域，其余按 scope 映射发布。
    async fn process(&mut self, evt: &LiveTraceEvent, scope: Option<SubagentScope>) {
        if let LiveTraceEvent::Subagent {
            task_id,
            agent_type,
            parent_task_id,
            event,
        } = evt
        {
            let child_scope = SubagentScope {
                task_id: *task_id,
                agent_type: agent_type.clone(),
                parent_task_id: *parent_task_id,
            };
            if self.announced.insert(*task_id) {
                let header = subagent_header_event(
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    &child_scope,
                );
                publish_persisted(&self.db, &self.events, header, self.user_turn_id).await;
            }
            let child_done = matches!(event.as_ref(), LiveTraceEvent::TurnDone { .. });
            Box::pin(self.process(event, Some(child_scope))).await;
            if child_done {
                // 子代理结束：冲刷其缓冲并回收状态（task_id 不复用）。
                if let Some(mut child) = self.subagents.remove(task_id) {
                    child
                        .state
                        .flush_pending_delta(
                            &self.db,
                            &self.events,
                            &self.session_id,
                            &self.project_id,
                            self.user_turn_id,
                            Some(&child.scope.clone()),
                        )
                        .await;
                }
            }
            return;
        }

        // 子级 TurnDone 不冒泡为父级 turn_done：发 subagent_done 组尾通知。
        if let (Some(child_scope), LiveTraceEvent::TurnDone { status }) = (scope.as_ref(), evt) {
            let state = &mut self
                .subagents
                .entry(child_scope.task_id)
                .or_insert_with(|| ChildBridge {
                    scope: child_scope.clone(),
                    state: BridgeState::new(),
                })
                .state;
            state
                .flush_pending_delta(
                    &self.db,
                    &self.events,
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    Some(child_scope),
                )
                .await;
            let done = subagent_done_event(
                &self.session_id,
                &self.project_id,
                self.user_turn_id,
                child_scope,
                status,
            );
            publish_persisted(&self.db, &self.events, done, self.user_turn_id).await;
            return;
        }

        let state = match &scope {
            Some(s) => {
                &mut self
                    .subagents
                    .entry(s.task_id)
                    .or_insert_with(|| ChildBridge {
                        scope: s.clone(),
                        state: BridgeState::new(),
                    })
                    .state
            }
            None => &mut self.state,
        };

        // 相位事件只针对父时间线（子代理相位不与父轮次混排）。
        if scope.is_none() {
            let phase_turn = match evt {
                LiveTraceEvent::LlmRequestStart { turn } => Some(*turn),
                LiveTraceEvent::AssistantDelta { turn, .. } => Some(*turn),
                LiveTraceEvent::ToolCallStart { turn, .. } => Some(*turn),
                _ => None,
            };
            if let Some(phase) = state.phase_for_event(evt) {
                if let Some(turn) = phase_turn {
                    publish_turn_phase(
                        &self.db,
                        &self.events,
                        &self.session_id,
                        &self.project_id,
                        self.user_turn_id,
                        turn,
                        phase,
                    )
                    .await;
                }
            }
        }

        match evt {
            LiveTraceEvent::AssistantNarrationMark { turn } => {
                state.narration_turns.insert(*turn);
                state
                    .flush_pending_delta(
                        &self.db,
                        &self.events,
                        &self.session_id,
                        &self.project_id,
                        self.user_turn_id,
                        scope.as_ref(),
                    )
                    .await;
                if let Some(mut chat_evt) = chat_event_from_live_trace(
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    evt,
                    &mut state.assistant_raw_buffers,
                    &mut state.assistant_display_buffers,
                ) {
                    if let Some(s) = scope.as_ref() {
                        apply_subagent_scope(&mut chat_evt, s);
                    }
                    publish_persisted(&self.db, &self.events, chat_evt, self.user_turn_id).await;
                }
                return;
            }
            LiveTraceEvent::ThinkingDelta { turn, delta } => {
                let buf = state.thinking_buffers.entry(*turn).or_default();
                buf.push_str(delta);
                let mut chat_evt = crate::observability::chat_events::thinking_delta_event(
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    *turn,
                    buf,
                );
                if let Some(s) = scope.as_ref() {
                    apply_subagent_scope(&mut chat_evt, s);
                }
                publish_persisted(&self.db, &self.events, chat_evt, self.user_turn_id).await;
                return;
            }
            LiveTraceEvent::AssistantDelta {
                turn, narration, ..
            } => {
                if *narration {
                    state.narration_turns.insert(*turn);
                }
                if let Some(mut chat_evt) = chat_event_from_live_trace(
                    &self.session_id,
                    &self.project_id,
                    self.user_turn_id,
                    evt,
                    &mut state.assistant_raw_buffers,
                    &mut state.assistant_display_buffers,
                ) {
                    if let Some(s) = scope.as_ref() {
                        apply_subagent_scope(&mut chat_evt, s);
                    }
                    if state.should_flush_delta(*turn) {
                        publish_persisted(&self.db, &self.events, chat_evt, self.user_turn_id)
                            .await;
                    } else {
                        state.pending_delta_turn = Some(*turn);
                    }
                }
                return;
            }
            LiveTraceEvent::AssistantDone { .. } | LiveTraceEvent::TurnDone { .. } => {
                state
                    .flush_pending_delta(
                        &self.db,
                        &self.events,
                        &self.session_id,
                        &self.project_id,
                        self.user_turn_id,
                        scope.as_ref(),
                    )
                    .await;
            }
            _ => {}
        }
        if let Some(mut chat_evt) = chat_event_from_live_trace(
            &self.session_id,
            &self.project_id,
            self.user_turn_id,
            evt,
            &mut state.assistant_raw_buffers,
            &mut state.assistant_display_buffers,
        ) {
            if let LiveTraceEvent::ArtifactReady { artifact, .. } = evt {
                if let Some(path) = artifact.path.as_deref() {
                    let kind = artifact.resolved_kind();
                    let title = artifact
                        .title
                        .clone()
                        .unwrap_or_else(|| anycode_core::artifact_title_for_path(path));
                    let _ = self
                        .db
                        .upsert_artifact(&self.project_id, &self.session_id, path, kind, &title)
                        .await;
                }
            }
            if let Some(s) = scope.as_ref() {
                apply_subagent_scope(&mut chat_evt, s);
            }
            publish_persisted(&self.db, &self.events, chat_evt, self.user_turn_id).await;
        }
    }
}

/// Consume runtime live trace events, persist canonical events, then publish SSE.
pub fn spawn_live_bridge(
    events: Arc<EventBus>,
    db: DashboardDb,
    session_id: String,
    project_id: String,
    user_turn_id: u32,
    mut rx: UnboundedReceiver<LiveTraceEvent>,
) {
    tokio::spawn(async move {
        let mut bridge = LiveBridge::new(db, events, session_id, project_id, user_turn_id);
        let mut flush_timer = tokio::time::interval(DELTA_FLUSH);
        flush_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(evt) = msg else { break };
                    bridge.process(&evt, None).await;
                }
                _ = flush_timer.tick() => {
                    bridge.flush_all_pending().await;
                }
            }
        }
        bridge.flush_all_pending().await;
    });
}

#[must_use]
pub fn log_tail_fallback_enabled() -> bool {
    matches!(
        std::env::var("ANYCODE_DASHBOARD_LOG_TAIL_FALLBACK").as_deref(),
        Ok("1") | Ok("true") | Ok("on")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_tail_fallback_defaults_off() {
        std::env::remove_var("ANYCODE_DASHBOARD_LOG_TAIL_FALLBACK");
        assert!(!log_tail_fallback_enabled());
    }

    #[test]
    fn subagent_buffers_are_isolated_per_task_id() {
        // 父 / 子 / 另一子的同名 turn 缓冲互不污染。
        let mut bridge_states = HashMap::new();
        let parent = BridgeState::new();
        let child_a = Uuid::new_v4();
        let child_b = Uuid::new_v4();
        bridge_states.insert(child_a, BridgeState::new());
        bridge_states.insert(child_b, BridgeState::new());

        let mut parent = parent;
        parent.assistant_display_buffers.insert(1, "parent".into());
        bridge_states
            .get_mut(&child_a)
            .unwrap()
            .assistant_display_buffers
            .insert(1, "child-a".into());
        bridge_states
            .get_mut(&child_b)
            .unwrap()
            .assistant_display_buffers
            .insert(1, "child-b".into());

        assert_eq!(parent.assistant_display_buffers.get(&1).unwrap(), "parent");
        assert_eq!(
            bridge_states
                .get(&child_a)
                .unwrap()
                .assistant_display_buffers
                .get(&1)
                .unwrap(),
            "child-a"
        );
        assert_eq!(
            bridge_states
                .get(&child_b)
                .unwrap()
                .assistant_display_buffers
                .get(&1)
                .unwrap(),
            "child-b"
        );
    }
}
