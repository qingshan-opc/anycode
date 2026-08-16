//! Compact `tool_result` text for the next LLM hop (not UI / artifacts).

use anycode_core::prelude::*;
use serde_json::Value;

pub(super) fn render_tool_result_for_model(tool_name: &str, output: &ToolOutput) -> String {
    let rendered = match tool_name {
        "Grep" => render_grep(&output.result),
        "Glob" => render_glob(&output.result),
        "FileRead" | "Read" => render_file_read(&output.result),
        "WebSearch" => render_web_search(&output.result),
        "WebFetch" => render_web_fetch(&output.result),
        "Bash" | "PowerShell" => render_shell(&output.result),
        "Skill" => render_skill(&output.result, output.error.is_some()),
        name if is_mcp_result_name(name) => {
            render_mcp(&output.result).unwrap_or_else(|| compact_json(&output.result))
        }
        _ => compact_json(&output.result),
    };

    if let Some(err) = output.error.as_deref() {
        merge_error(err, &rendered, &output.result)
    } else {
        rendered
    }
}

fn is_mcp_result_name(name: &str) -> bool {
    name.starts_with("mcp__")
        || matches!(
            name,
            "Mcp" | "ListMcpResourcesTool" | "ReadMcpResourceTool" | "ReadMcpResourceDir"
        )
}

fn compact_json(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| v.to_string())
}

fn merge_error(err: &str, rendered: &str, result: &Value) -> String {
    let err_trim = err.trim();
    if err_trim.is_empty() {
        return rendered.to_string();
    }
    if rendered.contains(err_trim) {
        return rendered.to_string();
    }
    if result.get("error").and_then(|v| v.as_str()) == Some(err_trim) {
        return rendered.to_string();
    }
    if rendered.is_empty() {
        format!("ERROR: {err_trim}")
    } else {
        format!("ERROR: {err_trim}\n{rendered}")
    }
}

fn render_grep(result: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        lines.push(format!("error: {err}"));
    }
    if let Some(matches) = result.get("matches").and_then(|v| v.as_array()) {
        for m in matches {
            if let Some(line) = flatten_rg_match(m) {
                lines.push(line);
            }
        }
    }
    push_str_array(&mut lines, result.get("files"));
    push_str_array(&mut lines, result.get("counts"));
    if let Some(raw) = result.get("raw_lines").and_then(|v| v.as_array()) {
        for r in raw {
            if let Some(s) = r.as_str() {
                if let Ok(v) = serde_json::from_str::<Value>(s) {
                    match v.get("type").and_then(|t| t.as_str()) {
                        Some("begin" | "end" | "summary") => continue,
                        Some("context" | "match") => {
                            if let Some(line) = flatten_rg_match(&v) {
                                lines.push(line);
                            }
                            continue;
                        }
                        _ => {}
                    }
                }
                if !s.is_empty() && !s.starts_with('{') {
                    lines.push(s.to_string());
                }
            }
        }
    }
    if result.get("truncated").and_then(|v| v.as_bool()) == Some(true) {
        lines.push("truncated=true".into());
    }
    if let Some(code) = result.get("exit_code").and_then(|v| v.as_i64()) {
        if code != 0 && code != 1 {
            lines.push(format!("exit_code={code}"));
        }
    }
    if let Some(stderr) = result.get("stderr").and_then(|v| v.as_str()) {
        if !stderr.trim().is_empty() {
            lines.push(format!("stderr:\n{}", stderr.trim_end()));
        }
    }
    if lines.is_empty() {
        "no matches".into()
    } else {
        lines.join("\n")
    }
}

fn flatten_rg_match(m: &Value) -> Option<String> {
    let data = m.get("data").unwrap_or(m);
    let path = data
        .pointer("/path/text")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("path").and_then(|v| v.as_str()))?;
    let line_no = data.get("line_number").and_then(|v| v.as_u64());
    let text = data
        .pointer("/lines/text")
        .and_then(|v| v.as_str())
        .or_else(|| data.get("text").and_then(|v| v.as_str()))
        .unwrap_or("")
        .trim_end_matches(['\n', '\r']);
    Some(match line_no {
        Some(n) => format!("{path}:{n}:{text}"),
        None => format!("{path}:{text}"),
    })
}

fn push_str_array(lines: &mut Vec<String>, value: Option<&Value>) {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return;
    };
    for item in arr {
        if let Some(s) = item.as_str() {
            lines.push(s.to_string());
        }
    }
}

