//! Shared async shell runners for Bash / PowerShell (timeout + optional background).

use anycode_core::{CoreError, DiskTaskOutput};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use uuid::Uuid;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MIN_TIMEOUT_MS: u64 = 1_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// Per-stream (stdout / stderr) in-memory capture cap for foreground shells.
/// Without a cap a runaway or malicious command (`yes`, log spam) can OOM the
/// host process, since output is captured into memory before returning.
/// Follows the truncation convention of `MAX_SKILL_OUTPUT_BYTES` (256KB) but
/// defaults to a larger 1MB per stream; override via env for tests / tuning.
pub const DEFAULT_STREAM_MAX_BYTES: usize = 1024 * 1024;

pub fn shell_stream_max_bytes() -> usize {
    std::env::var("ANYCODE_SHELL_STREAM_MAX_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_STREAM_MAX_BYTES)
}

pub fn clamp_timeout_ms(raw: u64) -> u64 {
    raw.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS)
}

pub fn tasks_disk() -> Option<DiskTaskOutput> {
    dirs::home_dir().map(|h| DiskTaskOutput::new(h.join(".anycode").join("tasks")))
}

pub fn nested_output_log_path(task_id: Uuid) -> Option<String> {
    tasks_disk().map(|d| d.output_path(task_id).to_string_lossy().into_owned())
}

#[derive(Debug)]
pub struct ShellCapture {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// Bytes discarded from the FRONT of stdout because it exceeded the
    /// per-stream cap (0 = not truncated). See `read_capped_tail`.
    pub stdout_dropped_bytes: u64,
    /// Same as above, for stderr.
    pub stderr_dropped_bytes: u64,
}

impl ShellCapture {
    pub fn stdout_truncated(&self) -> bool {
        self.stdout_dropped_bytes > 0
    }

    pub fn stderr_truncated(&self) -> bool {
        self.stderr_dropped_bytes > 0
    }
}

fn apply_cwd(cmd: &mut Command, wd: Option<&Path>) {
    if let Some(wd) = wd {
        cmd.current_dir(wd);
    }
}

#[cfg(unix)]
fn configure_process_group(cmd: &mut Command) {
    // Own process group so timeout/cancel can kill the whole tree (dev servers, etc.).
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_cmd: &mut Command) {}

fn kill_process_group(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // Negative PGID = signal entire group started with process_group(0).
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{pid}")])
            .status();
    }
    let _ = child.start_kill();
}

/// Read a child stdio stream into memory with a hard byte cap.
///
/// Keeps the TAIL (last `cap` bytes), dropping from the front, and reports how
/// many bytes were discarded. Tail-preserving is deliberate: downstream parses
/// the END of stdout (e.g. the last-line `ANYCODE_ARTIFACT:` marker), so a
/// head-truncation would silently break that contract; with tail-truncation the
/// marker survives and callers can detect data loss via the dropped-byte count
/// (surfaced as `*_truncated` / `*_dropped_bytes` in the tool result JSON).
async fn read_capped_tail(
    mut reader: impl tokio::io::AsyncRead + Unpin,
    cap: usize,
) -> (Vec<u8>, u64) {
    let mut buf: Vec<u8> = Vec::with_capacity(cap.min(64 * 1024));
    let mut dropped: u64 = 0;
    let mut chunk = [0u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > cap {
                    let excess = buf.len() - cap;
                    buf.drain(..excess);
                    dropped += excess as u64;
                }
            }
        }
    }
    (buf, dropped)
}

/// Run a program with wall-clock timeout; stdout/stderr captured into memory
/// (each capped at `max_stream_bytes`, tail-preserving — see `read_capped_tail`).
pub async fn run_foreground_capped(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: u64,
    max_stream_bytes: usize,
) -> Result<ShellCapture, CoreError> {
    let timeout = Duration::from_millis(clamp_timeout_ms(timeout_ms));
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    apply_cwd(&mut cmd, cwd);
    configure_process_group(&mut cmd);

    let mut child = cmd.spawn().map_err(CoreError::IoError)?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();

    let stdout_task = tokio::spawn(async move {
        match stdout_pipe.take() {
            Some(r) => read_capped_tail(r, max_stream_bytes).await,
            None => (Vec::new(), 0),
        }
    });
    let stderr_task = tokio::spawn(async move {
        match stderr_pipe.take() {
            Some(r) => read_capped_tail(r, max_stream_bytes).await,
            None => (Vec::new(), 0),
        }
    });

    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => {
            let (stdout, stdout_dropped) = stdout_task.await.unwrap_or_default();
            let (stderr, stderr_dropped) = stderr_task.await.unwrap_or_default();
            Ok(ShellCapture {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
                exit_code: status.code(),
                timed_out: false,
                stdout_dropped_bytes: stdout_dropped,
                stderr_dropped_bytes: stderr_dropped,
            })
        }
        Ok(Err(e)) => {
            stdout_task.abort();
            stderr_task.abort();
            Err(CoreError::IoError(e))
        }
        Err(_) => {
            kill_process_group(&mut child);
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            Ok(ShellCapture {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: true,
                stdout_dropped_bytes: 0,
                stderr_dropped_bytes: 0,
            })
        }
    }
}

