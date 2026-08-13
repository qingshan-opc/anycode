//! P0.2 产物反推 family 兜底:`infer_family` 关键词误判(如写了 `.html` 却判成
//! General)不再导致完成守卫静默跳过——从会话内写过的文件路径确定性反推
//! TaskFamily,并就地合成 GatePlan 跑对应门禁。
//!
//! 边界:
//! - 只看写工具落过的路径(FileWrite/Edit/NotebookEdit)。已申报产物在申报点
//!   已被 delivery_acceptance 验收,不在此重复兜底。
//! - markdown 不算交付信号(改 README/笔记不触发办公交付门禁)。
//! - 反推优先级:WebDesign > OfficeDelivery > CrossFileCoding(交付物门禁优先于
//!   代码门禁;代码门禁为 workspace 级,不绑定具体产物)。

use anycode_core::{ExpectedArtifact, TaskFamily};
use std::collections::BTreeSet;

/// 从写文件痕迹反推 (family, 合成的 expected artifacts)。
/// 无 gateable 信号时返回 None(纯文本回答、仅写 md/txt 等)。
pub fn infer_family_from_signals(
    written_paths: &[String],
) -> Option<(TaskFamily, Vec<ExpectedArtifact>)> {
    let paths: BTreeSet<&String> = written_paths.iter().collect();
    if paths.is_empty() {
        return None;
    }

    let mut web = Vec::new();
    let mut office = Vec::new();
    let mut wrote_code = false;
    for p in paths.iter().copied() {
        let ext = std::path::Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "html" | "htm" => web.push(("html", p)),
            "docx" => office.push(("docx", p)),
            "pptx" => office.push(("pptx", p)),
            "xlsx" => office.push(("xlsx", p)),
            "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "py" => wrote_code = true,
            _ => {}
        }
    }

    if !web.is_empty() {
        return Some((TaskFamily::WebDesign, synthesize_expected(web)));
    }
    if !office.is_empty() {
        return Some((TaskFamily::OfficeDelivery, synthesize_expected(office)));
    }
    // 代码门禁为 workspace 级(cargo check / tsc / py_compile),无需逐文件 expected。
    if wrote_code {
        return Some((TaskFamily::CrossFileCoding, Vec::new()));
    }
    None
}

fn synthesize_expected(files: Vec<(&str, &String)>) -> Vec<ExpectedArtifact> {
    files
        .into_iter()
        .enumerate()
        .map(|(i, (kind, path))| ExpectedArtifact {
            id: format!("fallback_{kind}_{i}"),
            kind: kind.to_string(),
            required: true,
            // 全路径作为 glob:path_matches_glob 的 ends_with 语义可精确命中。
            path_globs: vec![path.clone()],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_write_infers_web_design() {
        let (family, expected) =
            infer_family_from_signals(&["/w/site/index.html".to_string()]).unwrap();
        assert_eq!(family, TaskFamily::WebDesign);
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].kind, "html");
        assert_eq!(expected[0].path_globs, vec!["/w/site/index.html"]);
    }

    #[test]
    fn docx_write_infers_office() {
        let (family, expected) =
            infer_family_from_signals(&["/w/report.docx".to_string()]).unwrap();
        assert_eq!(family, TaskFamily::OfficeDelivery);
        assert_eq!(expected[0].kind, "docx");
    }

    #[test]
    fn rust_write_infers_coding_without_expected() {
        let (family, expected) =
            infer_family_from_signals(&["/w/src/main.rs".to_string()]).unwrap();
        assert_eq!(family, TaskFamily::CrossFileCoding);
        assert!(expected.is_empty());
    }

    #[test]
    fn web_beats_code_when_both_written() {
        let (family, _) =
            infer_family_from_signals(&["/w/src/main.rs".to_string(), "/w/index.html".to_string()])
                .unwrap();
        assert_eq!(family, TaskFamily::WebDesign);
    }

    #[test]
    fn pure_text_session_has_no_fallback() {
        assert!(infer_family_from_signals(&[]).is_none());
        assert!(infer_family_from_signals(&["/w/notes.txt".to_string()]).is_none());
        // markdown 不是交付信号。
        assert!(infer_family_from_signals(&["/w/README.md".to_string()]).is_none());
    }
}
