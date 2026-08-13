//! Code-task validators: workspace-level stack verification for CrossFileCoding.
//!
//! 与产物 validators 不同,这两个 validator 不绑定具体 artifact,而是在 workspace
//! 根探测栈并运行官方验证命令:
//! - `code.stack_verify` (P0): Cargo.toml→`cargo check`;tsconfig.json+本地 tsc→
//!   `tsc --noEmit`;*.py→`py_compile` 全量;探测不到栈→Passed(无可验证对象)。
//! - `code.stack_tests` (P1): Cargo.toml→`cargo test`;pytest 工程→`pytest`。
//! 超时/工具链缺失 → EnvironmentFailed(不阻断完成,由守卫归为 Partial)。

use super::{ArtifactValidator, ValidationContext};
use anycode_core::{
    Artifact, ExpectedArtifact, GateRequirement, VerificationOutcome, VerificationResult,
};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

const MAX_PY_FILES: usize = 50;
const MAX_DIAG_OUTPUT: usize = 2_000;

pub struct CodeStackVerifyValidator;
pub struct CodeStackTestsValidator;

#[async_trait::async_trait]
impl ArtifactValidator for CodeStackVerifyValidator {
    fn id(&self) -> &'static str {
        "code.stack_verify"
    }
    fn version(&self) -> &'static str {
        "1"
    }
    async fn validate(
        &self,
        requirement: &GateRequirement,
        _expected: &ExpectedArtifact,
        _candidates: &[Artifact],
        context: &ValidationContext,
    ) -> VerificationResult {
        let ws = &context.workspace;
        let timeout = Duration::from_millis(requirement.timeout_ms.max(5_000));
        if ws.join("Cargo.toml").is_file() {
            return run_gate(requirement, ws, "cargo", &["check"], timeout).await;
        }
        let tsc = ws.join("node_modules").join(".bin").join("tsc");
        if ws.join("tsconfig.json").is_file() {
            if !tsc.is_file() {
                return env_failed(
                    requirement,
                    "tsc_missing",
                    "tsconfig.json present but node_modules/.bin/tsc not found (run install first)",
                );
            }
            return run_gate(
                requirement,
                ws,
                &tsc.to_string_lossy(),
                &["--noEmit"],
                timeout,
            )
            .await;
        }
        let py_files = collect_py_files(ws);
        if !py_files.is_empty() {
            let mut failures = Vec::new();
            for f in &py_files {
                let r = run_gate(
                    requirement,
                    ws,
                    "python3",
                    &["-m", "py_compile", &f.to_string_lossy()],
                    timeout,
                )
                .await;
                match r.outcome {
                    VerificationOutcome::Passed => {}
                    VerificationOutcome::EnvironmentFailed => return r,
                    VerificationOutcome::TaskFailed => {
                        failures.push(format!("{}: {}", f.display(), r.diagnostics.join("; ")));
                    }
                }
            }
            return if failures.is_empty() {
                passed(
                    requirement,
                    vec![format!("py_compile ok ({} files)", py_files.len())],
                )
            } else {
                task_failed(requirement, "py_compile_failed", failures)
            };
        }
        // 探测不到栈:Info 级放行(占位通过不算验证证据,守卫的 evidence
        // repair 仍可对「写了代码但无处验证」的会话生效)。
        passed_info(
            requirement,
            vec!["no detectable code stack (Cargo.toml/tsconfig.json/*.py)".into()],
        )
    }
}

#[async_trait::async_trait]
impl ArtifactValidator for CodeStackTestsValidator {
    fn id(&self) -> &'static str {
        "code.stack_tests"
    }
    fn version(&self) -> &'static str {
        "1"
    }
    async fn validate(
        &self,
        requirement: &GateRequirement,
        _expected: &ExpectedArtifact,
        _candidates: &[Artifact],
        context: &ValidationContext,
    ) -> VerificationResult {
        let ws = &context.workspace;
        let timeout = Duration::from_millis(requirement.timeout_ms.max(5_000));
        if ws.join("Cargo.toml").is_file() {
            return run_gate(requirement, ws, "cargo", &["test", "--workspace"], timeout).await;
        }
        if has_pytest_project(ws) {
            return run_gate(requirement, ws, "python3", &["-m", "pytest", "-q"], timeout).await;
        }
        passed_info(requirement, vec!["no test gate for detected stack".into()])
    }
}

fn has_pytest_project(ws: &Path) -> bool {
    ws.join("pytest.ini").is_file()
        || ws.join("pyproject.toml").is_file()
        || ws.join("setup.cfg").is_file()
}

fn collect_py_files(ws: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(ws.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 6 || out.len() >= MAX_PY_FILES {
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for ent in read.flatten() {
            let path = ent.path();
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if path.is_symlink() || name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if matches!(
                    name.as_ref(),
                    "node_modules" | "target" | "venv" | "__pycache__"
                ) {
                    continue;
                }
                stack.push((path, depth + 1));
            } else if name.ends_with(".py") {
                out.push(path);
                if out.len() >= MAX_PY_FILES {
                    break;
                }
            }
        }
    }
    out
}

