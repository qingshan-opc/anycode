-- Hosted catalog: DeepSeek V4 Pro must be present and selectable.
-- 018 temporarily set enabled=0; 025 re-enabled it, but INSERT IGNORE in 013
-- can miss the row, and CAST AS DOUBLE hid the catalog behind HTTP 500.

INSERT IGNORE INTO cloud_models (
  id, provider_id, display_name, upstream_model, context_window,
  price_per_1m_input, price_per_1m_output,
  price_per_1m_input_cny, price_per_1m_output_cny, currency,
  min_plan, enabled, sort_order
) VALUES
  (
    'deepseek-v4-flash', 'deepseek', 'DeepSeek V4 Flash', 'deepseek-v4-flash', 1000000,
    0.28, 0.56,
    2.0000, 4.0000, 'CNY',
    'free', 1, 5
  ),
  (
    'deepseek-v4-pro', 'deepseek', 'DeepSeek V4 Pro', 'deepseek-v4-pro', 1000000,
    0.87, 1.74,
    6.0000, 12.0000, 'CNY',
    'free', 1, 6
  );

UPDATE cloud_models
SET
  provider_id = 'deepseek',
  display_name = 'DeepSeek V4 Flash',
  upstream_model = 'deepseek-v4-flash',
  context_window = 1000000,
  price_per_1m_input_cny = 2.0000,
  price_per_1m_output_cny = 4.0000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 5
WHERE id = 'deepseek-v4-flash';

UPDATE cloud_models
SET
  provider_id = 'deepseek',
  display_name = 'DeepSeek V4 Pro',
  upstream_model = 'deepseek-v4-pro',
  context_window = 1000000,
  price_per_1m_input_cny = 6.0000,
  price_per_1m_output_cny = 12.0000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 6
WHERE id = 'deepseek-v4-pro';

UPDATE cloud_plans
SET hosted_models_enabled = 1
WHERE id IN ('free', 'pro', 'team', 'cloud_5h');

-- No JOIN: entitlements/subscriptions/cloud_plans may differ in
-- utf8mb4_general_ci vs utf8mb4_unicode_ci (MySQL 1267). All current
-- hosted plans enable models; set the flag directly.
UPDATE entitlements SET hosted_models_enabled = 1;
