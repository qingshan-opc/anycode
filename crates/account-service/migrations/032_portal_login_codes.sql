-- Browser-bound one-use portal handoff. No long-lived bearer is stored in URLs.
CREATE TABLE IF NOT EXISTS portal_login_codes (
  code_hash VARCHAR(128) NOT NULL PRIMARY KEY,
  user_id VARCHAR(64) NOT NULL,
  browser_hash VARCHAR(128) NOT NULL,
  expires_at DATETIME(3) NOT NULL,
  KEY idx_portal_login_codes_expiry (expires_at),
  CONSTRAINT fk_portal_login_codes_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