async fn run_gate(
    requirement: &GateRequirement,
    cwd: &Path,
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> VerificationResult {
    let cmd_line = format!("{program} {}", args.join(" "));
    let child = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => {
            return env_failed(
                requirement,
                "toolchain_missing",
                &format!("failed to spawn `{cmd_line}`: {e}"),
            )
        }
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Err(_) => env_failed(
            requirement,
            "gate_timeout",
            &format!("`{cmd_line}` exceeded {}ms", timeout.as_millis()),
        ),
        Ok(Err(e)) => env_failed(
            requirement,
            "toolchain_missing",
            &format!("`{cmd_line}` wait failed: {e}"),
        ),
        Ok(Ok(out)) if out.status.success() => passed(requirement, vec![cmd_line]),
        Ok(Ok(out)) => {
            let mut diag = String::from_utf8_lossy(&out.stderr).into_owned();
            if diag.trim().is_empty() {
                diag = String::from_utf8_lossy(&out.stdout).into_owned();
            }
            let tail: String = diag.chars().take(MAX_DIAG_OUTPUT).collect();
            task_failed(
                requirement,
                "stack_verify_failed",
                vec![format!("`{cmd_line}` exited {:?}", out.status.code()), tail],
            )
        }
    }
}

fn passed(requirement: &GateRequirement, diagnostics: Vec<String>) -> VerificationResult {
    VerificationResult {
        gate_id: requirement.id.clone(),
        validator_id: requirement.validator_id.clone(),
        validator_version: "1".into(),
        outcome: VerificationOutcome::Passed,
        severity: requirement.severity,
        artifact_path: None,
        artifact_hash: None,
        error_code: None,
        diagnostics,
        evidence_paths: vec![],
    }
}

/// 占位通过(无验证对象):降级为 Info,不计入「已验证」证据。
fn passed_info(requirement: &GateRequirement, diagnostics: Vec<String>) -> VerificationResult {
    VerificationResult {
        severity: anycode_core::GateSeverity::Info,
        ..passed(requirement, diagnostics)
    }
}

fn env_failed(requirement: &GateRequirement, code: &str, detail: &str) -> VerificationResult {
    VerificationResult {
        gate_id: requirement.id.clone(),
        validator_id: requirement.validator_id.clone(),
        validator_version: "1".into(),
        outcome: VerificationOutcome::EnvironmentFailed,
        severity: requirement.severity,
        artifact_path: None,
        artifact_hash: None,
        error_code: Some(code.into()),
        diagnostics: vec![detail.to_string()],
        evidence_paths: vec![],
    }
}

fn task_failed(
    requirement: &GateRequirement,
    code: &str,
    diagnostics: Vec<String>,
) -> VerificationResult {
    VerificationResult {
        gate_id: requirement.id.clone(),
        validator_id: requirement.validator_id.clone(),
        validator_version: "1".into(),
        outcome: VerificationOutcome::TaskFailed,
        severity: requirement.severity,
        artifact_path: None,
        artifact_hash: None,
        error_code: Some(code.into()),
        diagnostics,
        evidence_paths: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::GateSeverity;
    use std::collections::HashMap;

    fn req() -> GateRequirement {
        GateRequirement {
            id: "code.stack_verify.workspace".into(),
            validator_id: "code.stack_verify".into(),
            artifact_ref: "workspace".into(),
            severity: GateSeverity::P0,
            timeout_ms: 30_000,
        }
    }

    fn ctx(dir: &Path) -> ValidationContext {
        ValidationContext {
            workspace: dir.to_path_buf(),
            extras: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn empty_workspace_passes_with_no_stack() {
        let temp = tempfile::tempdir().unwrap();
        let r = CodeStackVerifyValidator
            .validate(&req(), &expected(), &[], &ctx(temp.path()))
            .await;
        assert_eq!(r.outcome, VerificationOutcome::Passed);
    }

    #[tokio::test]
    async fn python_syntax_error_fails() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("bad.py"), "def broken(:\n").unwrap();
        let r = CodeStackVerifyValidator
            .validate(&req(), &expected(), &[], &ctx(temp.path()))
            .await;
        assert_eq!(r.outcome, VerificationOutcome::TaskFailed);
        assert_eq!(r.error_code.as_deref(), Some("py_compile_failed"));
    }

    #[tokio::test]
    async fn python_ok_passes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("ok.py"), "x = 1\n").unwrap();
        let r = CodeStackVerifyValidator
            .validate(&req(), &expected(), &[], &ctx(temp.path()))
            .await;
        assert_eq!(r.outcome, VerificationOutcome::Passed);
    }

    #[tokio::test]
    async fn tsconfig_without_tsc_is_env_failure() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tsconfig.json"), "{}").unwrap();
        let r = CodeStackVerifyValidator
            .validate(&req(), &expected(), &[], &ctx(temp.path()))
            .await;
        assert_eq!(r.outcome, VerificationOutcome::EnvironmentFailed);
        assert_eq!(r.error_code.as_deref(), Some("tsc_missing"));
    }

    fn expected() -> ExpectedArtifact {
        ExpectedArtifact {
            id: "workspace".into(),
            kind: "file".into(),
            required: true,
            path_globs: vec![],
        }
    }
}
