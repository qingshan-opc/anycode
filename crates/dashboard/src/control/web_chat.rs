//! Shared types for Workbench chat dispatch results.
//!
//! Subprocess web-chat hub was removed; execution always goes through
//! `ChatRuntimeHost` (embedded `AgentRuntime`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebChatSendResult {
    pub session_id: String,
    pub pid: u32,
    pub log_path: String,
    pub started_at: String,
    pub queued: bool,
}
