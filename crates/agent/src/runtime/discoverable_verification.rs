//! Open-ended verification nudge: block hollow "please recompile" completions without tool evidence.
//!
//! Does not register language-specific validators — only tracks whether the agent wrote files
//! and ran discover/search/run tools before claiming done.
//!
//! P1.5 去关键词化:对代码类产物(.rs/.ts/.py/.html 等)不再依赖空洞短语命中——
//! 「写了代码 + 未跑与改动栈相关的验证命令 + 模型停下宣称完成」这一结构事实即触发
//! evidence repair;短语表仅作为文档类写法的兜底。验证相关性按栈判定:改了 `.rs`
//! 必须见过成功的 cargo/rustc 命令,改 `.ts` 必须见过 tsc/npm 系命令,依此类推。

use serde_json::Value;
use std::collections::BTreeSet;

const MAX_EVIDENCE_REPAIRS: u32 = 1;
const MAX_TRACKED_WRITTEN_PATHS: usize = 64;
const MAX_TRACKED_VERIFY_COMMANDS: usize = 32;
const MAX_TRACKED_FAILED_SUBTASKS: usize = 16;

const REPAIR_MESSAGE_ZH: &str = "你尚未用工具验证本次修改。请先：1) 读仓库 README/配置文件；2) 如不确定，用 WebSearch/WebFetch 查该技术栈的官方验证/编译方式；3) 用 Bash 实际执行并把输出作为证据；4) 验证通过后再声明完成。不要只让用户「重新编译」或「打开开发者工具看看」。";

const REPAIR_MESSAGE_EN: &str = "You have not verified this change with tools yet. Read repo docs/config, search official verify/build steps if needed, run them via Bash (or Browser when UI proof is required), and only then claim done. Do not ask the user to recompile or open an IDE as your only proof.";

/// P2.9 子任务结果门禁的返修消息:失败/部分的子任务结果不得作为完成证据。
const SUBTASK_REPAIR_MESSAGE_ZH: &str = "本轮有子任务以 failed/partial 结束（见上方工具结果）。其结果不得作为完成证据：请重跑该子任务、改用其他方式验证其产出，或在最终答复中明确告知用户该部分未完成。不要在子任务失败的情况下宣称整体完成。";

const SUBTASK_REPAIR_MESSAGE_EN: &str = "A subtask ended failed/partial this turn (see tool results above). Its output must not be cited as completion evidence: re-run it, verify its output another way, or explicitly tell the user that part is unfinished. Do not claim overall completion over a failed subtask.";

/// Tracks file writes and verification actions across an entire task/turn session.
#[derive(Debug, Default, Clone)]
pub struct SessionVerificationState {
    pub wrote_files: bool,
    pub ran_verification: bool,
    /// 写工具(file_path/notebook_path)落过的路径,供 family 反推与栈相关验证判定。
    pub written_paths: Vec<String>,
    /// 成功退出码的 Bash/PowerShell 命令原文,供栈相关验证判定。
    pub verify_commands: Vec<String>,
    /// 同步 Agent/Task 子任务结果中 status=failed/partial 的标识(P2.9)。
    pub failed_subtasks: Vec<String>,
    ran_browser_verification: bool,
}

