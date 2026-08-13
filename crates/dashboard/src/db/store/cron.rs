//! P1.6: cron ledger + cron job mirror in SQLite (dual-read window).
//!
//! The channel-bridge scheduler still writes `cron-runs.jsonl` and
//! `orchestration.json` — it has no dashboard DB handle. The dashboard is the
//! read side, so the mirror ingests here: `cron_runs` appends new jsonl lines
//! (offset tracked in kv_store), `cron_jobs` mirrors the file verbatim on
//! refresh. P2.6 moves the writers and drops the files.

use super::*;
use crate::cron_ledger::{CronJobRecord, CronRunRecord};

const KV_NS: &str = "cron";
const KV_OFFSET_KEY: &str = "runs_ingest_offset";

impl DashboardDb {
    /// Append new jsonl lines into `cron_runs`. Idempotent: the UNIQUE key
    /// dedupes re-ingests and the kv offset bounds the scan; a truncated or
    /// rotated file resets the offset and re-ingests (deduped by the UNIQUE).
    pub async fn cron_runs_ingest_jsonl(&self, path: &Path) -> Result<usize> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(0);
        };
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let offset: i64 = self.kv_get(KV_NS, KV_OFFSET_KEY).await?.unwrap_or(0);
        let mut start = usize::try_from(offset).unwrap_or(0);
        if start > lines.len() {
            start = 0; // rotated/truncated — re-ingest, UNIQUE dedupes
        }
        let mut ingested = 0usize;
        for line in &lines[start..] {
            let raw: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let job_id = raw.get("job_id").and_then(|v| v.as_str()).unwrap_or("");
            if job_id.is_empty() {
                continue;
            }
            let affected = sqlx::query(
                r#"INSERT OR IGNORE INTO cron_runs (job_id, session_id, fired_at, status, detail)
                   VALUES (?, ?, ?, ?, ?)"#,
            )
            .bind(job_id)
            .bind(raw.get("session_id").and_then(|v| v.as_str()).unwrap_or(""))
            .bind(raw.get("fired_at").and_then(|v| v.as_str()).unwrap_or(""))
            .bind(raw.get("status").and_then(|v| v.as_str()).unwrap_or(""))
            .bind(raw.get("detail").and_then(|v| v.as_str()).unwrap_or(""))
            .execute(&self.pool)
            .await?
            .rows_affected();
            ingested += usize::try_from(affected).unwrap_or(0);
        }
        self.kv_set(KV_NS, KV_OFFSET_KEY, &lines.len()).await?;
        Ok(ingested)
    }

    pub async fn cron_runs_list(
        &self,
        limit: i64,
        job_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<CronRunRecord>> {
        let rows = sqlx::query(
            r#"SELECT id, job_id, session_id, fired_at, status, detail
               FROM cron_runs
               WHERE (?1 IS NULL OR job_id = ?1)
                 AND (?2 IS NULL OR session_id = ?2)
               ORDER BY id DESC
               LIMIT ?3"#,
        )
        .bind(job_id)
        .bind(session_id)
        .bind(limit.max(1))
        .fetch_all(&self.pool)
        .await?;
        let mut out: Vec<CronRunRecord> = rows
            .into_iter()
            .map(|r| CronRunRecord {
                line_no: usize::try_from(r.get::<i64, _>("id")).unwrap_or(0),
                job_id: r.get("job_id"),
                session_id: r.get("session_id"),
                fired_at: r.get("fired_at"),
                status: r.get("status"),
                detail: r.get("detail"),
            })
            .collect();
        out.reverse(); // chronological, like the file reader
        Ok(out)
    }

    /// Mirror the orchestration file's crons verbatim: upsert every row and
    /// drop mirror rows the file no longer has (file is SSOT this window).
    pub async fn cron_jobs_mirror(&self, jobs: &[CronJobRecord]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for job in jobs {
            sqlx::query(
                r#"INSERT INTO cron_jobs
                     (id, schedule, command, name, enabled, schedule_timezone, session_id, failure_destination, tool_profile, project_id, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
                   ON CONFLICT(id) DO UPDATE SET
                     schedule = excluded.schedule,
                     command = excluded.command,
                     name = excluded.name,
                     enabled = excluded.enabled,
                     schedule_timezone = excluded.schedule_timezone,
                     session_id = excluded.session_id,
                     failure_destination = excluded.failure_destination,
                     tool_profile = excluded.tool_profile,
                     project_id = excluded.project_id,
                     updated_at = datetime('now')"#,
            )
            .bind(&job.id)
            .bind(&job.schedule)
            .bind(&job.command)
            .bind(&job.name)
            .bind(i64::from(job.enabled))
            .bind(&job.schedule_timezone)
            .bind(&job.session_id)
            .bind(&job.failure_destination)
            .bind(&job.tool_profile)
            .bind(&job.project_id)
            .execute(&mut *tx)
            .await?;
        }
        if jobs.is_empty() {
            sqlx::query("DELETE FROM cron_jobs")
                .execute(&mut *tx)
                .await?;
        } else {
            let placeholders = vec!["?"; jobs.len()].join(", ");
            let sql = format!("DELETE FROM cron_jobs WHERE id NOT IN ({placeholders})");
            let mut q = sqlx::query(&sql);
            for job in jobs {
                q = q.bind(&job.id);
            }
            q.execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn cron_jobs_list(&self) -> Result<Vec<CronJobRecord>> {
        let rows = sqlx::query(
            r#"SELECT id, schedule, command, name, enabled, schedule_timezone, session_id, failure_destination, tool_profile, project_id
               FROM cron_jobs ORDER BY id"#,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| CronJobRecord {
                id: r.get("id"),
                schedule: r.get("schedule"),
                command: r.get("command"),
                name: r.get("name"),
                enabled: r.get::<i64, _>("enabled") != 0,
                schedule_timezone: r.get("schedule_timezone"),
                session_id: r.get("session_id"),
                failure_destination: r.get("failure_destination"),
                tool_profile: r.get("tool_profile"),
                project_id: r.get("project_id"),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cron_runs_ingest_is_incremental_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();
        let log = dir.path().join("cron-runs.jsonl");
        std::fs::write(
            &log,
            "{\"job_id\":\"j1\",\"session_id\":\"s1\",\"fired_at\":\"2026-08-01T00:00:00Z\",\"status\":\"ok\",\"detail\":\"d1\"}\n",
        )
        .unwrap();
        assert_eq!(db.cron_runs_ingest_jsonl(&log).await.unwrap(), 1);
        // Re-ingest same file: offset advanced, nothing new.
        assert_eq!(db.cron_runs_ingest_jsonl(&log).await.unwrap(), 0);
        // Append a line: only the new one lands.
        std::fs::write(
            &log,
            "{\"job_id\":\"j1\",\"session_id\":\"s1\",\"fired_at\":\"2026-08-01T00:00:00Z\",\"status\":\"ok\",\"detail\":\"d1\"}\n{\"job_id\":\"j2\",\"session_id\":\"\",\"fired_at\":\"2026-08-02T00:00:00Z\",\"status\":\"failed\",\"detail\":\"boom\"}\n",
        )
        .unwrap();
        assert_eq!(db.cron_runs_ingest_jsonl(&log).await.unwrap(), 1);
        let all = db.cron_runs_list(10, None, None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].job_id, "j1");
        assert_eq!(all[1].status, "failed");
        let j1 = db.cron_runs_list(10, Some("j1"), None).await.unwrap();
        assert_eq!(j1.len(), 1);
    }

    #[tokio::test]
    async fn cron_jobs_mirror_matches_file_exactly() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::DashboardDb::open(dir.path().join("t.db"))
            .await
            .unwrap();
        let job = |id: &str| CronJobRecord {
            id: id.into(),
            schedule: "0 9 * * *".into(),
            command: "run".into(),
            name: None,
            enabled: true,
            schedule_timezone: None,
            session_id: None,
            failure_destination: None,
            tool_profile: None,
            project_id: None,
        };
        db.cron_jobs_mirror(&[job("a"), job("b")]).await.unwrap();
        assert_eq!(db.cron_jobs_list().await.unwrap().len(), 2);
        // File drops "b" → mirror follows.
        db.cron_jobs_mirror(&[job("a")]).await.unwrap();
        let jobs = db.cron_jobs_list().await.unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, "a");
    }
}
