//! Detect repeated sandbox-escape tool failures within a turn and strengthen the tool result.

use anycode_core::prelude::*;

const ESCAPE_MARKER: &str = "path escapes sandbox";
/// After this many consecutive identical escapes, append a hard stop-retry nudge.
pub(super) const SANDBOX_ESCAPE_NUDGE_AFTER: usize = 2;

const NUDGE_ZH: &str = "\
【沙箱路径连续失败】不要再对同一绝对路径重试 FileRead/Glob/Grep。\
请改用：1) 相对工作目录的路径；2) 已安装 skill 包路径 ~/.anycode/skills/<id>/...；\
3) 或换到正确的项目根目录。继续越界读取只会重复失败。";

const NUDGE_EN: &str = "\
[sandbox escape streak] Stop retrying the same absolute path with FileRead/Glob/Grep. \
Use: (1) a path relative to the working directory; (2) an installed skill under \
~/.anycode/skills/<id>/...; or (3) switch to the correct project root. \
Retrying out-of-sandbox paths will keep failing.";

pub(super) fn is_sandbox_escape_error(err: &str) -> bool {
    err.contains(ESCAPE_MARKER)
}

/// Stable-ish key so "same escape" retries can be counted (path + marker).
pub(super) fn sandbox_escape_key(err: &str) -> String {
    // Prefer the Debug-quoted candidate path after `): "` if present.
    if let Some(idx) = err.find("): \"") {
        let after_quote = &err[idx + 4..];
        if let Some(end) = after_quote.find('"') {
            return format!("{ESCAPE_MARKER}:{}", &after_quote[..end]);
        }
    }
    if let Some(idx) = err.find("): ") {
        let rest = err[idx + 3..].split(". Hint:").next().unwrap_or("").trim();
        if !rest.is_empty() {
            return format!("{ESCAPE_MARKER}:{rest}");
        }
    }
    ESCAPE_MARKER.to_string()
}

pub(super) fn locale_nudge(prefer_zh: bool) -> &'static str {
    if prefer_zh {
        NUDGE_ZH
    } else {
        NUDGE_EN
    }
}

/// Update streak counters; returns Some(nudge) when the model should stop thrashing.
pub(super) fn note_sandbox_escape(
    streak: &mut usize,
    last_key: &mut Option<String>,
    err: &str,
    prefer_zh: bool,
) -> Option<&'static str> {
    if !is_sandbox_escape_error(err) {
        *streak = 0;
        *last_key = None;
        return None;
    }
    let key = sandbox_escape_key(err);
    if last_key.as_deref() == Some(key.as_str()) {
        *streak = streak.saturating_add(1);
    } else {
        *streak = 1;
        *last_key = Some(key);
    }
    if *streak >= SANDBOX_ESCAPE_NUDGE_AFTER {
        Some(locale_nudge(prefer_zh))
    } else {
        None
    }
}

/// Append nudge text onto a ToolOutput so the model sees it in the tool_result.
pub(super) fn append_nudge_to_tool_output(out: &mut ToolOutput, nudge: &str) {
    match &mut out.result {
        serde_json::Value::Object(map) => {
            map.insert(
                "nudge".to_string(),
                serde_json::Value::String(nudge.to_string()),
            );
        }
        other => {
            *other = serde_json::json!({
                "result": other.clone(),
                "nudge": nudge,
            });
        }
    }
    match &mut out.error {
        Some(err) => {
            if !err.contains(nudge) {
                err.push_str("\n");
                err.push_str(nudge);
            }
        }
        None => out.error = Some(nudge.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streak_triggers_on_second_identical_escape() {
        let mut streak = 0usize;
        let mut last = None;
        let err = "path escapes sandbox (must be under /tmp/w): \"/Users/x\". Hint: use relative";
        assert!(note_sandbox_escape(&mut streak, &mut last, err, true).is_none());
        assert_eq!(streak, 1);
        let nudge = note_sandbox_escape(&mut streak, &mut last, err, true);
        assert!(nudge.is_some());
        assert_eq!(streak, 2);
        assert!(nudge.unwrap().contains("沙箱"));
    }

    #[test]
    fn different_paths_reset_streak_key() {
        let mut streak = 0usize;
        let mut last = None;
        let a = "path escapes sandbox (must be under /tmp/w): \"/Users/a\"";
        let b = "path escapes sandbox (must be under /tmp/w): \"/Users/b\"";
        assert!(note_sandbox_escape(&mut streak, &mut last, a, false).is_none());
        assert!(note_sandbox_escape(&mut streak, &mut last, b, false).is_none());
        assert_eq!(streak, 1);
        let nudge = note_sandbox_escape(&mut streak, &mut last, b, false);
        assert!(nudge.is_some());
        assert!(nudge.unwrap().contains("sandbox escape streak"));
    }

    #[test]
    fn non_escape_clears_streak() {
        let mut streak = 2usize;
        let mut last = Some("x".into());
        assert!(note_sandbox_escape(&mut streak, &mut last, "File not found", false).is_none());
        assert_eq!(streak, 0);
        assert!(last.is_none());
    }

    #[test]
    fn append_nudge_mutates_json_and_error() {
        let mut out = ToolOutput {
            result: serde_json::json!({"error": "path escapes sandbox"}),
            error: Some("path escapes sandbox".into()),
            duration_ms: 1,
        };
        append_nudge_to_tool_output(&mut out, "STOP");
        assert_eq!(
            out.result.get("nudge").and_then(|v| v.as_str()),
            Some("STOP")
        );
        assert!(out.error.as_deref().unwrap().contains("STOP"));
    }
}
