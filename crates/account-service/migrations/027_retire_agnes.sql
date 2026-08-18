-- Retire Agnes from hosted catalog. Cloud is DeepSeek-only.
-- Do not touch upstream_accounts: that table is optional and may be absent.

UPDATE cloud_models
SET enabled = 0
WHERE provider_id = 'agnes' OR id LIKE 'agnes%';
