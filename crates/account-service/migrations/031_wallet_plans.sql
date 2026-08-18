-- 套餐 = 每月入账钱包 + 席位；用量只扣 credit_balance_fen。
-- 牌价：DeepSeek 官方非高峰 ×2（CNY / 1M）。
-- Free：一次性 ¥10 试用额度（已有钱包为 0 的 free 组织补发）。

UPDATE cloud_models
SET
  price_per_1m_input = 0.22,
  price_per_1m_output = 0.66,
  price_per_1m_input_cny = 3.2000,
  price_per_1m_output_cny = 9.6000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 5
WHERE id = 'deepseek-v4-flash';

UPDATE cloud_models
SET
  price_per_1m_input = 0.66,
  price_per_1m_output = 1.98,
  price_per_1m_input_cny = 9.6000,
  price_per_1m_output_cny = 28.5000,
  currency = 'CNY',
  min_plan = 'free',
  enabled = 1,
  sort_order = 6
WHERE id = 'deepseek-v4-pro';

UPDATE cloud_plans SET
  display_name = 'Free',
  description = '新用户赠送 ¥10 云端额度（DeepSeek 官方非高峰 ×2 扣减）',
  monthly_price_fen = 0,
  yearly_price_fen = 0,
  token_limit = 0,
  api_key_limit = 1,
  seat_limit = 1,
  quota_window_secs = 0,
  calls_per_window = 0,
  hosted_models_enabled = 1,
  promo_label = NULL,
  featured = 0,
  enabled = 1,
  sort_order = 10,
  updated_at = NOW(3)
WHERE id = 'free';

UPDATE cloud_plans SET
  display_name = 'Plus',
  description = '¥49/月，入账 ¥49 额度，10 席',
  monthly_price_fen = 4900,
  yearly_price_fen = 49000,
  token_limit = 0,
  api_key_limit = 3,
  seat_limit = 10,
  quota_window_secs = 0,
  calls_per_window = 0,
  hosted_models_enabled = 1,
  promo_label = '推荐',
  featured = 1,
  enabled = 1,
  sort_order = 20,
  updated_at = NOW(3)
WHERE id = 'cloud_5h';

UPDATE cloud_plans SET
  display_name = 'Pro',
  description = '¥199/月，入账 ¥199 额度，10 席',
  monthly_price_fen = 19900,
  yearly_price_fen = 199000,
  token_limit = 0,
  api_key_limit = 5,
  seat_limit = 10,
  quota_window_secs = 0,
  calls_per_window = 0,
  hosted_models_enabled = 1,
  promo_label = NULL,
  featured = 0,
  enabled = 1,
  sort_order = 30,
  updated_at = NOW(3)
WHERE id = 'pro';

UPDATE cloud_plans SET
  display_name = 'Team',
  description = '¥699/月，入账 ¥699 共享额度，10 席可加购',
  monthly_price_fen = 69900,
  yearly_price_fen = 699000,
  token_limit = 0,
  api_key_limit = 20,
  seat_limit = 10,
  quota_window_secs = 0,
  calls_per_window = 0,
  hosted_models_enabled = 1,
  promo_label = NULL,
  featured = 0,
  enabled = 1,
  sort_order = 40,
  updated_at = NOW(3)
WHERE id = 'team';

UPDATE entitlements e
JOIN subscriptions s ON s.organization_id = e.organization_id
SET e.credit_balance_fen = e.credit_balance_fen + 1000,
    e.hosted_models_enabled = 1,
    e.updated_at = NOW()
WHERE s.plan = 'free'
  AND e.credit_balance_fen = 0;
