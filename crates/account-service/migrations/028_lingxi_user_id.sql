-- Link anyCode users to lingxi-accounts SSO identities (WeChat QR hop).

ALTER TABLE users ADD COLUMN lingxi_user_id VARCHAR(64) NULL;
CREATE UNIQUE INDEX uk_users_lingxi_user_id ON users (lingxi_user_id);
