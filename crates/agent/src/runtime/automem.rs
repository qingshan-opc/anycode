//! Auto-memory（LLM 驱动）运行时接线：提取 fork、autoDream 巩固、MEMORY.md 召回注入。
//!
//! 领域判定全部在 `anycode_memory::automem`（纯函数）；本模块负责 IO、fork 编排与
//! 工具面门控：
//! - 提取：任务成功收尾后 `maybe_automem_after_task` 组装 transcript → `plan_extract`
//!   → 后台受限 fork（`automem-extract`）。
//! - 巩固：每个成功任务推进 dream 计数，`dream_gate` 打开时跑四阶段巩固 fork
//!   （`automem-dream`），跨进程锁 + 状态文件。
//! - 召回：`automem_index_section` 把按项目的 `MEMORY.md`（截断后）注入编译上下文。
//! - 门控：fork 任务的每次工具调用经 `automem_can_use_tool`（见 `tool_invocation.rs`）。

use super::AgentRuntime;
use anycode_core::prelude::*;
use anycode_memory::automem;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;
use uuid::Uuid;

/// 提取 fork 的 agent 类型标记（`AgentRuntime::new` 注册为 GeneralPurposeAgent）。
pub(crate) const AUTOMEM_EXTRACT_AGENT_TYPE: &str = "automem-extract";
/// 巩固 fork 的 agent 类型标记。
pub(crate) const AUTOMEM_DREAM_AGENT_TYPE: &str = "automem-dream";

/// 提取 fork 最大模型轮次（对齐 Claude `maxTurns: 5`，防"验证兔子洞"）。
const AUTOMEM_EXTRACT_MAX_TURNS: usize = 5;
/// 巩固 fork 最大模型轮次（四阶段需要更多探索）。
const AUTOMEM_DREAM_MAX_TURNS: usize = 10;
/// 提取/巩固 fork 的工具调用上限。
const AUTOMEM_MAX_TOOL_CALLS: usize = 40;
/// dream 锁陈旧阈值（秒）：超时视为持有者已崩溃，可回收。
const DREAM_LOCK_STALE_SECS: i64 = 2 * 3600;

/// 判断 agent_type 是否为 auto-memory fork（用于防递归与钩子豁免）。
pub(crate) fn is_automem_agent_type(agent_type: &str) -> bool {
    agent_type.starts_with("automem")
}

/// dream 状态文件（`{memory_dir}/.automem-dream-state.json`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DreamState {
    /// 上次巩固完成时间（RFC3339）。
    #[serde(default)]
    last_consolidated_at: Option<DateTime<Utc>>,
    /// 距上次巩固累计的成功任务数（≈ Claude 的 session 门）。
    #[serde(default)]
    tasks_since: u32,
    /// 提取游标：上次提取推进到的消息 id（`lastMemoryMessageUuid` 语义）。
    #[serde(default)]
    last_extract_msg_id: Option<Uuid>,
}

fn dream_state_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join(".automem-dream-state.json")
}

fn dream_lock_path(memory_dir: &Path) -> PathBuf {
    memory_dir.join(".automem-dream.lock")
}

