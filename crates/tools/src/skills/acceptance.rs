//! Declarative post-run acceptance checks declared in `SKILL.md` frontmatter.
//!
//! Weak chat models (e.g. deepseek-v4-flash) cannot reliably self-verify their
//! own deliverables. Skills declare machine-checkable outcomes:
//!
//! ```yaml
//! acceptance:
//!   - type: file-exists
//!     path: "out/report.docx"
//!   - type: ooxml-openable
//!     path: "out/report.docx"
//! ```
//!
//! The `Skill` tool runs these after the `run` script exits; failures are
//! surfaced as a tool error so the agent loop turns them into a repair turn.

use std::path::Path;

/// One declarative check from frontmatter `acceptance:`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SkillAcceptanceCheck {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AcceptanceOutcome {
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub ok: bool,
    pub detail: String,
}

/// Run all checks; `path` is resolved against `cwd` (the task working
/// directory the skill ran in) unless absolute.
#[must_use]
pub fn run_acceptance_checks(
    checks: &[SkillAcceptanceCheck],
    cwd: &Path,
) -> Vec<AcceptanceOutcome> {
    checks
        .iter()
        .map(|c| {
            let rel = c.path.trim();
            let full = if Path::new(rel).is_absolute() {
                std::path::PathBuf::from(rel)
            } else {
                cwd.join(rel)
            };
            let (ok, detail) = evaluate(&c.kind, &full);
            AcceptanceOutcome {
                kind: c.kind.clone(),
                path: rel.to_string(),
                ok,
                detail,
            }
        })
        .collect()
}

fn evaluate(kind: &str, full: &Path) -> (bool, String) {
    match kind.trim() {
        "file-exists" => {
            if full.is_file() {
                (true, "exists".into())
            } else {
                (false, format!("{} not found", full.display()))
            }
        }
        "file-nonempty" => match std::fs::metadata(full) {
            Ok(m) if m.is_file() && m.len() > 0 => (true, format!("{} bytes", m.len())),
            Ok(_) => (false, format!("{} empty or not a file", full.display())),
            Err(e) => (false, format!("{}: {e}", full.display())),
        },
        "ooxml-openable" => match crate::verification::office::zip_openable(full) {
            Ok(()) => (true, "zip/OOXML openable".into()),
            Err(e) => (false, e),
        },
        other => (
            false,
            format!("unknown acceptance check type `{other}` (expected file-exists | file-nonempty | ooxml-openable)"),
        ),
    }
}

/// One-line summary for tool errors, e.g. `ooxml-openable out/x.docx: bad zip`.
#[must_use]
pub fn summarize_failures(outcomes: &[AcceptanceOutcome]) -> Option<String> {
    let failed: Vec<String> = outcomes
        .iter()
        .filter(|o| !o.ok)
        .map(|o| format!("{} {}: {}", o.kind, o.path, o.detail))
        .collect();
    if failed.is_empty() {
        None
    } else {
        Some(failed.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_exists_and_nonempty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hi").unwrap();
        std::fs::write(tmp.path().join("empty.txt"), "").unwrap();

        let checks = vec![
            SkillAcceptanceCheck {
                kind: "file-exists".into(),
                path: "a.txt".into(),
            },
            SkillAcceptanceCheck {
                kind: "file-nonempty".into(),
                path: "empty.txt".into(),
            },
            SkillAcceptanceCheck {
                kind: "file-exists".into(),
                path: "missing.txt".into(),
            },
        ];
        let out = run_acceptance_checks(&checks, tmp.path());
        assert!(out[0].ok);
        assert!(!out[1].ok);
        assert!(!out[2].ok);
        let summary = summarize_failures(&out).unwrap();
        assert!(summary.contains("empty.txt"));
        assert!(summary.contains("missing.txt"));
    }

    #[test]
    fn ooxml_openable_accepts_real_zip() {
        let tmp = tempfile::tempdir().unwrap();
        // Minimal valid zip (empty archive) counts as openable container.
        let zip_path = tmp.path().join("t.docx");
        {
            let f = std::fs::File::create(&zip_path).unwrap();
            let mut zw = zip::ZipWriter::new(f);
            zw.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            use std::io::Write;
            zw.write_all(b"<w:document/>").unwrap();
            zw.finish().unwrap();
        }
        let checks = vec![SkillAcceptanceCheck {
            kind: "ooxml-openable".into(),
            path: "t.docx".into(),
        }];
        let out = run_acceptance_checks(&checks, tmp.path());
        assert!(out[0].ok, "detail: {}", out[0].detail);
    }

    #[test]
    fn unknown_type_fails_loudly() {
        let tmp = tempfile::tempdir().unwrap();
        let checks = vec![SkillAcceptanceCheck {
            kind: "magic".into(),
            path: "x".into(),
        }];
        let out = run_acceptance_checks(&checks, tmp.path());
        assert!(!out[0].ok);
        assert!(out[0].detail.contains("unknown acceptance check type"));
    }
}
