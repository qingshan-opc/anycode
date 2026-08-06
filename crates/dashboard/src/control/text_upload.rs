//! Persist text reference uploads for web-chat stdin protocol.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextFilePayload {
    pub filename: String,
    pub content: String,
}

const MAX_FILES: usize = 10;
/// 提取后文本硬顶：超过即截断（不拒绝），防止巨型文件撑爆上下文与磁盘。
const MAX_EXTRACTED_BYTES: usize = 2 * 1024 * 1024;
/// 单文件提取文本超过此阈值即落盘走指针引用，不内联进 prompt。
const INLINE_FILE_MAX_BYTES: usize = 64 * 1024;
/// 每条消息内联附件总预算。
const INLINE_TOTAL_BUDGET: usize = 200 * 1024;
/// 指针块里附带的头部预览字节数。
const POINTER_HEAD_BYTES: usize = 2 * 1024;
const ALLOWED_EXTENSIONS: &[&str] = &[
    "txt", "md", "json", "csv", "log", "pdf", "xlsx", "docx", "pptx",
];

fn truncate_extracted(text: String) -> String {
    if text.len() <= MAX_EXTRACTED_BYTES {
        return text;
    }
    let mut truncated = truncate_at_char_boundary(&text, MAX_EXTRACTED_BYTES).to_string();
    truncated.push_str("\n...[truncated at 2 MB]");
    truncated
}

fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn human_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

pub fn validate_text_payloads(files: &[TextFilePayload]) -> Result<()> {
    if files.len() > MAX_FILES {
        bail!("at most {MAX_FILES} text files per message");
    }
    for (i, f) in files.iter().enumerate() {
        let name = f.filename.trim();
        if name.is_empty() {
            bail!("text file {i}: filename is required");
        }
        let ext = Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
            bail!(
                "text file {i}: unsupported type (allowed: .txt .md .json .csv .log .pdf .xlsx .docx .pptx)"
            );
        }
        let text = normalize_upload_content(&f.filename, &f.content)?;
        if text.trim().is_empty() {
            bail!("text file {i}: content is empty");
        }
    }
    Ok(())
}

pub fn write_text_payloads(session_id: &str, files: &[TextFilePayload]) -> Result<Vec<PathBuf>> {
    if files.is_empty() {
        return Ok(vec![]);
    }
    validate_text_payloads(files)?;
    let dir = crate::cancel_ipc::dashboard_state_dir()
        .join("uploads")
        .join(session_id);
    std::fs::create_dir_all(&dir)?;
    let mut paths = Vec::with_capacity(files.len());
    for f in files {
        let safe_name = sanitize_filename(&f.filename);
        let path = dir.join(format!("{}-{}", Uuid::new_v4().simple(), safe_name));
        let text = truncate_extracted(normalize_upload_content(&f.filename, &f.content)?);
        std::fs::write(&path, text)?;
        paths.push(path);
    }
    Ok(paths)
}

pub fn text_file_line(path: &Path) -> String {
    format!("@anycode/text-file:{}\n", path.display())
}

/// Inline uploaded text/office content into the user prompt for embedded chat.
///
/// 小文件直接内联；超过 `INLINE_FILE_MAX_BYTES` 或总预算的文件落盘到
/// `uploads/<session_id>/`，prompt 里只放 `@anycode/text-file:` 指针 + 头部预览，
/// agent 用 FileRead/Grep 自取。
pub fn append_to_prompt_routed(
    prompt: &str,
    session_id: &str,
    files: Option<&[TextFilePayload]>,
) -> Result<String> {
    let Some(files) = files else {
        return Ok(prompt.to_string());
    };
    if files.is_empty() {
        return Ok(prompt.to_string());
    }
    validate_text_payloads(files)?;
    let mut out = prompt.to_string();
    let mut inline_used = 0usize;
    for f in files {
        let text = truncate_extracted(normalize_upload_content(&f.filename, &f.content)?);
        let fits_inline =
            text.len() <= INLINE_FILE_MAX_BYTES && inline_used + text.len() <= INLINE_TOTAL_BUDGET;
        if fits_inline {
            inline_used += text.len();
            out.push_str(&format!(
                "\n\n--- attached: {} ---\n{}",
                f.filename.trim(),
                text
            ));
            continue;
        }
        let paths = write_text_payloads(session_id, std::slice::from_ref(f))?;
        let Some(path) = paths.first() else {
            bail!("failed to persist uploaded file {}", f.filename.trim());
        };
        let head = truncate_at_char_boundary(&text, POINTER_HEAD_BYTES);
        out.push_str(&format!(
            "\n\n--- attached: {} (saved to disk, {}) ---\n{}Head:\n{}\n\
             Use FileRead to read this file and Grep to search it; do not ask the user to paste it.",
            f.filename.trim(),
            human_size(text.len()),
            text_file_line(path),
            head,
        ));
    }
    Ok(out)
}

