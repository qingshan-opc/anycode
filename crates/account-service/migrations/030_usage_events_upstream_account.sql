-- 006 added this column, but runtime migrate() starts at 009, so production
-- never got it. INSERT ... upstream_account_id then failed and billing was dropped.
ALTER TABLE usage_events
  ADD COLUMN upstream_account_id VARCHAR(64) NULL;
