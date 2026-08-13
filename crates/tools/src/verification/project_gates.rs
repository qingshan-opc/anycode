//! P1.7 项目级自定义门禁:`{workspace}/.anycode/gates/` 下的脚本在完成守卫期执行,
//! 类 Claude Code Stop hook 语义,但吸取教训——**超时语义显式,默认 fail-closed**
//! (超时/无法执行 = gate 失败),可经 plan extras 显式开 fail-open。
//!
//! 约定:
//! - 目录不存在 → 单个 Info 级 Passed(观测项,不阻断)。
//! - 目录存在 → 按文件名序执行前 8 个可执行文件或 `.sh`,每个产出一条结果:
//!   exit 0 → Passed(P1);非 0 → TaskFailed(P1),stderr 尾部进诊断。
//! - 环境变量:`ANYCODE_WORKSPACE`、`ANYCODE_TASK_ID`(来自 gate id 前缀)。
//! - extras:`project_gates_timeout_ms`(默认 60s)、`project_gates_fail_open=1`。

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anycode_core::{GateRequirement, GateSeverity, VerificationOutcome, VerificationResult};

const MAX_GATE_SCRIPTS: usize = 8;
const MAX_SCRIPT_OUTPUT: usize = 2_000;
const DEFAULT_TIMEOUT_MS: u64 = 60_000;

pub struct ProjectGatesConfig {
    pub timeout: Duration,
    pub fail_open: bool,
}

impl ProjectGatesConfig {
    pub fn from_extras(extras: &std::collections::HashMap<String, String>) -> Self {
        let timeout = extras
            .get("project_gates_timeout_ms")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .max(1_000);
        let fail_open = extras
            .get("project_gates_fail_open")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        Self {
            timeout: Duration::from_millis(timeout),
            fail_open,
        }
    }
}

/// 执行项目门禁脚本;`.anycode/gates/` 不存在时返回 None(调用方无需附结果)。
pub async fn run_project_gates(
    workspace: &Path,
    task_id: &str,
    config: &ProjectGatesConfig,
) -> Option<Vec<VerificationResult>> {
    let dir = workspace.join(".anycode").join("gates");
    if !dir.is_dir() {
        return None;
    }
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && (p.extension().and_then(|e| e.to_str()) == Some("sh") || is_executable(p))
        })
        .collect();
    scripts.sort();
    scripts.truncate(MAX_GATE_SCRIPTS);
    if scripts.is_empty() {
        return None;
    }

    let mut results = Vec::new();
    for script in scripts {
        let name = script
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "gate".into());
        let req = GateRequirement {
            id: format!("project.gate.{name}"),
            validator_id: "project.custom_gates".into(),
            artifact_ref: "workspace".into(),
            severity: GateSeverity::P1,
            timeout_ms: config.timeout.as_millis() as u64,
        };
        results.push(run_one(&req, &script, workspace, task_id, config).await);
    }
    Some(results)
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        false
    }
}