fn load_dream_state(memory_dir: &Path) -> DreamState {
    let raw = match std::fs::read_to_string(dream_state_path(memory_dir)) {
        Ok(r) => r,
        Err(_) => return DreamState::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_dream_state(memory_dir: &Path, state: &DreamState) {
    if let Ok(raw) = serde_json::to_string_pretty(state) {
        let tmp = memory_dir.join(".automem-dream-state.json.tmp");
        if std::fs::write(&tmp, raw).is_ok() {
            let _ = std::fs::rename(&tmp, dream_state_path(memory_dir));
        }
    }
}

/// 跨进程 dream 锁：create_new 占位；holder 崩溃留下的陈旧锁可回收。
struct DreamLock {
    path: PathBuf,
}

impl DreamLock {
    fn try_acquire(memory_dir: &Path) -> Option<Self> {
        let path = dream_lock_path(memory_dir);
        if let Ok(meta) = std::fs::metadata(&path) {
            let stale = meta
                .modified()
                .ok()
                .and_then(|m| m.elapsed().ok())
                .is_some_and(|age| age.as_secs() as i64 > DREAM_LOCK_STALE_SECS);
            if stale {
                let _ = std::fs::remove_file(&path);
            }
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                use std::io::Write;
                let _ = writeln!(
                    f,
                    "pid={} ts={}",
                    std::process::id(),
                    Utc::now().to_rfc3339()
                );
                Some(Self { path })
            }
            Err(_) => None,
        }
    }
}

impl Drop for DreamLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl AgentRuntime {
    /// auto-memory 目录：`{base}/projects/{sanitized-cwd}/memory/`；未启用时为 `None`。
    pub(super) fn automem_dir_for(&self, working_directory: &str) -> Option<PathBuf> {
        self.automem.as_ref()?;
        let wd = working_directory.trim();
        if wd.is_empty() {
            return None;
        }
        Some(automem::auto_mem_path(&self.automem_base, wd))
    }

    /// 召回侧：按项目的 `MEMORY.md` 索引段（截断 + point-in-time 告诫）。
    /// automem fork 自身不注入（它们的 prompt 自带目录路径，且避免递归放大）。
    pub(super) fn automem_index_section(
        &self,
        agent_type: &str,
        working_directory: &str,
    ) -> Option<String> {
        if is_automem_agent_type(agent_type) {
            return None;
        }
        let dir = self.automem_dir_for(working_directory)?;
        let entrypoint = automem::auto_mem_entrypoint(&dir);
        let raw = std::fs::read_to_string(entrypoint).ok()?;
        if raw.trim().is_empty() {
            return None;
        }
        let truncated = automem::truncate_entrypoint_content(&raw);
        Some(format!(
            "## Persistent Memory Index (MEMORY.md)\n\n\
             Project memory directory: `{}` — read topic files on demand with FileRead/Grep.\n\
             Memories are point-in-time observations; verify against current code before asserting as fact, \
             and update or remove stale entries with Edit/FileWrite inside that directory.\n\n\
             {}",
            dir.display(),
            truncated.content
        ))
    }

    /// 工具面门控：fork 任务（task_id 已登记）的每次调用先过 `automem_can_use_tool`。
    pub(super) fn automem_gate_dir(&self, task_id: TaskId) -> Option<PathBuf> {
        self.automem_gates
            .lock()
            .ok()
            .and_then(|g| g.get(&task_id).cloned())
    }

    /// 任务/turn 成功收尾后的 auto-memory 入口（提取 + dream 门控）。
    /// `execute_task` 与 `execute_turn_from_messages` 共用；只做轻量判定与 spawn，不阻塞返回。
    pub(super) fn maybe_automem_after_turn(
        &self,
        agent_type: &str,
        working_directory: &str,
        system_prompt_append: Option<&str>,
        log_task_id: TaskId,
        messages: &[Message],
    ) {
        let Some(ref settings) = self.automem else {
            return;
        };
        if !settings.enabled || !settings.fork_agent {
            return;
        }
        // 防递归：automem fork 自身、嵌套子代理（Task 工具派出）不触发。
        if is_automem_agent_type(agent_type)
            || system_prompt_append == Some(super::nested_task::SUBAGENT_SYSTEM_APPEND)
        {
            return;
        }
        let Some(dir) = self.automem_dir_for(working_directory) else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(target: "anycode_agent", "automem dir create failed: {}", e);
            return;
        }

        let mut dream_state = load_dream_state(&dir);
        dream_state.tasks_since = dream_state.tasks_since.saturating_add(1);

        // cursor 语义（对齐 Claude `lastMemoryMessageUuid`）：只提取上次游标之后的新消息；
        // 无论是否 spawn（含互斥跳过）都把游标推进到当前末尾，主 agent 与后台代理互不重复。
        let (entries, writes) = collect_turn_signal(messages, dream_state.last_extract_msg_id);
        dream_state.last_extract_msg_id = messages.last().map(|m| m.id);
        let entry_refs: Vec<(&str, &str)> = entries
            .iter()
            .map(|(r, t)| (r.as_str(), t.as_str()))
            .collect();
        let write_refs: Vec<(&str, &str)> = writes
            .iter()
            .map(|(n, p)| (n.as_str(), p.as_str()))
            .collect();
        let plan = automem::plan_extract(
            &entry_refs,
            &write_refs,
            &dir,
            settings.transcript_max_chars.max(1_000),
        );

        if plan.should_extract {
            match self.spawn_automem_fork(
                AUTOMEM_EXTRACT_AGENT_TYPE,
                dir.clone(),
                automem::build_extract_prompt(&dir.to_string_lossy(), &plan.injected_context),
                AUTOMEM_EXTRACT_MAX_TURNS,
                None,
            ) {
                Ok(()) => self.logger().line(log_task_id, "[automem_extract] spawned"),
                Err(e) => warn!(target: "anycode_agent", "automem extract spawn failed: {}", e),
            }
        } else if let Some(reason) = &plan.skip_reason {
            self.logger()
                .line(log_task_id, &format!("[automem_extract] skipped: {reason}"));
        }

        // autoDream 门控：时间门 → 任务数门 → 跨进程锁（锁的所有权移交给 fork，
        // fork 结束即 Drop 释放；spawn 失败锁也随作用域释放）。
        let hours_since = dream_state
            .last_consolidated_at
            .map(|t| (Utc::now() - t).num_seconds() as f64 / 3600.0)
            .unwrap_or(f64::INFINITY);
        let gate = automem::dream_gate(
            hours_since,
            dream_state.tasks_since as usize,
            settings.dream_min_hours,
            settings.dream_min_sessions as usize,
            false,
        );
        if matches!(gate, automem::DreamGate::Open { .. }) {
            if let Some(lock) = DreamLock::try_acquire(&dir) {
                // 任务 transcript：`~/.anycode/tasks/<task_id>/events.jsonl`（DiskTaskOutput）。
                let transcript_dir = self
                    .automem_base
                    .join("tasks")
                    .to_string_lossy()
                    .into_owned();
                let extra = format!(
                    "{}\n\n{}",
                    automem::MEMORY_FILE_FORMAT_GUIDANCE,
                    automem::dream_tool_constraints_extra(&[])
                );
                match self.spawn_automem_fork(
                    AUTOMEM_DREAM_AGENT_TYPE,
                    dir.clone(),
                    automem::build_consolidation_prompt(
                        &dir.to_string_lossy(),
                        &transcript_dir,
                        &extra,
                    ),
                    AUTOMEM_DREAM_MAX_TURNS,
                    Some(lock),
                ) {
                    Ok(()) => {
                        dream_state.last_consolidated_at = Some(Utc::now());
                        dream_state.tasks_since = 0;
                        self.logger().line(log_task_id, "[automem_dream] spawned");
                    }
                    Err(e) => {
                        warn!(target: "anycode_agent", "automem dream spawn failed: {}", e)
                    }
                }
            }
        }
        save_dream_state(&dir, &dream_state);
    }

    /// 启动受限后台 fork：工具名单 = 注册工具 − automem 白名单外整名剔除；
    /// 输入级门控（只读 Bash / 目录内 Edit·Write）经 `automem_gates` 在
    /// `run_tool_invocation_pipeline` 生效，fork 结束自动解除。
    fn spawn_automem_fork(
        &self,
        agent_type: &str,
        memory_dir: PathBuf,
        prompt: String,
        max_turns: usize,
        dream_lock: Option<DreamLock>,
    ) -> Result<(), CoreError> {
        let runtime = self
            .self_weak
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|w| w.upgrade()))
            .ok_or_else(|| CoreError::Other(anyhow::anyhow!("automem: runtime not attached")))?;

        let deny = {
            // 尽力读取工具表；失败则退化为仅名单过滤（输入级门控仍兜底）。
            match self.tools.try_read() {
                Ok(tools) => {
                    let all: Vec<&str> = tools.keys().map(|k| k.as_str()).collect();
                    automem::automem_extra_deny_names(&all)
                }
                Err(_) => vec![],
            }
        };

        let task_id = Uuid::new_v4();
        let fork = Task {
            id: task_id,
            agent_type: AgentType::new(agent_type),
            prompt,
            context: TaskContext {
                session_id: Uuid::new_v4(),
                working_directory: memory_dir.to_string_lossy().into_owned(),
                environment: HashMap::new(),
                user_id: None,
                system_prompt_append: Some(AUTOMEM_SYSTEM_APPEND.to_string()),
                context_injections: vec![],
                nested_model_override: None,
                nested_worktree_repo_root: None,
                nested_worktree_path: None,
                nested_cancel: None,
                channel_progress_tx: None,
                live_trace_tx: None,
                tool_deny_names: deny,
                tool_deny_prefixes: vec!["mcp__".to_string()],
                user_vision_images: vec![],
                budget: TaskBudget::default(),
                loop_limits: AgentLoopLimits {
                    max_agent_turns: max_turns,
                    max_tool_calls: AUTOMEM_MAX_TOOL_CALLS,
                },
                chat_turn: None,
            },
            created_at: Utc::now(),
        };

        if let Ok(mut gates) = self.automem_gates.lock() {
            gates.insert(task_id, memory_dir);
        }
        let agent_type_owned = agent_type.to_string();
        tokio::spawn(async move {
            let _lock = dream_lock;
            let result = runtime.execute_task(fork).await;
            if let Ok(mut gates) = runtime.automem_gates.lock() {
                gates.remove(&task_id);
            }
            if let Err(e) = result {
                warn!(target: "anycode_agent", "automem fork {} failed: {}", agent_type_owned, e);
            }
        });
        Ok(())
    }
}

