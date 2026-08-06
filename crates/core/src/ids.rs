//! 跨域标识与消息元数据常量。

use uuid::Uuid;

/// 任务 ID
pub type TaskId = Uuid;

/// Agent ID
pub type AgentId = Uuid;

/// 会话 ID
pub type SessionId = Uuid;

/// 工具名称
pub type ToolName = String;

/// Assistant `Message.metadata` 键：保存本轮 `Vec<ToolCall>` JSON，供 LLM 客户端重建工具调用历史。
pub const ANYCODE_TOOL_CALLS_METADATA_KEY: &str = "anycode_tool_calls";

/// Assistant `Message.metadata` 键：DeepSeek/GLM thinking 模式的 `reasoning_content`。
/// 发生过 tool_calls 的助手轮次必须在后续请求中原样回传，否则 API 返回 400。
pub const ANYCODE_REASONING_CONTENT_METADATA_KEY: &str = "reasoning_content";

/// User `Message.metadata`：本条为会话压缩后的续接摘要（与 Claude Code `isCompactSummary` 对齐）。
pub const ANYCODE_COMPACT_SUMMARY_METADATA_KEY: &str = "anycode_compact_summary";

/// User `Message.metadata`：由运行时注入的「上下文状态」伪用户消息（Workspace / Workflow 等），
/// 参与 LLM 请求但不在 TUI transcript 中展示，避免与真实对话混在一起。
pub const ANYCODE_CONTEXT_USER_METADATA_KEY: &str = "anycode_context_user";

/// Assistant `Message.metadata` 键：Responses API 链式状态
/// `{"id": "<server response id>", "prefix_hash": "<u64 hex>"}`。
/// 仅对支持 `previous_response_id` 的端点生效；无状态端点（如 DeepSeek `/responses`）不写入。
pub const ANYCODE_RESPONSE_ID_METADATA_KEY: &str = "anycode_response_id";

/// User `Message.metadata` 键：快照内临时注入的回复语言提醒（仅存在于发往 LLM 的
/// 请求快照，不落历史）。Responses 链式 prefix hash 必须跳过此类 ephemeral 消息。
pub const REPLY_LANGUAGE_REMINDER_METADATA_KEY: &str = "reply_language_reminder";