/// Best-effort cleanup of `uploads/<session_id>` dirs idle longer than `max_age`.
pub fn sweep_uploads_dir(max_age: std::time::Duration) {
    let dir = crate::cancel_ipc::dashboard_state_dir().join("uploads");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let newest_mtime = std::fs::read_dir(&path)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok()?.modified().ok())
            .max();
        match newest_mtime {
            // 空目录或读不到 mtime：一并清掉
            None => {
                let _ = std::fs::remove_dir_all(&path);
            }
            Some(mtime)
                if now
                    .duration_since(mtime)
                    .map(|d| d > max_age)
                    .unwrap_or(false) =>
            {
                let _ = std::fs::remove_dir_all(&path);
            }
            _ => {}
        }
    }
}

fn normalize_upload_content(filename: &str, content: &str) -> Result<String> {
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => {
            let raw = decode_base64_payload("pdf", content)?;
            pdf_extract::extract_text_from_mem(&raw)
                .map_err(|e| anyhow::anyhow!("pdf text extraction failed: {e}"))
        }
        "xlsx" => {
            super::office_extract::extract_xlsx_text(&decode_base64_payload("xlsx", content)?)
        }
        "docx" => {
            super::office_extract::extract_docx_text(&decode_base64_payload("docx", content)?)
        }
        "pptx" => {
            super::office_extract::extract_pptx_text(&decode_base64_payload("pptx", content)?)
        }
        _ => Ok(content.to_string()),
    }
}

fn decode_base64_payload(ext: &str, content: &str) -> Result<Vec<u8>> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, content.trim())
        .map_err(|e| anyhow::anyhow!("{ext} file: invalid base64 payload: {e}"))
}

