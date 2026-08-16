-- Skill App bindings and persistent mini-app state (ADR 020).

CREATE TABLE IF NOT EXISTS project_skill_apps (
  project_id TEXT NOT NULL,
  skill_id TEXT NOT NULL,
  slot TEXT NOT NULL DEFAULT 'dock',
  enabled INTEGER NOT NULL DEFAULT 1,
  config_json TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (project_id, skill_id)
);
CREATE INDEX IF NOT EXISTS idx_project_skill_apps_project
  ON project_skill_apps(project_id);

CREATE TABLE IF NOT EXISTS skill_app_state (
  project_id TEXT NOT NULL,
  skill_id TEXT NOT NULL,
  state_json TEXT NOT NULL DEFAULT '{}',
  brief_json TEXT,
  updated_at TEXT NOT NULL DEFAULT (datetime('now')),
  PRIMARY KEY (project_id, skill_id)
);
