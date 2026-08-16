-- Hide archived sessions from the sidebar without deleting history.
ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_sessions_archived_started
  ON sessions(archived, started_at DESC);