fn render_glob(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        let path = result.get("path").and_then(|v| v.as_str()).unwrap_or("");
        return if path.is_empty() {
            format!("error: {err}")
        } else {
            format!("error: {err}\npath={path}")
        };
    }
    let files = result
        .get("filenames")
        .or_else(|| result.get("matches"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut lines: Vec<String> = files
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if result.get("truncated").and_then(|v| v.as_bool()) == Some(true) {
        lines.push("truncated=true".into());
    }
    if lines.is_empty() {
        "no files".into()
    } else {
        lines.join("\n")
    }
}

fn render_file_read(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        let mut s = format!("error: {err}");
        if let Some(p) = result.get("path").and_then(|v| v.as_str()) {
            s.push_str("\npath=");
            s.push_str(p);
        }
        if let Some(h) = result.get("hint").and_then(|v| v.as_str()) {
            s.push_str("\nhint=");
            s.push_str(h);
        }
        return s;
    }
    let path = result.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let encoding = result
        .get("encoding")
        .and_then(|v| v.as_str())
        .unwrap_or("utf-8");
    if encoding == "binary" {
        let size = result
            .get("size_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let note = result.get("note").and_then(|v| v.as_str()).unwrap_or("");
        return format!("path={path} encoding=binary size_bytes={size}\n{note}");
    }
    let content = result.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let offset = result.get("offset").and_then(|v| v.as_u64());
    let truncated = result
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut header_lines = Vec::new();
    if !path.is_empty() {
        header_lines.push(format!("path={path}"));
    }
    if let Some(off) = offset {
        let end = result
            .get("end_line")
            .and_then(|v| v.as_u64())
            .unwrap_or(off);
        header_lines.push(format!("lines={off}-{end}"));
    }
    if truncated {
        header_lines.push("truncated=true".into());
    }

    let body = if let Some(off) = offset {
        content
            .lines()
            .enumerate()
            .map(|(i, line)| format!("L{}|{line}", off as usize + i))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        content.to_string()
    };

    if header_lines.is_empty() {
        body
    } else if body.is_empty() {
        header_lines.join("\n")
    } else {
        format!("{}\n{body}", header_lines.join("\n"))
    }
}

fn render_shell(result: &Value) -> String {
    if result.get("background").and_then(|v| v.as_bool()) == Some(true) {
        return compact_json(result);
    }
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        let mut parts = vec![format!("error: {err}")];
        push_named_stream(&mut parts, "stdout", result.get("stdout"));
        push_named_stream(&mut parts, "stderr", result.get("stderr"));
        if let Some(c) = result.get("exit_code").and_then(|v| v.as_i64()) {
            parts.push(format!("exit_code={c}"));
        }
        if let Some(h) = result.get("hint").and_then(|v| v.as_str()) {
            if !h.is_empty() {
                parts.push(h.to_string());
            }
        }
        return parts.join("\n");
    }

    let mut parts = Vec::new();
    let stdout = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    if !stdout.is_empty() {
        parts.push(stdout.trim_end_matches('\n').to_string());
    }
    let stderr = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    if !stderr.is_empty() {
        parts.push(format!("stderr:\n{}", stderr.trim_end_matches('\n')));
    }
    if let Some(c) = result.get("exit_code").and_then(|v| v.as_i64()) {
        parts.push(format!("exit_code={c}"));
    }
    if result.get("stdout_truncated").and_then(|v| v.as_bool()) == Some(true) {
        let dropped = result
            .get("stdout_dropped_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        parts.push(format!("stdout_truncated=true dropped_bytes={dropped}"));
    }
    if result.get("stderr_truncated").and_then(|v| v.as_bool()) == Some(true) {
        let dropped = result
            .get("stderr_dropped_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        parts.push(format!("stderr_truncated=true dropped_bytes={dropped}"));
    }
    if result
        .get("sandbox_escape_ignored")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        parts.push("sandbox_escape_ignored=true".into());
    }
    parts.join("\n")
}

fn push_named_stream(parts: &mut Vec<String>, name: &str, value: Option<&Value>) {
    let Some(s) = value.and_then(|v| v.as_str()) else {
        return;
    };
    if s.is_empty() {
        return;
    }
    parts.push(format!("{name}:\n{}", s.trim_end_matches('\n')));
}

fn render_skill(result: &Value, is_error: bool) -> String {
    let mut obj = result.clone();
    if let Some(map) = obj.as_object_mut() {
        if !is_error {
            map.remove("sop_contract");
        }
        if map
            .get("stderr")
            .and_then(|v| v.as_str())
            .is_some_and(str::is_empty)
        {
            map.remove("stderr");
        }
    }
    compact_json(&obj)
}

fn render_mcp(result: &Value) -> Option<String> {
    let arr = result.get("content").and_then(|c| c.as_array())?;
    let texts: Vec<&str> = arr
        .iter()
        .filter_map(|block| {
            let ty = block.get("type").and_then(|t| t.as_str()).unwrap_or("text");
            if ty == "text" {
                block.get("text").and_then(|t| t.as_str())
            } else {
                None
            }
        })
        .filter(|s| !s.is_empty())
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

fn render_web_search(result: &Value) -> String {
    let mut lines = Vec::new();
    if let Some(provider) = result.get("provider").and_then(|v| v.as_str()) {
        if !provider.is_empty() {
            lines.push(format!("provider={provider}"));
        }
    }
    let payload = match result.get("raw") {
        Some(Value::String(s)) => {
            serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
        }
        Some(other) => other.clone(),
        None => result.clone(),
    };
    let hits = extract_search_hits(&payload);
    if hits.is_empty() {
        match payload {
            Value::String(s) if !s.trim().is_empty() => {
                lines.push(s.chars().take(2_000).collect());
            }
            Value::Object(_) => lines.push("no results".into()),
            other => {
                let compact = compact_json(&other);
                if compact != "{}" && compact != "null" {
                    lines.push(compact.chars().take(2_000).collect());
                }
            }
        }
    } else {
        lines.extend(hits);
    }
    if lines.is_empty() {
        "no results".into()
    } else {
        lines.join("\n")
    }
}

fn extract_search_hits(v: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    push_labeled(&mut lines, "heading", v.get("Heading"));
    push_labeled(&mut lines, "answer", v.get("Answer"));
    push_labeled(
        &mut lines,
        "abstract",
        v.get("AbstractText").or_else(|| v.get("Abstract")),
    );
    if let Some(url) = v.get("AbstractURL").and_then(|x| x.as_str()) {
        if !url.is_empty() {
            lines.push(format!("source={url}"));
        }
    }
    collect_ddg_topics(&mut lines, v.get("RelatedTopics"));
    collect_ddg_topics(&mut lines, v.get("Results"));
    for key in ["results", "organic", "items"] {
        if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
            for item in arr.iter().take(12) {
                if let Some(line) = format_generic_hit(item) {
                    lines.push(line);
                }
            }
        }
    }
    if let Some(arr) = v.pointer("/webPages/value").and_then(|x| x.as_array()) {
        for item in arr.iter().take(12) {
            if let Some(line) = format_generic_hit(item) {
                lines.push(line);
            }
        }
    }
    lines
}

fn push_labeled(lines: &mut Vec<String>, label: &str, value: Option<&Value>) {
    let Some(s) = value.and_then(|v| v.as_str()) else {
        return;
    };
    let s = s.trim();
    if !s.is_empty() {
        lines.push(format!("{label}: {s}"));
    }
}

fn collect_ddg_topics(lines: &mut Vec<String>, value: Option<&Value>) {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return;
    };
    for item in arr.iter().take(16) {
        if let Some(nested) = item.get("Topics").and_then(|t| t.as_array()) {
            for child in nested.iter().take(8) {
                if let Some(line) = format_ddg_topic(child) {
                    lines.push(line);
                }
            }
            continue;
        }
        if let Some(line) = format_ddg_topic(item) {
            lines.push(line);
        }
    }
}

