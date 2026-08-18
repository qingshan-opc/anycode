-- Cloud conversations pinned to a home device (remote command plane).

CREATE TABLE IF NOT EXISTS cloud_conversations (
  id VARCHAR(64) NOT NULL PRIMARY KEY,
  user_id VARCHAR(64) NOT NULL,
  home_device_id VARCHAR(64) NOT NULL,
  local_session_id VARCHAR(64) NULL,
  title VARCHAR(512) NOT NULL DEFAULT '',
  project_name VARCHAR(255) NULL,
  status VARCHAR(32) NOT NULL DEFAULT 'idle',
  last_event_seq BIGINT NOT NULL DEFAULT 0,
  created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
  updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),
  KEY idx_cloud_conv_user_device (user_id, home_device_id, updated_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS cloud_turn_events (
  id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,
  conversation_id VARCHAR(64) NOT NULL,
  seq BIGINT NOT NULL,
  kind VARCHAR(64) NOT NULL,
  payload_json MEDIUMTEXT NOT NULL,
  created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
  UNIQUE KEY uk_cloud_evt (conversation_id, seq),
  KEY idx_cloud_evt_conv (conversation_id, seq),
  CONSTRAINT fk_cloud_evt_conv FOREIGN KEY (conversation_id) REFERENCES cloud_conversations(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