impl SessionVerificationState {
    /// 记录一次工具调用的痕迹。`input` 为工具入参(取 file_path/command),
    /// `tool_text` 为面向模型的结果文本(判定成功/失败)。
    pub fn note_tool(&mut self, tool_name: &str, input: &Value, tool_text: &str) {
        if is_write_tool(tool_name) {
            self.wrote_files = true;
            if let Some(path) = write_tool_path(tool_name, input) {
                if self.written_paths.len() < MAX_TRACKED_WRITTEN_PATHS
                    && !self.written_paths.iter().any(|p| p == &path)
                {
                    self.written_paths.push(path);
                }
            }
        }
        if tool_name.starts_with("Browser") && verification_tool_succeeded(tool_name, tool_text) {
            self.ran_browser_verification = true;
        }
        if is_verification_tool(tool_name) && verification_tool_succeeded(tool_name, tool_text) {
            self.ran_verification = true;
            if matches!(tool_name, "Bash" | "PowerShell") {
                if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                    let cmd = cmd.trim();
                    if !cmd.is_empty() && self.verify_commands.len() < MAX_TRACKED_VERIFY_COMMANDS {
                        self.verify_commands.push(cmd.to_string());
                    }
                }
            }
        }
        if matches!(tool_name, "Agent" | "Task") {
            if let Some(failed) = parse_failed_subtask(tool_text) {
                if self.failed_subtasks.len() < MAX_TRACKED_FAILED_SUBTASKS
                    && !self.failed_subtasks.iter().any(|s| s == &failed)
                {
                    self.failed_subtasks.push(failed);
                }
            }
        }
    }

    /// 写过的文件扩展名集合(小写、无点),供 family 反推。
    pub fn written_extensions(&self) -> BTreeSet<String> {
        self.written_paths
            .iter()
            .filter_map(|p| {
                std::path::Path::new(p)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
            })
            .collect()
    }

    fn written_code_stacks(&self) -> BTreeSet<CodeStack> {
        self.written_extensions()
            .iter()
            .filter_map(|e| CodeStack::from_extension(e))
            .collect()
    }

    /// P1.5:验证是否与改动的栈相关。写了代码文件时,要求至少一个改动栈
    /// 见过对应的成功验证命令;未写代码文件时退化为旧的 ran_verification。
    pub fn ran_relevant_verification(&self) -> bool {
        let stacks = self.written_code_stacks();
        if stacks.is_empty() {
            return self.ran_verification;
        }
        stacks
            .iter()
            .any(|s| s.is_verified_by(&self.verify_commands, self.ran_browser_verification))
    }

    /// 是否写过需要工具验证的代码/页面类文件(结构性 evidence repair 触发面)。
    pub fn wrote_verifiable_files(&self) -> bool {
        !self.written_code_stacks().is_empty()
    }
}

/// 可从扩展名识别的验证相关栈。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodeStack {
    Rust,
    Node,
    Python,
    Web,
}

impl CodeStack {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "rs" => Some(Self::Rust),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::Node),
            "py" => Some(Self::Python),
            "html" | "htm" | "css" | "vue" | "svelte" => Some(Self::Web),
            _ => None,
        }
    }

    /// 该栈是否被任一成功命令/浏览器验证覆盖。
    fn is_verified_by(&self, commands: &[String], ran_browser: bool) -> bool {
        match self {
            Self::Rust => commands.iter().any(|c| {
                let c = c.to_ascii_lowercase();
                c.contains("cargo ") || c.starts_with("cargo") || c.contains("rustc")
            }),
            Self::Node => commands.iter().any(|c| {
                let c = c.to_ascii_lowercase();
                [
                    "tsc", "npm ", "npx ", "pnpm", "yarn", "node ", "bun ", "vitest", "jest",
                ]
                .iter()
                .any(|k| c.contains(k))
            }),
            Self::Python => commands.iter().any(|c| {
                let c = c.to_ascii_lowercase();
                c.contains("pytest") || c.contains("python") || c.contains("uv run")
            }),
            // 页面栈:浏览器实证或任意构建/检查命令均可。
            Self::Web => {
                ran_browser
                    || commands.iter().any(|c| {
                        let c = c.to_ascii_lowercase();
                        ["npm ", "npx ", "pnpm", "yarn", "bun ", "tsc", "open "]
                            .iter()
                            .any(|k| c.contains(k))
                    })
            }
        }
    }
}