fn sanitize_filename(name: &str) -> String {
    let base = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.txt");
    base.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// env 是进程全局的，凡是依赖 ANYCODE_DASHBOARD_STATE_DIR 的测试必须串行。
    fn with_state_dir<R>(f: impl FnOnce(&Path) -> R) -> R {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = ENV_LOCK.lock().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("ANYCODE_DASHBOARD_STATE_DIR", tmp.path());
        let r = f(tmp.path());
        std::env::remove_var("ANYCODE_DASHBOARD_STATE_DIR");
        r
    }

    #[test]
    fn rejects_unsupported_extension() {
        let err = validate_text_payloads(&[TextFilePayload {
            filename: "evil.exe".into(),
            content: "x".into(),
        }])
        .unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }

    #[test]
    fn rejects_too_many_files() {
        let files: Vec<TextFilePayload> = (0..MAX_FILES + 1)
            .map(|i| TextFilePayload {
                filename: format!("f{i}.txt"),
                content: "x".into(),
            })
            .collect();
        let err = validate_text_payloads(&files).unwrap_err();
        assert!(err.to_string().contains("at most"));
    }

    #[test]
    fn accepts_office_extensions_and_decodes_base64() {
        // 扩展名通过校验；无效 base64 报解码错误而非 unsupported
        for name in ["a.xlsx", "a.docx", "a.pptx"] {
            let err = validate_text_payloads(&[TextFilePayload {
                filename: name.into(),
                content: "!!!not-base64!!!".into(),
            }])
            .unwrap_err();
            assert!(
                err.to_string().contains("invalid base64"),
                "{name}: unexpected error {err}"
            );
        }
    }

    #[test]
    fn routed_inlines_small_files() {
        let out = append_to_prompt_routed(
            "hello",
            "sess-small",
            Some(&[TextFilePayload {
                filename: "note.txt".into(),
                content: "file body".into(),
            }]),
        )
        .unwrap();
        assert!(out.starts_with("hello"));
        assert!(out.contains("--- attached: note.txt ---"));
        assert!(out.contains("file body"));
        assert!(!out.contains("@anycode/text-file:"));
    }

    #[test]
    fn routed_points_large_file_to_disk() {
        with_state_dir(|state_dir| {
            let big = "y".repeat(INLINE_FILE_MAX_BYTES + 1024);
            let out = append_to_prompt_routed(
                "hello",
                "sess-big",
                Some(&[TextFilePayload {
                    filename: "big.txt".into(),
                    content: big.clone(),
                }]),
            )
            .unwrap();
            assert!(out.contains("--- attached: big.txt (saved to disk,"));
            assert!(out.contains("@anycode/text-file:"));
            assert!(out.contains("Head:\n"));
            assert!(out.contains("Use FileRead to read this file"));
            assert!(!out.contains(&big), "全文不应内联");
            // 磁盘文件存在且内容完整
            let dir = state_dir.join("uploads").join("sess-big");
            let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
            assert_eq!(entries.len(), 1);
            let on_disk = std::fs::read_to_string(entries[0].path()).unwrap();
            assert_eq!(on_disk, big);
        });
    }

    #[test]
    fn routed_total_budget_overflow_goes_to_disk() {
        with_state_dir(|_| {
            // 每个文件都在 INLINE_FILE_MAX_BYTES 内；前 3 个吃掉预算，第 4 个溢出落盘
            let chunk = "a".repeat(60 * 1024);
            let files: Vec<TextFilePayload> = (0..4)
                .map(|i| TextFilePayload {
                    filename: format!("f{i}.txt"),
                    content: chunk.clone(),
                })
                .collect();
            let out = append_to_prompt_routed("hello", "sess-budget", Some(&files)).unwrap();
            assert!(out.contains("--- attached: f0.txt ---"));
            assert!(out.contains("--- attached: f2.txt ---"));
            assert!(out.contains("--- attached: f3.txt (saved to disk,"));
        });
    }

    #[test]
    fn oversized_extracted_text_is_truncated_not_rejected() {
        with_state_dir(|state_dir| {
            let huge = "z".repeat(MAX_EXTRACTED_BYTES + 100);
            let out = append_to_prompt_routed(
                "hi",
                "sess-huge",
                Some(&[TextFilePayload {
                    filename: "huge.txt".into(),
                    content: huge,
                }]),
            )
            .unwrap();
            // 超硬顶的文件走指针路由：截断标记落在磁盘文件里，prompt 只有 head 预览
            assert!(out.contains("@anycode/text-file:"));
            let dir = state_dir.join("uploads").join("sess-huge");
            let entry = std::fs::read_dir(&dir).unwrap().flatten().next().unwrap();
            let on_disk = std::fs::read_to_string(entry.path()).unwrap();
            assert!(on_disk.ends_with("[truncated at 2 MB]"));
            assert!(on_disk.len() <= MAX_EXTRACTED_BYTES + 64);
        });
    }

    #[test]
    fn sweep_removes_only_stale_upload_dirs() {
        with_state_dir(|state_dir| {
            let stale = state_dir.join("uploads").join("sess-stale");
            let fresh = state_dir.join("uploads").join("sess-fresh");
            std::fs::create_dir_all(&stale).unwrap();
            std::fs::create_dir_all(&fresh).unwrap();
            std::fs::write(stale.join("old.txt"), "x").unwrap();
            std::fs::write(fresh.join("new.txt"), "x").unwrap();
            // stale 目录里的文件回拨 mtime 到 8 天前
            let old = filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 3600),
            );
            filetime::set_file_mtime(stale.join("old.txt"), old).unwrap();

            sweep_uploads_dir(std::time::Duration::from_secs(7 * 24 * 3600));
            assert!(!stale.exists());
            assert!(fresh.exists());
        });
    }
}