/// auto-memory fork 的系统提示追加（同时作为「本任务是 automem fork」的判定标记）。
const AUTOMEM_SYSTEM_APPEND: &str = "\
You are an auto-memory maintenance subagent. Work only within the memory directory given in your task. \
Do not touch project source files, do not run state-changing commands, and do not ask the user anything. \
Finish with a one-sentence summary.";

/// 从完成任务的消息流组装提取信号：cursor 之后（无 cursor 时尾部）user/assistant/tool
/// 文本 + 该范围内 `(tool, file_path)` 写入列表（供 `has_memory_writes_since` 互斥）。
fn collect_turn_signal(
    messages: &[Message],
    cursor: Option<Uuid>,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    const MAX_ENTRIES: usize = 24;
    const PER_MSG_CHARS: usize = 2_000;

    // cursor 命中则只取其后的消息；未命中（状态丢失 / 历史被 compact）退化为尾部窗口。
    let scope: &[Message] = match cursor {
        Some(id) => match messages.iter().position(|m| m.id == id) {
            Some(pos) => &messages[pos + 1..],
            None => messages,
        },
        None => messages,
    };

    let mut entries: Vec<(&str, String)> = Vec::new();
    let mut writes: Vec<(String, String)> = Vec::new();

    for m in scope {
        match m.role {
            MessageRole::System => continue,
            MessageRole::User | MessageRole::Assistant | MessageRole::Tool => {}
        }
        let role = match m.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
            MessageRole::System => unreachable!(),
        };
        // 跳过编译器注入的上下文段（记忆召回、gate 计划等），只保留真实对话。
        let injected = m
            .metadata
            .get(ANYCODE_CONTEXT_USER_METADATA_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if injected {
            continue;
        }
        let text = match &m.content {
            MessageContent::Text(t) => t.trim().to_string(),
            MessageContent::ToolResult { content, .. } => {
                format!("[tool_result] {}", content.trim())
            }
            MessageContent::ToolUse { name, input } => {
                if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                    writes.push((name.clone(), fp.to_string()));
                }
                format!("[tool_use {name}]")
            }
        };
        if text.is_empty() {
            continue;
        }
        let truncated: String = text.chars().take(PER_MSG_CHARS).collect();
        entries.push((role, truncated));
    }

    // 工具调用也可能记录在 assistant 消息的 tool_calls metadata（execute_task 路径）。
    for m in scope {
        if m.role != MessageRole::Assistant {
            continue;
        }
        if let Some(v) = m.metadata.get(ANYCODE_TOOL_CALLS_METADATA_KEY) {
            if let Some(calls) = v.as_array() {
                for c in calls {
                    let name = c.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let fp = c
                        .get("input")
                        .and_then(|i| i.get("file_path"))
                        .and_then(|f| f.as_str())
                        .unwrap_or("");
                    if !name.is_empty() && !fp.is_empty() {
                        writes.push((name.to_string(), fp.to_string()));
                    }
                }
            }
        }
    }

    let tail = if entries.len() > MAX_ENTRIES {
        entries.split_off(entries.len() - MAX_ENTRIES)
    } else {
        entries
    };
    let owned = tail
        .into_iter()
        .map(|(r, t)| (r.to_string(), t))
        .collect::<Vec<_>>();
    (owned, writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn msg(role: MessageRole, text: &str) -> Message {
        Message {
            id: Uuid::new_v4(),
            role,
            content: MessageContent::Text(text.to_string()),
            timestamp: Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn collect_turn_signal_skips_injected_and_collects_writes() {
        let mut injected = msg(MessageRole::User, "## Task Spec compiled context");
        injected.metadata.insert(
            ANYCODE_CONTEXT_USER_METADATA_KEY.to_string(),
            serde_json::Value::Bool(true),
        );
        let mut assistant = msg(MessageRole::Assistant, "working on it");
        assistant.metadata.insert(
            ANYCODE_TOOL_CALLS_METADATA_KEY.to_string(),
            serde_json::json!([{"name": "Edit", "input": {"file_path": "/mem/note.md"}}]),
        );
        let messages = vec![
            msg(MessageRole::System, "system prompt"),
            msg(MessageRole::User, "remember I prefer dark mode"),
            assistant,
            msg(MessageRole::Tool, "file written"),
            injected,
        ];
        let (entries, writes) = collect_turn_signal(&messages, None);
        let roles: Vec<&str> = entries.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "tool"]);
        assert!(!entries.iter().any(|(_, t)| t.contains("Task Spec")));
        assert_eq!(
            writes,
            vec![("Edit".to_string(), "/mem/note.md".to_string())]
        );
    }

    #[test]
    fn collect_turn_signal_tails_long_histories() {
        let messages: Vec<Message> = (0..40)
            .map(|i| msg(MessageRole::User, &format!("turn {i}")))
            .collect();
        let (entries, _) = collect_turn_signal(&messages, None);
        assert_eq!(entries.len(), 24);
        assert_eq!(entries[0].1, "turn 16");
    }

    #[test]
    fn collect_turn_signal_respects_cursor() {
        let m1 = msg(MessageRole::User, "already extracted");
        let m2 = msg(MessageRole::Assistant, "old reply");
        let m3 = msg(MessageRole::User, "fresh signal");
        let m4 = msg(MessageRole::Assistant, "fresh reply");
        let cursor = m2.id;
        let messages = vec![m1, m2, m3, m4];
        let (entries, _) = collect_turn_signal(&messages, Some(cursor));
        let texts: Vec<&str> = entries.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(texts, ["fresh signal", "fresh reply"]);

        // cursor 丢失（历史被 compact）→ 退化尾部窗口，不丢信号。
        let (entries, _) = collect_turn_signal(&messages, Some(Uuid::new_v4()));
        assert_eq!(entries.len(), 4);

        // cursor 在末尾 → 无新消息，不产生提取。
        let last = messages.last().unwrap().id;
        let (entries, _) = collect_turn_signal(&messages, Some(last));
        assert!(entries.is_empty());
    }

    #[test]
    fn dream_state_roundtrip_and_lock_exclusion() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let mut state = load_dream_state(dir);
        assert!(state.last_consolidated_at.is_none());
        state.tasks_since = 7;
        state.last_consolidated_at = Some(Utc::now());
        save_dream_state(dir, &state);
        let loaded = load_dream_state(dir);
        assert_eq!(loaded.tasks_since, 7);
        assert!(loaded.last_consolidated_at.is_some());

        // 锁互斥：持锁期间第二次获取失败，Drop 后可再获取。
        let lock = DreamLock::try_acquire(dir).expect("first acquire");
        assert!(DreamLock::try_acquire(dir).is_none());
        drop(lock);
        assert!(DreamLock::try_acquire(dir).is_some());
    }

    #[test]
    fn stale_dream_lock_is_reclaimed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let lock_path = dream_lock_path(dir);
        std::fs::write(&lock_path, "pid=0").unwrap();
        // 手工把 mtime 拨到 3 小时前 → 视为陈旧可回收。
        let past = filetime_past(&lock_path);
        assert!(past);
        assert!(DreamLock::try_acquire(dir).is_some());
    }

    #[cfg(unix)]
    fn filetime_past(path: &Path) -> bool {
        std::process::Command::new("touch")
            .arg("-t")
            .arg("200001010000.00")
            .arg(path)
            .spawn()
            .map(|mut c| c.wait().map(|s| s.success()).unwrap_or(false))
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    fn filetime_past(_path: &Path) -> bool {
        // 非 unix 平台跳过陈旧拨时（测试环境均为 unix）。
        true
    }
}