fn format_ddg_topic(item: &Value) -> Option<String> {
    let text = item.get("Text").and_then(|v| v.as_str())?.trim();
    if text.is_empty() {
        return None;
    }
    let url = item
        .get("FirstURL")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    Some(if url.is_empty() {
        text.to_string()
    } else {
        format!("{text} ({url})")
    })
}

fn format_generic_hit(item: &Value) -> Option<String> {
    let title = item
        .get("title")
        .or_else(|| item.get("name"))
        .or_else(|| item.get("Title"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let url = item
        .get("url")
        .or_else(|| item.get("link"))
        .or_else(|| item.get("href"))
        .or_else(|| item.get("URL"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let snippet = item
        .get("snippet")
        .or_else(|| item.get("description"))
        .or_else(|| item.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if title.is_empty() && url.is_empty() && snippet.is_empty() {
        return None;
    }
    let mut line = if title.is_empty() {
        url.to_string()
    } else if url.is_empty() {
        title.to_string()
    } else {
        format!("{title} ({url})")
    };
    if !snippet.is_empty() {
        if !line.is_empty() {
            line.push_str(" — ");
        }
        line.push_str(snippet);
    }
    Some(line)
}

fn render_web_fetch(result: &Value) -> String {
    if let Some(err) = result.get("error").and_then(|v| v.as_str()) {
        return format!("error: {err}");
    }
    let mut lines = Vec::new();
    let code = result.get("code").and_then(|v| v.as_u64());
    let code_text = result
        .get("codeText")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if let Some(c) = code {
        if code_text.is_empty() {
            lines.push(format!("HTTP {c}"));
        } else {
            lines.push(format!("HTTP {c} {code_text}"));
        }
    }
    if result
        .get("body_truncated_to_max_fetch")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        lines.push("truncated=true".into());
    }
    if let Some(note) = result.get("prompt_note").and_then(|v| v.as_str()) {
        if !note.trim().is_empty() {
            lines.push(format!("note: {note}"));
        }
    }
    if let Some(body) = result.get("result").and_then(|v| v.as_str()) {
        lines.push(body.trim_end().to_string());
    }
    if lines.is_empty() {
        compact_json(result)
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn out(name: &str, result: Value, error: Option<&str>) -> String {
        render_tool_result_for_model(
            name,
            &ToolOutput {
                result,
                error: error.map(str::to_string),
                duration_ms: 0,
            },
        )
    }

    #[test]
    fn grep_flattens_rg_json_matches() {
        let result = json!({
            "matches": [{
                "type": "match",
                "data": {
                    "path": {"text": "src/foo.rs"},
                    "lines": {"text": "fn bar() {\n"},
                    "line_number": 12,
                    "absolute_offset": 340,
                    "submatches": [{"match": {"text": "bar"}, "start": 3, "end": 6}]
                }
            }],
            "raw_lines": [],
            "match_count": 1,
            "exit_code": 0,
            "truncated": false,
            "stderr": ""
        });
        let text = out("Grep", result, None);
        assert_eq!(text, "src/foo.rs:12:fn bar() {");
        assert!(!text.contains("absolute_offset"));
        assert!(!text.contains("submatches"));
    }

    #[test]
    fn glob_dedupes_filenames_and_drops_duration() {
        let result = json!({
            "durationMs": 12,
            "numFiles": 2,
            "filenames": ["a.rs", "b.rs"],
            "truncated": true,
            "matches": ["a.rs", "b.rs"],
            "count": 2
        });
        let text = out("Glob", result, None);
        assert_eq!(text, "a.rs\nb.rs\ntruncated=true");
        assert!(!text.contains("durationMs"));
        assert!(!text.contains("numFiles"));
    }

    #[test]
    fn file_read_plain_text_with_header() {
        let result = json!({
            "content": "fn main() {}\n",
            "path": "/abs/src.rs",
            "size_bytes": 12,
            "encoding": "utf-8"
        });
        let text = out("FileRead", result, None);
        assert_eq!(text, "path=/abs/src.rs\nfn main() {}\n");
        assert!(!text.contains("size_bytes"));
        assert!(!text.contains("encoding"));
    }

    #[test]
    fn file_read_range_uses_line_numbers() {
        let result = json!({
            "content": "alpha\nbeta",
            "path": "notes.md",
            "offset": 10,
            "end_line": 11,
            "truncated": false
        });
        let text = out("FileRead", result, None);
        assert!(text.contains("lines=10-11"));
        assert!(text.contains("L10|alpha"));
        assert!(text.contains("L11|beta"));
        assert!(text.starts_with("path=notes.md\nlines=10-11\n"));
    }

    #[test]
    fn bash_omits_false_truncation_flags() {
        let result = json!({
            "stdout": "hi\n",
            "stderr": "",
            "exit_code": 0,
            "stdout_truncated": false,
            "stderr_truncated": false,
            "stdout_dropped_bytes": 0,
            "stderr_dropped_bytes": 0,
            "sandbox_escape_ignored": false
        });
        assert_eq!(out("Bash", result, None), "hi\nexit_code=0");
    }

    #[test]
    fn bash_failure_does_not_repeat_error_prefix_when_json_has_error() {
        let result = json!({
            "error": "timed out",
            "stdout": "",
            "stderr": "still starting",
            "exit_code": 124,
            "hint": "use run_in_background"
        });
        let text = out("Bash", result, Some("Command timed out"));
        assert!(text.starts_with("ERROR: Command timed out"));
        assert!(text.contains("stderr:\nstill starting"));
        assert!(text.contains("exit_code=124"));
        assert!(!text.contains("RESULT:"));
    }

    #[test]
    fn command_failed_does_not_duplicate_when_exit_code_present() {
        let result = json!({
            "stdout": "",
            "stderr": "ls: no such file\n",
            "exit_code": 2,
            "stdout_truncated": false,
            "stderr_truncated": false,
            "stdout_dropped_bytes": 0,
            "stderr_dropped_bytes": 0,
            "sandbox_escape_ignored": false
        });
        let text = out("Bash", result, Some("Command failed"));
        assert!(text.starts_with("ERROR: Command failed"));
        assert!(text.contains("exit_code=2"));
        assert!(!text.contains("RESULT:"));
        assert!(!text.contains("stdout_truncated"));
    }

    #[test]
    fn file_not_found_does_not_duplicate_json_error() {
        let result = json!({
            "error": "File not found",
            "path": "/missing.rs"
        });
        let text = out("FileRead", result, Some("File not found"));
        assert_eq!(text, "error: File not found\npath=/missing.rs");
        assert!(!text.contains("ERROR:"));
        assert!(!text.contains("RESULT:"));
    }

    #[test]
    fn mcp_unwraps_text_content_blocks() {
        let result = json!({
            "content": [{"type": "text", "text": "hello from mcp"}],
            "isError": false,
            "structuredContent": {"x": 1}
        });
        assert_eq!(out("mcp__server__tool", result, None), "hello from mcp");
    }

    #[test]
    fn skill_success_drops_sop_contract() {
        let result = json!({
            "stdout": "ok",
            "stderr": "",
            "code": 0,
            "sop_contract": "## Inputs\n...",
            "acceptance": []
        });
        let text = out("Skill", result, None);
        assert!(text.contains("ok"));
        assert!(!text.contains("sop_contract"));
    }

    #[test]
    fn web_search_unwraps_ddg_raw_json() {
        let raw = serde_json::json!({
            "Heading": "Rust",
            "AbstractText": "A language",
            "AbstractURL": "https://www.rust-lang.org/",
            "RelatedTopics": [
                {"Text": "Cargo", "FirstURL": "https://doc.rust-lang.org/cargo/"},
                {"Topics": [{"Text": "Nested", "FirstURL": "https://example.com/n"}]}
            ]
        })
        .to_string();
        let text = out(
            "WebSearch",
            json!({ "provider": "duckduckgo", "raw": raw }),
            None,
        );
        assert!(text.contains("provider=duckduckgo"));
        assert!(text.contains("heading: Rust"));
        assert!(text.contains("abstract: A language"));
        assert!(text.contains("source=https://www.rust-lang.org/"));
        assert!(text.contains("Cargo (https://doc.rust-lang.org/cargo/)"));
        assert!(text.contains("Nested (https://example.com/n)"));
        assert!(!text.contains("RelatedTopics"));
        assert!(!text.contains("\"raw\""));
    }

    #[test]
    fn web_fetch_drops_duration_and_keeps_status_body() {
        let text = out(
            "WebFetch",
            json!({
                "bytes": 12,
                "code": 200,
                "codeText": "OK",
                "result": "hello body",
                "durationMs": 321,
                "prompt_note": "",
                "body_truncated_to_max_fetch": false
            }),
            None,
        );
        assert_eq!(text, "HTTP 200 OK\nhello body");
        assert!(!text.contains("durationMs"));
        assert!(!text.contains("bytes"));
    }

    #[test]
    fn bash_empty_stdout_still_emits_exit_zero() {
        let result = json!({
            "stdout": "",
            "stderr": "",
            "exit_code": 0,
            "stdout_truncated": false,
            "stderr_truncated": false
        });
        assert_eq!(out("Bash", result, None), "exit_code=0");
    }

    #[test]
    fn grep_drops_rg_json_control_events_in_raw_lines() {
        let begin = r#"{"type":"begin","data":{"path":{"text":"a.rs"}}}"#;
        let summary = r#"{"type":"summary","data":{"stats":{"matched_lines":1}}}"#;
        let context = r#"{"type":"context","data":{"path":{"text":"a.rs"},"lines":{"text":"prev\n"},"line_number":11}}"#;
        let text = out(
            "Grep",
            json!({
                "matches": [{
                    "type": "match",
                    "data": {
                        "path": {"text": "a.rs"},
                        "lines": {"text": "fn bar() {\n"},
                        "line_number": 12
                    }
                }],
                "raw_lines": [begin, context, summary],
                "exit_code": 0,
                "truncated": false,
                "stderr": ""
            }),
            None,
        );
        assert_eq!(text, "a.rs:12:fn bar() {\na.rs:11:prev");
        assert!(!text.contains("\"type\":\"begin\""));
        assert!(!text.contains("matched_lines"));
    }
}