async fn run_one(
    req: &GateRequirement,
    script: &Path,
    workspace: &Path,
    task_id: &str,
    config: &ProjectGatesConfig,
) -> VerificationResult {
    let is_sh = script.extension().and_then(|e| e.to_str()) == Some("sh");
    let (program, args): (&str, Vec<String>) = if is_sh && !is_executable(script) {
        ("sh", vec![script.to_string_lossy().into_owned()])
    } else {
        (&*script.to_string_lossy(), Vec::new())
    };
    let child = tokio::process::Command::new(program)
        .args(&args)
        .current_dir(workspace)
        .env("ANYCODE_WORKSPACE", workspace)
        .env("ANYCODE_TASK_ID", task_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let child = match child {
        Ok(c) => c,
        Err(e) => return infra_outcome(req, config, "spawn_failed", &format!("{e}")),
    };
    match tokio::time::timeout(config.timeout, child.wait_with_output()).await {
        Err(_) => infra_outcome(
            req,
            config,
            "gate_timeout",
            &format!("exceeded {}ms", config.timeout.as_millis()),
        ),
        Ok(Err(e)) => infra_outcome(req, config, "wait_failed", &format!("{e}")),
        Ok(Ok(out)) if out.status.success() => VerificationResult {
            gate_id: req.id.clone(),
            validator_id: req.validator_id.clone(),
            validator_version: "1".into(),
            outcome: VerificationOutcome::Passed,
            severity: req.severity,
            artifact_path: None,
            artifact_hash: None,
            error_code: None,
            diagnostics: vec![],
            evidence_paths: vec![],
        },
        Ok(Ok(out)) => {
            let mut diag = String::from_utf8_lossy(&out.stderr).into_owned();
            if diag.trim().is_empty() {
                diag = String::from_utf8_lossy(&out.stdout).into_owned();
            }
            VerificationResult {
                gate_id: req.id.clone(),
                validator_id: req.validator_id.clone(),
                validator_version: "1".into(),
                outcome: VerificationOutcome::TaskFailed,
                severity: req.severity,
                artifact_path: None,
                artifact_hash: None,
                error_code: Some(format!("exit_{:?}", out.status.code())),
                diagnostics: vec![diag.chars().take(MAX_SCRIPT_OUTPUT).collect()],
                evidence_paths: vec![],
            }
        }
    }
}

/// 超时/无法执行:默认 fail-closed(TaskFailed);显式 fail_open 时 Info 级放行。
fn infra_outcome(
    req: &GateRequirement,
    config: &ProjectGatesConfig,
    code: &str,
    detail: &str,
) -> VerificationResult {
    let (outcome, severity) = if config.fail_open {
        (VerificationOutcome::Passed, GateSeverity::Info)
    } else {
        (VerificationOutcome::TaskFailed, req.severity)
    };
    VerificationResult {
        gate_id: req.id.clone(),
        validator_id: req.validator_id.clone(),
        validator_version: "1".into(),
        outcome,
        severity,
        artifact_path: None,
        artifact_hash: None,
        error_code: Some(code.into()),
        diagnostics: vec![detail.to_string()],
        evidence_paths: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg() -> ProjectGatesConfig {
        ProjectGatesConfig::from_extras(&HashMap::new())
    }

    #[tokio::test]
    async fn absent_dir_is_none() {
        let temp = tempfile::tempdir().unwrap();
        assert!(run_project_gates(temp.path(), "t1", &cfg()).await.is_none());
    }

    #[tokio::test]
    async fn passing_script_passes() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".anycode/gates");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("check.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        let results = run_project_gates(temp.path(), "t1", &cfg()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, VerificationOutcome::Passed);
    }

    #[tokio::test]
    async fn failing_script_is_task_failed_with_stderr() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".anycode/gates");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("lint.sh"),
            "#!/bin/sh\necho bad style >&2\nexit 1\n",
        )
        .unwrap();
        let results = run_project_gates(temp.path(), "t1", &cfg()).await.unwrap();
        assert_eq!(results[0].outcome, VerificationOutcome::TaskFailed);
        assert!(results[0].diagnostics[0].contains("bad style"));
    }

    #[tokio::test]
    async fn timeout_is_fail_closed_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".anycode/gates");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("slow.sh"), "#!/bin/sh\nsleep 5\n").unwrap();
        let mut extras = HashMap::new();
        extras.insert("project_gates_timeout_ms".to_string(), "1000".to_string());
        let results =
            run_project_gates(temp.path(), "t1", &ProjectGatesConfig::from_extras(&extras))
                .await
                .unwrap();
        assert_eq!(results[0].outcome, VerificationOutcome::TaskFailed);
        assert_eq!(results[0].error_code.as_deref(), Some("gate_timeout"));
    }

    #[tokio::test]
    async fn timeout_fail_open_is_info_pass() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join(".anycode/gates");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("slow.sh"), "#!/bin/sh\nsleep 5\n").unwrap();
        let mut extras = HashMap::new();
        extras.insert("project_gates_timeout_ms".to_string(), "1000".to_string());
        extras.insert("project_gates_fail_open".to_string(), "1".to_string());
        let results =
            run_project_gates(temp.path(), "t1", &ProjectGatesConfig::from_extras(&extras))
                .await
                .unwrap();
        assert_eq!(results[0].outcome, VerificationOutcome::Passed);
        assert_eq!(results[0].severity, GateSeverity::Info);
    }
}
