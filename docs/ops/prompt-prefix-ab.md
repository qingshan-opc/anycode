# Prompt 前缀稳定化 A/B 手册(P1.5)

`ANYCODE_PROMPT_PREFIX_STABLE=1` 把易变内容(date / reply_language / session context)移到
`<!-- SYSTEM_PROMPT_DYNAMIC_BOUNDARY -->` 之后的尾块,前缀对 provider 侧 prompt cache 字节稳定。
**默认关闭**,待 A/B 数据证明 prefix-hit 收益后再默认开启。

## 观测链路(2026-08-13 起可用)

provider 用量里的 cache 命中已全链路透传:

1. provider 响应(zai `prompt_cache_hit_tokens` 等)→ `TokenUsage.cache_read_tokens`;
2. agent 写 `[llm_response_end] … cache_read_tokens=N cache_creation_tokens=N`;
3. dashboard `llm_usage.rs` 摄取进 `project_events.payload_json`(`$.cache_read_tokens`);
4. `/api/metrics/usage` 的 `by_model`/`by_project`/`by_day` 聚合输出 cache_read/cache_creation
   并按 `ANYCODE_DASHBOARD_CACHE_HIT_RATIO`(默认 0.1,DeepSeek 风格)折价计入 `estimated_cost_cny`。

## A/B 步骤

1. 基线:不加 env 跑 3–7 天真实使用,导出 `GET /api/metrics/usage?days=7`
   (`/api/metrics/usage/export` 可下 CSV),记录 `cache_read_tokens / input_tokens` 比值。
2. 实验:设 `ANYCODE_PROMPT_PREFIX_STABLE=1` 重启,跑同样时长与量级,再导一次。
3. 判定:实验组 cache_read 比值显著高于基线(前缀命中生效)且无回归投诉,则把
   `prefix_stable_mode()` 默认改为开;收益不显著则保持默认关并记录结论。

注意:DeepSeek `/responses` 无状态(见 memory responses-transport-landed),链式前缀命中
只在支持 prompt cache 的 provider 上有意义;A/B 分析时按 provider 分组看。
