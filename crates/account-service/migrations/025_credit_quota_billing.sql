-- 额度制计费（类 Cursor）：充值获得额度，按 token × 单价（DeepSeek 官方价 2 倍）
-- 从 credit_balance_fen 扣减，额度永久有效、不过期。
-- 云端托管目录同时收敛为仅 DeepSeek V4 Flash / Pro（ Agnes 等全部下线）。

ALTER TABLE entitlements ADD COLUMN credit_balance_fen BIGINT NOT NULL DEFAULT 0;

UPDATE cloud_models
SET enabled = 0
WHERE id NOT IN ('deepseek-v4-flash', 'deepseek-v4-pro');

-- DeepSeek V4 Flash：官方 1.0/2.0 CNY per 1M → 2 倍 = 2.0/4.0。
UPDATE cloud_models
SET
  price_per_1m_input = 0.28,
  price_per_1m_output = 0.56,
  price_per_1m_input_cny = 2.0000,
  price_per_1m_output_cny = 4.0000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 5
WHERE id = 'deepseek-v4-flash';

-- DeepSeek V4 Pro：官方 3.0/6.0 CNY per 1M → 2 倍 = 6.0/12.0。
UPDATE cloud_models
SET
  price_per_1m_input = 0.87,
  price_per_1m_output = 1.74,
  price_per_1m_input_cny = 6.0000,
  price_per_1m_output_cny = 12.0000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 6
WHERE id = 'deepseek-v4-pro';