fn write_tool_path(tool_name: &str, input: &Value) -> Option<String> {
    let key = match tool_name {
        "NotebookEdit" => "notebook_path",
        _ => "file_path",
    };
    input
        .get(key)
        .or_else(|| input.get("path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 解析同步子任务结果:`{"status":"failed"|"partial", ...}` 视为失败证据。
fn parse_failed_subtask(tool_text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(tool_text).ok()?;
    let status = v.get("status").and_then(|s| s.as_str())?;
    if !matches!(status, "failed" | "partial") {
        return None;
    }
    let label = v
        .get("description")
        .or_else(|| v.get("nested_task_id"))
        .or_else(|| v.get("task_id"))
        .and_then(|s| s.as_str())
        .unwrap_or(status);
    Some(format!("{label}({status})"))
}

pub fn is_write_tool(name: &str) -> bool {
    matches!(name, "FileWrite" | "Edit" | "NotebookEdit")
}

pub fn is_verification_tool(name: &str) -> bool {
    matches!(name, "Bash" | "WebSearch" | "WebFetch")
        || name.starts_with("Browser")
        || name == "PowerShell"
}

pub fn verification_tool_succeeded(tool_name: &str, tool_text: &str) -> bool {
    if tool_name == "Bash" || tool_name == "PowerShell" {
        return bash_exit_success(tool_text);
    }
    if tool_name == "WebSearch" || tool_name == "WebFetch" {
        let lower = tool_text.to_ascii_lowercase();
        return !lower.contains("error")
            && !lower.contains("failed")
            && !tool_text.trim().is_empty();
    }
    if tool_name.starts_with("Browser") {
        return !tool_text.trim().is_empty();
    }
    false
}

fn bash_exit_success(tool_text: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(tool_text) {
        if let Some(code) = v.get("exit_code").and_then(|c| c.as_i64()) {
            return code == 0;
        }
    }
    let lower = tool_text.to_ascii_lowercase();
    if lower.contains("command failed") {
        return false;
    }
    if let Some(code) = parse_lean_exit_code(tool_text) {
        return code == 0;
    }
    if lower.contains("\"exit_code\":") {
        return lower.contains("\"exit_code\":0") || lower.contains("\"exit_code\": 0");
    }
    !lower.contains("exit_code=1") && !tool_text.trim().is_empty()
}

fn parse_lean_exit_code(tool_text: &str) -> Option<i64> {
    tool_text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("exit_code=")
            .and_then(|rest| rest.trim().parse().ok())
    })
}

pub fn hollow_completion_phrase(text: &str) -> bool {
    let t = text.to_lowercase();
    let zh = text;
    [
        "重新编译",
        "开发者工具",
        "编译即可",
        "已全部修正",
        "全部问题已修复",
        "已修好",
        "已修复",
        "recompile",
        "re-compile",
        "open the ide",
        "open wechat",
        "developer tools",
        "try it yourself",
        "please compile",
        "should work now",
        "all fixed",
        "all issues fixed",
    ]
    .iter()
    .any(|p| t.contains(p) || zh.contains(p))
}

/// Returns repair message when agent wrote files, claims done without verification, and budget remains.
///
/// P1.5:代码类写法不再要求命中空洞短语——写了可验证文件且未跑栈相关验证即返修;
/// 非代码写法(md/txt 等)保持短语触发,避免误伤纯文档任务。
pub fn maybe_evidence_repair(
    state: &SessionVerificationState,
    assistant_text: &str,
    evidence_repairs_used: u32,
) -> Option<String> {
    if evidence_repairs_used >= MAX_EVIDENCE_REPAIRS {
        return None;
    }
    if !state.wrote_files {
        return None;
    }
    if state.ran_relevant_verification() {
        return None;
    }
    let text = assistant_text.trim();
    if text.is_empty() {
        return None;
    }
    if !state.wrote_verifiable_files() && !hollow_completion_phrase(text) {
        return None;
    }
    Some(if text.chars().any(|c| c as u32 >= 0x4E00) {
        REPAIR_MESSAGE_ZH.to_string()
    } else {
        REPAIR_MESSAGE_EN.to_string()
    })
}

/// P2.9:存在 failed/partial 子任务且尚未返修过时,注入「不得作为完成证据」返修。
pub fn maybe_subtask_repair(
    state: &SessionVerificationState,
    assistant_text: &str,
    subtask_repairs_used: u32,
) -> Option<String> {
    if subtask_repairs_used >= 1 || state.failed_subtasks.is_empty() {
        return None;
    }
    let zh = assistant_text.chars().any(|c| c as u32 >= 0x4E00);
    let base = if zh {
        SUBTASK_REPAIR_MESSAGE_ZH
    } else {
        SUBTASK_REPAIR_MESSAGE_EN
    };
    Some(format!(
        "{base}\n失败子任务: {}",
        state.failed_subtasks.join(", ")
    ))
}

/// P2.8 逃逸度量:写了可验证文件、始终未跑栈相关验证、且 evidence repair 预算
/// 已耗尽——守卫将放行,这是一次「应拦未拦」。
pub fn verification_escape(state: &SessionVerificationState, evidence_repairs_used: u32) -> bool {
    evidence_repairs_used >= MAX_EVIDENCE_REPAIRS
        && state.wrote_verifiable_files()
        && !state.ran_relevant_verification()
}

/// Parse a successful Bash command for verify_recipe memory.
pub fn try_verify_recipe_from_bash(command: &str, tool_text: &str, cwd: &str) -> Option<String> {
    if !bash_exit_success(tool_text) {
        return None;
    }
    let cmd = command.trim();
    if cmd.is_empty() || cmd.len() > 500 {
        return None;
    }
    let stack = infer_stack_hint(cmd, cwd);
    Some(format!("verify_recipe: {stack} → {cmd} @ {cwd}"))
}

fn infer_stack_hint(command: &str, cwd: &str) -> String {
    let c = command.to_ascii_lowercase();
    let p = cwd.to_ascii_lowercase();
    if c.contains("docker compose") || c.contains("docker-compose") {
        return "docker-compose".into();
    }
    if p.contains("miniprogram") || c.contains("cli preview") || c.contains("miniprogram-ci") {
        return "wechat-miniprogram".into();
    }
    if c.contains("cargo ") {
        return "rust".into();
    }
    if c.contains("npm ") || c.contains("pnpm ") || c.contains("yarn ") {
        return "node".into();
    }
    if c.contains("pytest") || c.contains("python ") {
        return "python".into();
    }
    "shell".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash_ok(cmd: &str) -> (Value, String) {
        (
            serde_json::json!({"command": cmd}),
            r#"{"exit_code":0,"stdout":"ok"}"#.to_string(),
        )
    }

    fn write_input(path: &str) -> Value {
        serde_json::json!({"file_path": path, "content": "x"})
    }

    #[test]
    fn detects_hollow_phrases() {
        assert!(hollow_completion_phrase("已全部修正。重新编译即可。"));
        assert!(!hollow_completion_phrase("cargo test passed with 12 tests"));
    }

    #[test]
    fn nudge_when_wrote_without_verify() {
        let mut s = SessionVerificationState::default();
        s.wrote_files = true;
        assert!(maybe_evidence_repair(&s, "重新编译即可", 0).is_some());
        assert!(maybe_evidence_repair(&s, "重新编译即可", 1).is_none());
    }

    #[test]
    fn no_nudge_when_verified() {
        let mut s = SessionVerificationState::default();
        s.wrote_files = true;
        s.ran_verification = true;
        assert!(maybe_evidence_repair(&s, "重新编译即可", 0).is_none());
    }

    #[test]
    fn bash_json_exit_zero_counts() {
        assert!(verification_tool_succeeded(
            "Bash",
            r#"{"exit_code":0,"stdout":"ok"}"#
        ));
        assert!(!verification_tool_succeeded(
            "Bash",
            r#"{"exit_code":1,"stderr":"fail"}"#
        ));
        assert!(verification_tool_succeeded("Bash", "exit_code=0"));
        assert!(verification_tool_succeeded("Bash", "hi\nexit_code=0"));
        assert!(!verification_tool_succeeded("Bash", ""));
        assert!(!verification_tool_succeeded("Bash", "exit_code=1"));
    }

    #[test]
    fn verify_recipe_from_docker() {
        let r = try_verify_recipe_from_bash(
            "docker compose config",
            r#"{"exit_code":0}"#,
            "/tmp/pindou",
        );
        assert!(r.unwrap().contains("docker compose config"));
    }

    #[test]
    fn code_write_without_phrase_still_nudges() {
        // P1.5: 结构性触发——写了 .rs 且无任何 cargo 验证,不依赖空洞短语。
        let mut s = SessionVerificationState::default();
        s.note_tool("FileWrite", &write_input("/w/src/main.rs"), "ok");
        assert!(maybe_evidence_repair(&s, "功能已经实现完成了", 0).is_some());
    }

    #[test]
    fn rust_write_requires_cargo_verification() {
        let mut s = SessionVerificationState::default();
        s.note_tool("FileWrite", &write_input("/w/src/main.rs"), "ok");
        // ls 成功不算 rust 验证。
        let (input, out) = bash_ok("ls -la");
        s.note_tool("Bash", &input, &out);
        assert!(s.ran_verification);
        assert!(!s.ran_relevant_verification());
        assert!(maybe_evidence_repair(&s, "done", 0).is_some());
        // cargo check 成功才算。
        let (input, out) = bash_ok("cargo check");
        s.note_tool("Bash", &input, &out);
        assert!(s.ran_relevant_verification());
        assert!(maybe_evidence_repair(&s, "done", 0).is_none());
    }

    #[test]
    fn doc_write_still_needs_phrase() {
        // 非代码写法保持短语触发,避免误伤纯文档任务。
        let mut s = SessionVerificationState::default();
        s.note_tool("FileWrite", &write_input("/w/notes.md"), "ok");
        assert!(maybe_evidence_repair(&s, "已经写好了总结", 0).is_none());
        assert!(maybe_evidence_repair(&s, "已全部修正,重新编译即可", 0).is_some());
    }

    #[test]
    fn web_write_verified_by_browser() {
        let mut s = SessionVerificationState::default();
        s.note_tool("FileWrite", &write_input("/w/index.html"), "ok");
        assert!(maybe_evidence_repair(&s, "页面完成了", 0).is_some());
        s.note_tool("BrowserNavigate", &Value::Null, "navigated ok");
        assert!(s.ran_relevant_verification());
        assert!(maybe_evidence_repair(&s, "页面完成了", 0).is_none());
    }

    #[test]
    fn failed_subtask_tracked_and_repaired_once() {
        let mut s = SessionVerificationState::default();
        s.note_tool(
            "Agent",
            &Value::Null,
            r#"{"status":"failed","description":"research api","error":"boom"}"#,
        );
        assert_eq!(s.failed_subtasks.len(), 1);
        let msg = maybe_subtask_repair(&s, "done", 0).expect("repair");
        assert!(msg.contains("research api"));
        assert!(maybe_subtask_repair(&s, "done", 1).is_none());
        // completed 状态不追踪。
        let mut s2 = SessionVerificationState::default();
        s2.note_tool("Agent", &Value::Null, r#"{"status":"completed"}"#);
        assert!(s2.failed_subtasks.is_empty());
    }

    #[test]
    fn escape_flag_only_after_budget_exhausted() {
        let mut s = SessionVerificationState::default();
        s.note_tool("FileWrite", &write_input("/w/a.py"), "ok");
        assert!(!verification_escape(&s, 0));
        assert!(verification_escape(&s, 1));
        let (input, out) = bash_ok("pytest -q");
        s.note_tool("Bash", &input, &out);
        assert!(!verification_escape(&s, 1));
    }
}
