//! Task 日志生命周期(P2.7):`~/.anycode/tasks/<uuid>/` 目录随任务堆积
//! (线上已上千个),按年龄两级处理——超过压缩阈值的把 output.log /
//! events.jsonl  gzip 为 .gz 并删除原文;超过删除阈值(且已压缩)的
//! 整个目录移除。由 scheduler 每日 03:00 UTC 驱动一次。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use tracing::warn;

const LOG_FILES: [&str; 2] = ["output.log", "events.jsonl"];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RetentionReport {
    pub dirs_scanned: usize,
    pub dirs_compressed: usize,
    pub dirs_deleted: usize,
    pub bytes_reclaimed: u64,
}

pub fn compress_after_duration() -> Duration {
    duration_env("ANYCODE_TASK_LOG_COMPRESS_DAYS", 30)
}

pub fn delete_after_duration() -> Duration {
    duration_env("ANYCODE_TASK_LOG_DELETE_DAYS", 180)
}

fn duration_env(key: &str, default_days: u64) -> Duration {
    let days = std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|d| *d > 0)
        .unwrap_or(default_days);
    Duration::from_secs(days * 86_400)
}

/// 目录年龄 = 内部文件最新 mtime(空目录按目录自身 mtime)。
fn dir_age(dir: &Path, now: SystemTime) -> Duration {
    let mut latest: Option<SystemTime> = None;
    if let Ok(entries) = fs::read_dir(dir) {
        for ent in entries.flatten() {
            if let Ok(meta) = ent.metadata() {
                if let Ok(mtime) = meta.modified() {
                    if latest.is_none_or(|l| mtime > l) {
                        latest = Some(mtime);
                    }
                }
            }
        }
    }
    if latest.is_none() {
        if let Ok(meta) = dir.metadata() {
            if let Ok(mtime) = meta.modified() {
                latest = Some(mtime);
            }
        }
    }
    latest
        .and_then(|l| now.duration_since(l).ok())
        .unwrap_or(Duration::ZERO)
}

fn gzip_file(src: &Path) -> Result<u64, std::io::Error> {
    let data = fs::read(src)?;
    let dst = src.with_extension(format!(
        "{}.gz",
        src.extension().and_then(|e| e.to_str()).unwrap_or("log")
    ));
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(&data)?;
    let compressed = enc.finish()?;
    fs::write(&dst, &compressed)?;
    fs::remove_file(src)?;
    Ok(data.len().saturating_sub(compressed.len()) as u64)
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(dir) {
        for ent in entries.flatten() {
            if let Ok(meta) = ent.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn has_gz(dir: &Path) -> bool {
    LOG_FILES
        .iter()
        .any(|f| dir.join(format!("{f}.gz")).is_file())
}

/// 扫描 tasks_root 下的任务目录,按年龄压缩/删除。逐个目录 best-effort:
/// 单个失败记 warn 继续,不中断整轮。
pub fn sweep_task_logs(
    tasks_root: &Path,
    now: SystemTime,
    compress_after: Duration,
    delete_after: Duration,
) -> RetentionReport {
    let mut report = RetentionReport::default();
    let Ok(entries) = fs::read_dir(tasks_root) else {
        return report;
    };
    for ent in entries.flatten() {
        let dir: PathBuf = ent.path();
        if !dir.is_dir() {
            continue;
        }
        report.dirs_scanned += 1;
        let age = dir_age(&dir, now);
        if has_gz(&dir) && age >= delete_after {
            let size = dir_size(&dir);
            match fs::remove_dir_all(&dir) {
                Ok(()) => {
                    report.dirs_deleted += 1;
                    report.bytes_reclaimed += size;
                }
                Err(e) => {
                    warn!(target: "anycode_scheduler", "task log delete {}: {e}", dir.display())
                }
            }
            continue;
        }
        if age >= compress_after {
            let mut reclaimed = 0u64;
            let mut ok = false;
            for name in LOG_FILES {
                let src = dir.join(name);
                if !src.is_file() {
                    continue;
                }
                match gzip_file(&src) {
                    Ok(r) => {
                        reclaimed += r;
                        ok = true;
                    }
                    Err(e) => {
                        warn!(target: "anycode_scheduler", "task log gzip {}: {e}", src.display())
                    }
                }
            }
            if ok {
                report.dirs_compressed += 1;
                report.bytes_reclaimed += reclaimed;
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task_dir(root: &Path, name: &str, with_events: bool) -> PathBuf {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        // 足够大且高重复,gzip 一定有明显收益。
        fs::write(dir.join("output.log"), "line one two three\n".repeat(200)).unwrap();
        if with_events {
            fs::write(dir.join("events.jsonl"), "{\"k\":1}\n".repeat(100)).unwrap();
        }
        dir
    }

    #[test]
    fn fresh_dirs_are_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_task_dir(tmp.path(), "t1", true);
        let report = sweep_task_logs(
            tmp.path(),
            SystemTime::now(),
            Duration::from_secs(86_400),
            Duration::from_secs(86_400 * 2),
        );
        assert_eq!(report.dirs_scanned, 1);
        assert_eq!(report.dirs_compressed, 0);
        assert_eq!(report.dirs_deleted, 0);
        assert!(dir.join("output.log").is_file());
    }

    #[test]
    fn aged_dirs_are_compressed_then_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_task_dir(tmp.path(), "t1", true);
        // 阈值为 0:立即压缩。
        let report = sweep_task_logs(
            tmp.path(),
            SystemTime::now(),
            Duration::ZERO,
            Duration::from_secs(86_400),
        );
        assert_eq!(report.dirs_compressed, 1);
        assert!(!dir.join("output.log").exists());
        assert!(dir.join("output.log.gz").is_file());
        assert!(dir.join("events.jsonl.gz").is_file());
        assert!(report.bytes_reclaimed > 0);
        // 第二轮:已压缩且超过删除阈值 → 整目录移除。
        let report2 = sweep_task_logs(
            tmp.path(),
            SystemTime::now(),
            Duration::ZERO,
            Duration::ZERO,
        );
        assert_eq!(report2.dirs_deleted, 1);
        assert!(!dir.exists());
    }

    #[test]
    fn gz_content_roundtrips() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = make_task_dir(tmp.path(), "t1", false);
        sweep_task_logs(tmp.path(), SystemTime::now(), Duration::ZERO, Duration::MAX);
        let gz = fs::read(dir.join("output.log.gz")).unwrap();
        let mut dec = flate2::read::GzDecoder::new(&gz[..]);
        let mut text = String::new();
        use std::io::Read;
        dec.read_to_string(&mut text).unwrap();
        assert_eq!(text, "line one two three\n".repeat(200));
    }
}
