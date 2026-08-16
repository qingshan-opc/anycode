//! FileRead 摘录：与压缩后注入共享的数据结构（`state` / `post_compact` 共用，避免模块环依赖）。

use anycode_core::prelude::*;
use anycode_tools::catalog::TOOL_FILE_READ;

use super::microcompact::CLEARED_TOOL_RESULT_PLACEHOLDER;

pub const POST_COMPACT_MAX_FILES: usize = 5;
pub const POST_COMPACT_MAX_CHARS_PER_FILE: usize = 5_000;

#[derive(Debug, Clone)]
pub struct FileReadSnippet {
    pub path: String,
    pub excerpt: String,
}

fn try_parse_file_read_excerpt(raw: &str) -> Option<(String, String)> {
    if raw == CLEARED_TOOL_RESULT_PLACEHOLDER {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) {
        let path = v.get("path")?.as_str()?.to_string();
        let content = v.get("content")?.as_str()?;
        return excerpt_pair(path, content);
    }
    let mut lines = raw.lines();
    let header = lines.next()?;
    let path = parse_plain_file_read_path(header)?;
    let body = lines.collect::<Vec<_>>().join("\n");
    excerpt_pair(path, &body)
}

fn parse_plain_file_read_path(header: &str) -> Option<String> {
    let rest = header.strip_prefix("path=")?;
    let cut = [" lines=", " truncated=", " encoding="]
        .iter()
        .filter_map(|sep| rest.find(sep))
        .min()
        .unwrap_or(rest.len());
    let path = rest[..cut].trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

fn excerpt_pair(path: String, content: &str) -> Option<(String, String)> {
    if content.is_empty() {
        return None;
    }
    let excerpt = if content.chars().count() > POST_COMPACT_MAX_CHARS_PER_FILE {
        content
            .chars()
            .take(POST_COMPACT_MAX_CHARS_PER_FILE)
            .collect::<String>()
            + "\n… [truncated]"
    } else {
        content.to_string()
    };
    Some((path, excerpt))
}

/// 自会话中收集最近若干次 FileRead 成功结果（同路径后者覆盖前者）。
pub fn collect_from_session(session: &[Message]) -> Vec<FileReadSnippet> {
    collect_from_session_with_max(session, POST_COMPACT_MAX_FILES)
}

/// 与 [`collect_from_session`] 相同，但可指定保留的最大文件数。
pub fn collect_from_session_with_max(
    session: &[Message],
    max_files: usize,
) -> Vec<FileReadSnippet> {
    let mut by_path: Vec<FileReadSnippet> = Vec::new();
    for msg in session {
        if msg.role != MessageRole::Tool {
            continue;
        }
        let name = msg
            .metadata
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if name != TOOL_FILE_READ {
            continue;
        }
        let MessageContent::ToolResult {
            content, is_error, ..
        } = &msg.content
        else {
            continue;
        };
        if *is_error {
            continue;
        }
        if let Some((p, ex)) = try_parse_file_read_excerpt(content) {
            by_path.retain(|s| s.path != p);
            by_path.push(FileReadSnippet {
                path: p,
                excerpt: ex,
            });
        }
    }
    let len = by_path.len();
    if len > max_files {
        by_path.split_off(len - max_files)
    } else {
        by_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn file_read_msg(body: &str) -> Message {
        let mut meta = HashMap::new();
        meta.insert(
            "tool_name".to_string(),
            serde_json::Value::String(TOOL_FILE_READ.to_string()),
        );
        Message {
            id: Uuid::new_v4(),
            role: MessageRole::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: "t1".into(),
                content: body.to_string(),
                is_error: false,
            },
            timestamp: chrono::Utc::now(),
            metadata: meta,
        }
    }

    #[test]
    fn parses_plain_text_file_read_header() {
        let session = vec![file_read_msg(
            "path=/tmp/x.rs lines=1-2\nL1|fn main() {}\nL2|",
        )];
        let got = collect_from_session(&session);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/tmp/x.rs");
        assert!(got[0].excerpt.contains("fn main()"));
    }

    #[test]
    fn skips_cleared_placeholder() {
        let session = vec![file_read_msg(CLEARED_TOOL_RESULT_PLACEHOLDER)];
        assert!(collect_from_session(&session).is_empty());
    }

    #[test]
    fn parses_plain_text_after_head_tail_truncate_marker() {
        let session = vec![file_read_msg(
            "path=/tmp/x.rs\nfn start() {}\n...<truncated>...\nfn end() {}\n",
        )];
        let got = collect_from_session(&session);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/tmp/x.rs");
        assert!(got[0].excerpt.contains("fn start()"));
        assert!(got[0].excerpt.contains("fn end()"));
    }

    #[test]
    fn parses_path_with_spaces() {
        let session = vec![file_read_msg(
            "path=/Users/foo/My Project/src.rs\nfn main() {}\n",
        )];
        let got = collect_from_session(&session);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].path, "/Users/foo/My Project/src.rs");
    }
}