/// Run a program with wall-clock timeout; stdout/stderr captured into memory,
/// each capped at `shell_stream_max_bytes()` (default 1MB per stream).
pub async fn run_foreground(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: u64,
) -> Result<ShellCapture, CoreError> {
    run_foreground_capped(program, args, cwd, timeout_ms, shell_stream_max_bytes()).await
}

pub struct BackgroundChild {
    pub log_path: PathBuf,
    pub child: tokio::process::Child,
}

/// Spawn a long-running shell with stdio appended to `~/.anycode/tasks/<id>/output.log`.
pub fn spawn_background_child(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    task_id: Uuid,
) -> Result<BackgroundChild, CoreError> {
    let disk = tasks_disk().ok_or_else(|| {
        CoreError::IoError(std::io::Error::other(
            "HOME unavailable for background shell log path",
        ))
    })?;
    let log_path = disk.ensure_initialized(task_id)?;
    let out_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(CoreError::IoError)?;
    let err_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(CoreError::IoError)?;

    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::from(out_file));
    cmd.stderr(Stdio::from(err_file));
    cmd.stdin(Stdio::null());
    apply_cwd(&mut cmd, cwd);
    configure_process_group(&mut cmd);

    let child = cmd.spawn().map_err(CoreError::IoError)?;
    Ok(BackgroundChild { log_path, child })
}

pub fn kill_background_child(child: &mut tokio::process::Child) {
    kill_process_group(child);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn small_output_is_not_truncated() {
        let cap = run_foreground_capped("echo", &["hello-stream"], None, 10_000, 1024)
            .await
            .expect("echo");
        assert!(!cap.timed_out);
        assert_eq!(cap.stdout.trim_end(), "hello-stream");
        assert_eq!(cap.stdout_dropped_bytes, 0);
        assert!(!cap.stdout_truncated());
        assert_eq!(cap.stderr_dropped_bytes, 0);
    }

    #[tokio::test]
    async fn oversized_stdout_is_capped_and_keeps_tail() {
        // 4KB of filler followed by a tail marker (mimics ANYCODE_ARTIFACT: on
        // the last line); cap at 1KB. The tail must survive truncation.
        let script = "head -c 4096 /dev/zero | tr '\\0' 'x'; echo; echo TAIL-MARKER";
        let cap = run_foreground_capped("bash", &["-c", script], None, 10_000, 1024)
            .await
            .expect("run");
        assert!(!cap.timed_out);
        assert!(cap.stdout_truncated(), "{:?}", cap);
        assert!(cap.stdout_dropped_bytes > 0);
        // Tail-preserving: the end of the stream (marker line) is retained.
        assert!(
            cap.stdout.trim_end().ends_with("TAIL-MARKER"),
            "tail marker must survive truncation: {:?}",
            cap.stdout
        );
        assert!(
            cap.stdout.len() <= 1024 + 8,
            "capped output length: {}",
            cap.stdout.len()
        );
    }

    #[tokio::test]
    async fn oversized_stderr_is_capped_independently() {
        let script = "head -c 4096 /dev/zero | tr '\\0' 'e' 1>&2; echo ERR-TAIL 1>&2; echo ok-out";
        let cap = run_foreground_capped("bash", &["-c", script], None, 10_000, 512)
            .await
            .expect("run");
        assert!(cap.stderr_truncated());
        assert!(cap.stderr.trim_end().ends_with("ERR-TAIL"));
        // stdout is small and must not be affected by the stderr cap.
        assert!(!cap.stdout_truncated());
        assert_eq!(cap.stdout.trim_end(), "ok-out");
    }
}
