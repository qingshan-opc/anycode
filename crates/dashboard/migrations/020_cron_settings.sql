-- P1.6: land cron run ledger + cron job mirror + app settings in SQLite.
-- Files (cron-runs.jsonl, orchestration.json, config.json) stay the writer-side
-- SSOT for the dual-read window; P2.6 flips priority and drops file writes.

CREATE TABLE IF NOT EXISTS cron_runs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  job_id TEXT NOT NULL,
  session_id TEXT NOT NULL DEFAULT '',
  fired_at TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  UNIQUE(job_id, session_id, fired_at, status)
);
CREATE INDEX IF NOT EXISTS idx_cron_runs_fired ON cron_runs(fired_at);
CREATE INDEX IF NOT EXISTS idx_cron_runs_job ON cron_runs(job_id);

-- Read-optimized mirror of orchestration.json's crons array (refreshed on read).
CREATE TABLE IF NOT EXISTS cron_jobs (
  id TEXT PRIMARY KEY,
  schedule TEXT NOT NULL,
  command TEXT NOT NULL,
  name TEXT,
  enabled INTEGER NOT NULL DEFAULT 1,
  schedule_timezone TEXT,
  session_id TEXT,
  failure_destination TEXT,
  tool_profile TEXT,
  project_id TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Flattened top-level config.json keys (dual-read window mirror; file is SSOT).
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
