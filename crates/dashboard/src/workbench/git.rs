use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;
use std::process::Output;

#[derive(Debug, Clone, Serialize)]
pub struct GitStatusSummary {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub insertions: u64,
    pub deletions: u64,
    pub changed_files: u32,
    pub ahead: u32,
    pub behind: u32,
    pub has_upstream: bool,
    pub has_changes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GitChangeKind {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    TypeChanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitFileChange {
    pub path: String,
    /// Original path for renames (otherwise equal to `path`).
    pub old_path: String,
    pub kind: GitChangeKind,
    pub staged: bool,
    /// Character-style status code from `git status --porcelain` (e.g. "M", "A").
    pub status: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GitFileDiff {
    pub path: String,
    pub kind: GitChangeKind,
    pub diff: String,
    pub insertions: u32,
    pub deletions: u32,
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<Output> {
    std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("spawn git {}", args.join(" ")))
}

fn git_ok(output: &Output) -> bool {
    output.status.success()
}

fn git_stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Stdout without the outer trim — porcelain output is fixed-column (`XY path`)
/// and a leading space in the first line is significant.
fn git_stdout_raw(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn git_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

pub fn is_git_repo(root: &Path) -> bool {
    if root.join(".git").exists() {
        return true;
    }
    run_git(root, &["rev-parse", "--is-inside-work-tree"])
        .ok()
        .filter(|o| git_ok(o) && git_stdout(o) == "true")
        .is_some()
}

/// Parse `git diff --shortstat` style summary fragments.
pub fn parse_shortstat_line(line: &str) -> (u32, u64, u64) {
    let mut files = 0u32;
    let mut insertions = 0u64;
    let mut deletions = 0u64;
    for part in line.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut tokens = part.split_whitespace();
        let Some(n) = tokens.next().and_then(|t| t.parse::<u64>().ok()) else {
            continue;
        };
        let word = tokens.next().unwrap_or("");
        if word.starts_with("file") {
            files = n.min(u32::MAX as u64) as u32;
        } else if word.starts_with("insertion") {
            insertions = n;
        } else if word.starts_with("deletion") {
            deletions = n;
        }
    }
    (files, insertions, deletions)
}

fn merge_shortstat(a: &str, b: &str) -> (u32, u64, u64) {
    let (f1, i1, d1) = parse_shortstat_line(a);
    let (f2, i2, d2) = parse_shortstat_line(b);
    (f1.saturating_add(f2), i1 + i2, d1 + d2)
}

pub fn git_status(root: &Path) -> Result<GitStatusSummary> {
    if !is_git_repo(root) {
        return Ok(GitStatusSummary {
            is_repo: false,
            branch: None,
            insertions: 0,
            deletions: 0,
            changed_files: 0,
            ahead: 0,
            behind: 0,
            has_upstream: false,
            has_changes: false,
        });
    }

    let branch = run_git(root, &["branch", "--show-current"])
        .ok()
        .filter(git_ok)
        .map(|o| git_stdout(&o))
        .filter(|s| !s.is_empty());

    let unstaged = run_git(root, &["diff", "--shortstat"])?;
    let staged = run_git(root, &["diff", "--cached", "--shortstat"])?;
    let unstaged_text = if git_ok(&unstaged) {
        git_stdout(&unstaged)
    } else {
        String::new()
    };
    let staged_text = if git_ok(&staged) {
        git_stdout(&staged)
    } else {
        String::new()
    };
    let (mut changed_files, insertions, deletions) = merge_shortstat(&unstaged_text, &staged_text);

    let porcelain = run_git(root, &["status", "--porcelain"])?;
    let mut untracked = 0u32;
    if git_ok(&porcelain) {
        for line in git_stdout(&porcelain).lines() {
            if line.starts_with("??") {
                untracked += 1;
            }
        }
    }
    changed_files = changed_files.saturating_add(untracked);

    let upstream = run_git(root, &["rev-parse", "--abbrev-ref", "@{upstream}"]);
    let has_upstream = upstream
        .as_ref()
        .ok()
        .filter(|o| git_ok(o))
        .map(|o| !git_stdout(o).is_empty())
        .unwrap_or(false);

    let (ahead, behind) = if has_upstream {
        let counts = run_git(
            root,
            &["rev-list", "--left-right", "--count", "@{upstream}...HEAD"],
        )?;
        if git_ok(&counts) {
            let stdout = git_stdout(&counts);
            let parts: Vec<&str> = stdout.split_whitespace().collect();
            let behind = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let ahead = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            (ahead, behind)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    let has_changes = changed_files > 0 || insertions > 0 || deletions > 0;

    Ok(GitStatusSummary {
        is_repo: true,
        branch,
        insertions,
        deletions,
        changed_files,
        ahead,
        behind,
        has_upstream,
        has_changes,
    })
}

pub fn git_commit_all(root: &Path, message: &str) -> Result<()> {
    let add = run_git(root, &["add", "-A"])?;
    if !git_ok(&add) {
        anyhow::bail!("git add failed: {}", git_stderr(&add));
    }
    let commit = run_git(root, &["commit", "-m", message])?;
    if !git_ok(&commit) {
        anyhow::bail!("git commit failed: {}", git_stderr(&commit));
    }
    Ok(())
}

pub fn git_push(root: &Path) -> Result<String> {
    let push = run_git(root, &["push"])?;
    if !git_ok(&push) {
        anyhow::bail!("git push failed: {}", git_stderr(&push));
    }
    Ok(git_stderr(&push))
}

#[derive(Debug, Clone, Serialize)]
pub struct GitBranchInfo {
    pub name: String,
    pub current: bool,
    pub remote: bool,
}

/// Local + remote branches. Remote entries are named `origin/foo` and skip the
/// `origin/HEAD` symref.
pub fn git_branches(root: &Path) -> Result<Vec<GitBranchInfo>> {
    if !is_git_repo(root) {
        return Ok(Vec::new());
    }
    let current = run_git(root, &["branch", "--show-current"])
        .ok()
        .filter(git_ok)
        .map(|o| git_stdout(&o))
        .unwrap_or_default();
    let mut branches = Vec::new();
    for (pattern, remote) in [("refs/heads", false), ("refs/remotes", true)] {
        let out = run_git(
            root,
            &["for-each-ref", "--format=%(refname:short)", pattern],
        )?;
        if !git_ok(&out) {
            anyhow::bail!("git for-each-ref failed: {}", git_stderr(&out));
        }
        for line in git_stdout(&out).lines() {
            let name = line.trim();
            if name.is_empty() || name.ends_with("/HEAD") {
                continue;
            }
            branches.push(GitBranchInfo {
                name: name.to_string(),
                current: !remote && !current.is_empty() && name == current,
                remote,
            });
        }
    }
    Ok(branches)
}

/// Why a checkout was refused, structured so the API can return 409 + files.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum GitCheckoutError {
    DirtyTree { files: Vec<String> },
    Failed { message: String },
}

impl std::fmt::Display for GitCheckoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirtyTree { files } => {
                write!(f, "working tree has {} uncommitted change(s)", files.len())
            }
            Self::Failed { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for GitCheckoutError {}

/// Files with uncommitted changes (staged + unstaged + untracked).
fn dirty_files(root: &Path) -> Vec<String> {
    run_git(root, &["status", "--porcelain"])
        .ok()
        .filter(git_ok)
        .map(|o| {
            git_stdout_raw(&o)
                .lines()
                .filter(|l| l.len() > 3)
                .map(|l| l[3..].trim().to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Switch branches. Refuses with `DirtyTree` when the working tree has
/// uncommitted changes unless `force` — switching branches mid agent edit
/// silently corrupts the review flow, so the default is fail-closed.
pub fn git_checkout(root: &Path, branch: &str, force: bool) -> Result<(), GitCheckoutError> {
    if !force {
        let dirty = dirty_files(root);
        if !dirty.is_empty() {
            return Err(GitCheckoutError::DirtyTree {
                files: dirty.into_iter().take(50).collect(),
            });
        }
    }
    let out = run_git(root, &["checkout", branch]).map_err(|e| GitCheckoutError::Failed {
        message: format!("spawn git checkout: {e}"),
    })?;
    if !git_ok(&out) {
        return Err(GitCheckoutError::Failed {
            message: format!("git checkout failed: {}", git_stderr(&out)),
        });
    }
    Ok(())
}

/// Create a new branch at HEAD, optionally checking it out (same dirty-tree
/// guard as `git_checkout` when switching).
pub fn git_create_branch(root: &Path, name: &str, checkout: bool) -> Result<(), GitCheckoutError> {
    if checkout {
        // checkout -b carries the working tree onto the new branch, which is
        // exactly what users expect here — no dirty guard needed.
        let out =
            run_git(root, &["checkout", "-b", name]).map_err(|e| GitCheckoutError::Failed {
                message: format!("spawn git checkout -b: {e}"),
            })?;
        if !git_ok(&out) {
            return Err(GitCheckoutError::Failed {
                message: format!("git checkout -b failed: {}", git_stderr(&out)),
            });
        }
        return Ok(());
    }
    let out = run_git(root, &["branch", name]).map_err(|e| GitCheckoutError::Failed {
        message: format!("spawn git branch: {e}"),
    })?;
    if !git_ok(&out) {
        return Err(GitCheckoutError::Failed {
            message: format!("git branch failed: {}", git_stderr(&out)),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct GitLogEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    /// ISO 8601 author date.
    pub date: String,
    pub subject: String,
}

pub fn git_log(root: &Path, limit: u32, offset: u32) -> Result<Vec<GitLogEntry>> {
    if !is_git_repo(root) {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 200);
    let out = run_git(
        root,
        &[
            "log",
            &format!("--max-count={limit}"),
            &format!("--skip={offset}"),
            "--format=%H%x1f%h%x1f%an%x1f%aI%x1f%s",
        ],
    )?;
    if !git_ok(&out) {
        // Unborn HEAD (no commits yet) exits 128 — treat as empty history.
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for line in git_stdout(&out).lines() {
        let mut parts = line.splitn(5, '\x1f');
        let (Some(hash), Some(short_hash), Some(author), Some(date), Some(subject)) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            continue;
        };
        entries.push(GitLogEntry {
            hash: hash.to_string(),
            short_hash: short_hash.to_string(),
            author: author.to_string(),
            date: date.to_string(),
            subject: subject.to_string(),
        });
    }
    Ok(entries)
}

/// Unified diff of one commit (`git show`), truncated server-side.
pub fn git_commit_diff(root: &Path, hash: &str) -> Result<GitFileDiff> {
    const MAX_DIFF_BYTES: usize = 200 * 1024;
    let out = run_git(root, &["show", "--format=", "--patch", hash])?;
    if !git_ok(&out) {
        anyhow::bail!("git show failed: {}", git_stderr(&out));
    }
    let mut diff = String::from_utf8_lossy(&out.stdout).to_string();
    if diff.len() > MAX_DIFF_BYTES {
        diff.truncate(MAX_DIFF_BYTES);
        diff.push_str("\n… (diff truncated)\n");
    }
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    for line in diff.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            insertions += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            deletions += 1;
        }
    }
    Ok(GitFileDiff {
        path: hash.to_string(),
        kind: GitChangeKind::Modified,
        diff,
        insertions,
        deletions,
    })
}

/// Parse one `git status --porcelain` line into a file change.
/// Format: `XY path` (with possible ` -> ` for renames).
fn parse_porcelain_line(line: &str) -> Option<GitFileChange> {
    let bytes = line.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let x = bytes[0] as char;
    let y = bytes[1] as char;
    let path_part = &line[3..];

    let (kind, path, old_path) = if x == 'R' || y == 'R' {
        let (old, new) = path_part.split_once(" -> ")?;
        (
            GitChangeKind::Renamed,
            new.trim().to_string(),
            old.trim().to_string(),
        )
    } else if x == '?' {
        (
            GitChangeKind::Untracked,
            path_part.to_string(),
            String::new(),
        )
    } else {
        let kind = match (x, y) {
            ('A', _) => GitChangeKind::Added,
            ('D', _) | (_, 'D') => GitChangeKind::Deleted,
            ('T', _) | (_, 'T') => GitChangeKind::TypeChanged,
            _ => GitChangeKind::Modified,
        };
        (kind, path_part.to_string(), String::new())
    };

    Some(GitFileChange {
        path,
        old_path,
        kind,
        staged: x != '?' && x != ' ',
        status: format!("{}{}", x, y),
        insertions: 0,
        deletions: 0,
    })
}

/// List changed files in the working tree, like an IDE source-control view.
/// Groups staged + unstaged changes and reports per-file +/- counts.
pub fn git_changes(root: &Path) -> Result<Vec<GitFileChange>> {
    if !is_git_repo(root) {
        return Ok(Vec::new());
    }
    let porcelain = run_git(root, &["status", "--porcelain"])?;
    if !git_ok(&porcelain) {
        anyhow::bail!("git status failed: {}", git_stderr(&porcelain));
    }
    let mut changes: Vec<GitFileChange> = git_stdout_raw(&porcelain)
        .lines()
        .filter_map(parse_porcelain_line)
        .collect();

    // Per-file numstat: `git diff --numstat` (unstaged) + `git diff --cached --numstat` (staged).
    for args in [
        &["diff", "--numstat"][..],
        &["diff", "--cached", "--numstat"][..],
    ] {
        let out = run_git(root, args)?;
        if !git_ok(&out) {
            continue;
        }
        for line in git_stdout(&out).lines() {
            let mut parts = line.splitn(3, '\t');
            let (Some(add), Some(del), Some(path)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let path = path.trim();
            let add = add.trim().parse::<u32>().unwrap_or(0);
            let del = del.trim().parse::<u32>().unwrap_or(0);
            if let Some(c) = changes.iter_mut().find(|c| c.path == path) {
                c.insertions += add;
                c.deletions += del;
            }
        }
    }

    Ok(changes)
}

/// Produce the unified diff text for a single changed file (staged + unstaged
/// combined). Untracked files render as an all-additions diff.
pub fn git_file_diff(root: &Path, path: &str, kind: GitChangeKind) -> Result<GitFileDiff> {
    let mut insertions = 0u32;
    let mut deletions = 0u32;
    let mut diff = String::new();

    if kind == GitChangeKind::Untracked {
        // No git diff exists for untracked files; show the full file as added.
        let full = match std::fs::read_to_string(root.join(path)) {
            Ok(s) => s,
            Err(e) => anyhow::bail!("read untracked file {}: {}", path, e),
        };
        let line_count = full.lines().count() as u32;
        insertions = line_count;
        diff.push_str(&format!("diff --git a/{path} b/{path}\n"));
        diff.push_str("new file mode 100644\n");
        diff.push_str("--- /dev/null\n");
        diff.push_str(&format!("+++ b/{path}\n"));
        diff.push_str(&format!("@@ -0,0 +1,{} @@\n", line_count.max(1)));
        for line in full.lines() {
            diff.push_str(&format!("+{}\n", line));
        }
        if full.is_empty() {
            diff.push_str("+\n");
        }
    } else {
        for args in [
            &["diff", "--", path][..],
            &["diff", "--cached", "--", path][..],
        ] {
            let out = run_git(root, args)?;
            if git_ok(&out) {
                diff.push_str(&String::from_utf8_lossy(&out.stdout));
            }
        }
        // Tally +/- from the combined diff text.
        for line in diff.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                insertions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                deletions += 1;
            }
        }
    }

    Ok(GitFileDiff {
        path: path.to_string(),
        kind,
        diff,
        insertions,
        deletions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_shortstat_examples() {
        let (f, i, d) =
            parse_shortstat_line("3 files changed, 2154 insertions(+), 233 deletions(-)");
        assert_eq!(f, 3);
        assert_eq!(i, 2154);
        assert_eq!(d, 233);
        let (f2, i2, d2) = parse_shortstat_line("1 file changed, 10 insertions(+)");
        assert_eq!(f2, 1);
        assert_eq!(i2, 10);
        assert_eq!(d2, 0);
    }

    #[test]
    fn merge_shortstat_sums() {
        let (f, i, d) = merge_shortstat(
            "2 files changed, 10 insertions(+), 1 deletion(-)",
            "1 file changed, 5 insertions(+), 2 deletions(-)",
        );
        assert_eq!(f, 3);
        assert_eq!(i, 15);
        assert_eq!(d, 3);
    }

    #[test]
    fn parse_porcelain_basic() {
        let m = parse_porcelain_line(" M src/main.rs").unwrap();
        assert_eq!(m.kind, GitChangeKind::Modified);
        assert_eq!(m.path, "src/main.rs");
        assert!(!m.staged);
        assert_eq!(m.status, " M");

        let a = parse_porcelain_line("A  new.rs").unwrap();
        assert_eq!(a.kind, GitChangeKind::Added);
        assert_eq!(a.path, "new.rs");
        assert!(a.staged);

        let u = parse_porcelain_line("?? untracked.txt").unwrap();
        assert_eq!(u.kind, GitChangeKind::Untracked);
        assert_eq!(u.path, "untracked.txt");
    }

    #[test]
    fn parse_porcelain_rename() {
        let r = parse_porcelain_line("R  old.rs -> new.rs").unwrap();
        assert_eq!(r.kind, GitChangeKind::Renamed);
        assert_eq!(r.old_path, "old.rs");
        assert_eq!(r.path, "new.rs");
        assert!(r.staged);
    }

    #[test]
    fn parse_porcelain_deleted_staged_vs_unstaged() {
        // 已暂存删除：X='D'，路径即被删文件。
        let staged = parse_porcelain_line("D  gone.rs").unwrap();
        assert_eq!(staged.kind, GitChangeKind::Deleted);
        assert_eq!(staged.path, "gone.rs");
        assert!(staged.staged);
        assert_eq!(staged.status, "D ");

        // 未暂存删除：X=' '、Y='D' → 转换规则 `(_, 'D')` 也归为 Deleted。
        let unstaged = parse_porcelain_line(" D gone.rs").unwrap();
        assert_eq!(unstaged.kind, GitChangeKind::Deleted);
        assert_eq!(unstaged.path, "gone.rs");
        assert!(!unstaged.staged);
        assert_eq!(unstaged.status, " D");
    }

    #[test]
    fn parse_porcelain_type_changed_both_columns() {
        let staged = parse_porcelain_line("T  link").unwrap();
        assert_eq!(staged.kind, GitChangeKind::TypeChanged);
        assert!(staged.staged);

        let unstaged = parse_porcelain_line(" T link").unwrap();
        assert_eq!(unstaged.kind, GitChangeKind::TypeChanged);
        assert!(!unstaged.staged);
    }

    #[test]
    fn parse_porcelain_combined_xy() {
        // AM：工作区中新增后又修改 → X='A'，规则 `('A', _)` 归为 Added 且已暂存。
        let am = parse_porcelain_line("AM staged_and_modified.rs").unwrap();
        assert_eq!(am.kind, GitChangeKind::Added);
        assert_eq!(am.path, "staged_and_modified.rs");
        assert!(am.staged);

        // MM：暂存后又未暂存修改 → 落 default 归为 Modified。
        let mm = parse_porcelain_line("MM both.rs").unwrap();
        assert_eq!(mm.kind, GitChangeKind::Modified);
        assert!(mm.staged);
    }

    #[test]
    fn parse_porcelain_short_line_returns_none() {
        assert!(parse_porcelain_line("").is_none());
        assert!(parse_porcelain_line(" M").is_none()); // 不足 4 字节（缺路径）
    }

    /// Build a temp repo with one commit on the default branch.
    fn temp_repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {:?} failed", args);
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "Tester"]);
        std::fs::write(root.join("a.txt"), "hello\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-m", "initial"]);
        dir
    }

    #[test]
    fn branches_log_checkout_roundtrip() {
        let dir = temp_repo_with_commit();
        let root = dir.path();

        let branches = git_branches(root).unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].name, "main");
        assert!(branches[0].current);
        assert!(!branches[0].remote);

        let log = git_log(root, 10, 0).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].subject, "initial");
        assert_eq!(log[0].author, "Tester");

        // Create + switch to a feature branch.
        git_create_branch(root, "feature-x", true).unwrap();
        let branches = git_branches(root).unwrap();
        assert_eq!(branches.len(), 2);
        let current = branches.iter().find(|b| b.current).unwrap();
        assert_eq!(current.name, "feature-x");

        // Dirty tree blocks checkout without force…
        std::fs::write(root.join("a.txt"), "dirty\n").unwrap();
        let err = git_checkout(root, "main", false).unwrap_err();
        match err {
            GitCheckoutError::DirtyTree { files } => {
                assert!(files.iter().any(|f| f == "a.txt"));
            }
            other => panic!("expected DirtyTree, got {other}"),
        }
        // …and force allows it.
        git_checkout(root, "main", true).unwrap();
        let branches = git_branches(root).unwrap();
        assert!(branches.iter().find(|b| b.current).unwrap().name == "main");

        // Commit diff renders the patch.
        let hash = git_log(root, 1, 0).unwrap()[0].hash.clone();
        let diff = git_commit_diff(root, &hash).unwrap();
        assert!(diff.diff.contains("+hello"));
        assert_eq!(diff.insertions, 1);
    }

    #[test]
    fn log_empty_on_unborn_head() {
        let dir = tempfile::tempdir().unwrap();
        let out = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(git_log(dir.path(), 10, 0).unwrap().is_empty());
    }
}
