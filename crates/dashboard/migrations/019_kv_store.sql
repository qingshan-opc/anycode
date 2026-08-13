-- 019: generic key-value landing zone for file→SQLite migration (roadmap P0.6).
-- Namespaced JSON blobs; first movers are runtime settings/identity previously
-- persisted as loose JSON files. Dual-read (kv first, file fallback) with
-- write-through during the transition window.
CREATE TABLE IF NOT EXISTS kv_store (
  namespace TEXT NOT NULL,
  key TEXT NOT NULL,
  value_json TEXT NOT NULL,
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (namespace, key)
);
