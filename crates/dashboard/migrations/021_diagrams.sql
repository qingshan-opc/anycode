-- P1.8: persisted transcript diagrams for the /diagram/{id} full-address page.
-- id is a deterministic content hash (kind + source) so re-renders of the same
-- diagram resolve to the same shareable URL without duplicate rows.

CREATE TABLE IF NOT EXISTS diagrams (
  id TEXT PRIMARY KEY,
  session_id TEXT,
  kind TEXT NOT NULL,
  source TEXT NOT NULL,
  title TEXT,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_diagrams_session ON diagrams(session_id);
