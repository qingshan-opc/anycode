# anyCode 维护者路线图（SSOT）

本文档是 **执行层 backlog 的单一事实来源**：最近交付、下一迭代、后续池、已拍板与待决策。  
产品级 **MVP 边界、工具 P0–P8 矩阵、验收场景** 仍以文档站为准（避免在本文件重复整张矩阵）：

| 语言 | 源码路径 |
|------|-----------|
| English | [`https://anycode.work/docs/guide/roadmap.md`](../docs/user/guide/roadmap) |
| 中文 | [`https://anycode.work/docs/zh/guide/roadmap.md`](../docs/user/zh/guide/roadmap) |

协作约定：**迭代任务与决策状态只改本文件（及 `docs/adr/`）**；不要在 `docs/user` 再维护一份相同的 now/next/later 列表。

在线浏览本文件：<https://github.com/qingjiuzys/anycode/blob/main/docs/roadmap.md>

---

## 1. 文档治理（落地规则）

1. **分工**  
   - **`docs/user/.../roadmap.md`**：产品叙事、MVP、工具阶段矩阵、MCP/LSP 提纲。  
   - **`docs/roadmap.md`（本文件）**：可执行的 now/next/later、已完成摘要、决策表。

2. **Next**  
   - 建议保持 **≤7** 条；溢出移到 **Later** 或拆成独立 GitHub issue。

3. **Later**  
   - 每 **1～2 个月**扫一次：长期无进展则写入 ADR（明确不做或合并主题），避免清单无限膨胀。

4. **待决策**  
   - 本文件只保留 **表格级摘要**；选项、取舍、后果写在 **`docs/adr/`**。

5. **最近已交付**  
   - 保留约 **两个版本窗口** 的摘要即可；更老的历史可查 `CHANGELOG.md` 或 git。

---

## 2. 最近已交付（摘要）

**窗口：0.41–0.42（Desktop 主产品线）**

- **Desktop CEF Workbench**：`anyCode.app` 内嵌 dashboard（进程内 ephemeral `127.0.0.1:0` + one-shot `dw_session`）；正式入口不再是终端 CLI / 第三方 IM。  
- **嵌套 Agent 时间线**：子 Agent 进度进会话瀑布；协作取消与 background task 诊断态。  
- **Skill Apps（ADR 020）**：`ui/surface.yaml` + VisualBrief；dock / conversation / project pin；`SkillAppPresent` / `Push` / `Read`；hello / ppt / video starter。  
- **记忆 DATA-02**：热层 **sled → sqlite**；WAL、turn/tool hooks、`promote_fragment_to_hot`。  
- **瀑布时间线 v1**：`thinking_delta` 升格为时间线旁白；工具簇 settled pill 用 activity recap；按 LLM turn 切思考块。  
- **MCP 治理壳**：`mcp.governance`（strict / allowlist / `max_calls_per_server`）+ Settings UI；拒绝/超配额写 `~/.anycode/audit/events.jsonl`（`mcp_denied` / `mcp_quota_exceeded`）。

**已移除 / 明确不做（勿再当 backlog）**

- 终端 **CLI REPL / TUI / `run`** 作为主入口（crate 已删）。  
- 第三方 IM：**微信 / Telegram / Discord** 通道与 OpenClaw 统一 ingress（含 G1 CDN、G12 Discord AskUserQuestion 文本回落）。  
- 云端 Portal **不**跑 Agent；本地 Desktop App 跑 Agent（纠正旧文案「0.3 不做网页端操作 Agent」——本地 Workbench 已是操作面）。

更早交付（Setup 记忆向导、Digital Workbench V1–V3、cron 调度器、OpenClaw 5.19 对标等）见 `CHANGELOG.md`。

---

## 3. 已完成（摘要表）

| 主题 | 状态（简） |
|------|------------|
| 子 Agent 真异步 **v1** | **`run_in_background`** + **`TaskOutput`** / **`TaskStop`**（进程内注册表；**`TaskStop`** 置协作式标志 + **`AbortHandle`** 兜底）。 |
| **嵌套协作取消 v2+v2.1** | 见 §2；**`cancelled`** → **`background_status: cancelled`**；HTTP / syscall 边界见 **`CHANGELOG`**。 |
| **AskUserQuestion** | Workbench 内 HITL（点选交给 Agent）；通道侧实现随 IM crate **已移除**。 |
| **LSP 一等配置** | **`config.json` `lsp`** + 文档；回退 **`ANYCODE_LSP_COMMAND`**。 |

**Issue [#3](https://github.com/qingjiuzys/anycode/issues/3)** 正文草稿仍见 [`issue-drafts/001-ask-user-question.md`](issue-drafts/001-ask-user-question.md)（通道卡片选题为非目标）。

---

## 3.5 anyCode **0.3** 迭代范围（2026-06 产品方向）

**原则**：0.3 聚焦 **云端网页账号控制台（Portal）** — 登录、订阅/用量/账单、API 与企业能力入口。Agent **执行**在本地 **Desktop Workbench** / `AgentRuntime`；**云端 Portal 不跑 Agent**、不做远程队列执行或云端 Agent 托管。对标参考见 [`openclaw-sync-brief-2026-05.md`](comparisons/openclaw-sync-brief-2026-05.md)、[`claude-reference-brief-2026-06.md`](comparisons/claude-reference-brief-2026-06.md)；技术 hardening 见 §4（**0.4**）。

| 包 | 主题 | 完成定义（简） |
|----|------|----------------|
| **0.3-A** | **Web Account Console** | 网页登录/登出、会话态、用户菜单与账号设置；loopback **`local_trusted`** 保留；非 loopback 需 token/密码 |
| **0.3-B** | **Subscription / Billing Shell** | 套餐管理、订阅状态、账单与发票入口 UI；可先本地/mock，**不绑真实支付** |
| **0.3-C** | **Usage & Entitlements** | 现有 token/cost 指标包装为「用量管理」；套餐限额/权益模型（quota、overage 提示） |
| **0.3-D** | **API / Developer Access** | API key 创建/撤销/轮换；开发者 token 状态与 scope 说明 |
| **0.3-E** | **Enterprise Admin** | 组织、成员、角色、审计日志、企业设置入口；SSO/OIDC **设计/占位**，不强制完整 IdP |

### 3.5.1 执行顺序（0.3）

1. **0.3-A** — 账号与会话（`/login`、`/api/auth/*`、Settings → Auth）。
2. **0.3-B + 0.3-C** — 套餐壳 + 用量页（可 mock 账单数据）。
3. **0.3-D** — API 管理（key 生命周期）。
4. **0.3-E** — 企业壳层（org/member/role/audit 入口）。

### 3.5.2 明确 Out of scope（0.3 不做）

- **云端 Portal 操作 Agent**：登录站 / 账单站不触发 run/goal、不托管工具审批；本地 **Desktop App** 才是 Agent 操作面。
- 远程队列执行、云端 Agent 控制台、OpenClaw Gateway/Codex 式托管。
- HTTP `anycode daemon`（[ADR 003](adr/018-http-daemon-deprecated.md)）。
- 第三方 IM 通道（已移除）。

### 3.5.3 云端产品纠偏（0.3+，[ADR 011](adr/011-cloud-account-platform.md)）

| 包 | 主题 | 完成定义（简） |
|----|------|----------------|
| **0.3-F** | **Cloud Portal** | 独立 Web 站点（`account-portal`），由 `anycode-account` serve；`/` 可访问 |
| **0.3-G** | **Device link** | 云端登录 → `anycode://` 跳转本地 App；`anycode auth login\|link` |
| **0.3-H** | **Stripe 订阅** | Checkout + webhook；Portal 账单页 |
| **0.3-I** | **Model gateway** | `anycode-model-gateway` 托管推理 + 计量；Portal 模型市场 |
| **0.3-J** | **混合模型客户端** | `anycode-cloud` provider；Settings 分区 BYOK / Cloud |

本地 `/login` 保留 loopback 开发；**C 端主路径**为云端 Portal + 设备关联。

### 3.5.4 原 harness 对标项（→ 0.4 / §4）

下列曾误标为 0.3 的项已移回 **§4 Epic A–G**（技术 hardening / **0.4**）：Eval/Release Gate、Tool Governance、MCP Doctor、Cron Ops、Terminal UX、Channel Reliability。见 [`planning/production-harness-hardening.md`](planning/production-harness-hardening.md)、[`planning/closure-plan-2026-06.md`](planning/closure-plan-2026-06.md)。

---

## 4. 生产级下一阶段（2026-05 起）

**2026-06 方向：** **0.3** 产品主线为网页账号控制台（§3.5）；Harness/eval/MCP/cron 等 runtime hardening 归入本节 Epic（**0.4**）。WorkBuddy Phase 1–3 已基本落地；剩余 Partial 按 **[closure-plan-2026-06.md](planning/closure-plan-2026-06.md)** 在 hardening 波次收完，**不阻塞** 0.3 账号/订阅壳层。

**方向切换**：OpenClaw 5.19 parity 的短线修补已基本完成；下一阶段按生产级能力推进，避免继续以 provider alias / 小单测数量作为主目标。每个 Epic 的完成定义必须包含：用户场景、失败场景、targeted tests、日志/诊断入口、文档/CHANGELOG、禁用或回滚边界。

| # | Epic | 主题 | 完成定义（简） |
|---|------|------|----------------|
| A | Eval / Release | **可重复评测与发布验收** | `anycode eval` 最小场景、mock LLM / fixture repo、release readiness 文档；CI 分层 |
| B | Security / Tools | **工具治理控制面** | `tool-calls.jsonl` 审计、WebFetch/MCP sanitizer 与 scanner、Bash env policy |
| C | Agent Runtime | **长任务与持久后台诊断** | overflow single-retry、compaction checkpoint metadata、background task state |
| D | MCP / LSP | **受控 MCP 与工具生态** | `doctor mcp` / `mcp status`、ADR 007 controlled reconnect、resource UX |
| E | Automation / Cron | **可审计自动化** | stable cron session、`cron runs` 查询、failure destination、per-job tool profile |
| F | Channels | **已关闭** | IM crate 已移除；勿再排期 WeChat / Discord / Telegram |
| G | Memory / Terminal / Ops | **上下文、终端与诊断** | evidence index、memory doctor、transcript 负载模型、error taxonomy、doctor 命令 |

**2026-05 已交付**：OpenClaw 对标简报、流式 REPL resize 不变量、DeepSeek `anyOf` schema 规范化、stream→chat fallback transcript、pipeline 向量 WARN、[`cron-runs.jsonl`](ops/cron-observability.md)、`CronCreate` 校验 + IANA 时区、WebFetch 私网/DNS/redirect 防护、provider kebab 别名、微信出站重试、cli_smoke 隔离。详见 [`openclaw-sync-brief-2026-05.md`](comparisons/openclaw-sync-brief-2026-05.md)。

### 4.1 执行顺序

1. **Epic A** 先行：评测 harness 会约束后续大改，避免只靠 `cargo test --workspace`。
2. **Epic B + D** 第二批：工具/MCP 是生产风险面，先做审计和诊断，再做 reconnect。
3. **Epic C** 第三批：overflow retry 与 durable state，不承诺一步到位恢复执行。
4. **Epic E** 第四批：cron stable session（**Epic F IM 已关闭**）。
5. **Epic G** 收束 release candidate：memory evidence / provenance、transcript 负载模型、doctor / release readiness。

---

## 4.9 Skill Apps（人机协同视觉宿主）— [ADR 020](adr/020-skill-apps.md)

| 切片 | 状态 | 说明 |
|------|------|------|
| 合约 `ui/surface.yaml` + VisualBrief | **落地** | `crates/tools/src/skills/surface.rs` |
| 三槽宿主 + hello fixture | **落地** | dock / conversation / project pin；`skills-starter/skill-app-hello` |
| Present / Push / Read + brief 锁定 | **落地** | `SkillAppPresent` / `SkillAppPush` / `SkillAppRead` |
| anycode-ppt 视觉工作台 | **落地** | `skills-starter/anycode-ppt/ui` |
| anycode-video 短视频工作台 | **落地** | `skills-starter/anycode-video`（html-video 模板 + Playwright MP4） |
| scaffold / vet / 文档 | **落地** | 本 ADR + user guide |

后续增强：多 Skill App 同时钉在 header、MCP Apps 共用同一宿主、Doc 专用 brief schema。

---

## 5. 后续（Later）

- **真正恢复执行的跨进程后台 Agent**：先完成 diagnostic state，再决定是否恢复执行。
- **memory-wiki 全栈 / 服务端梦境**：仍不做；**本地梦境整理 + Memory Center + E2EE 密文同步**已按 [ADR 014](adr/014-dream-memory-and-experience-packs.md) 落地。
- **Transcript 虚拟滚动（ADR 006）**：Workbench 已用 `@tanstack/react-virtual`；RFC 仍 Proposed — 补负载目标与估高跳/留白调参后再 Accepted。
- **会话 rewind（ADR 004）/ `/clear`（ADR 005）**：先统一语义，再改快照。
- **Webhook / TaskFlow / SQLite ledger**：除非 cron 使用场景明确，不复制 Gateway。
- **execute_turn / execute_task 双实现收敛**、桌面传输去 loopback（custom protocol / UDS）、M4 hidden-split 评测、LLM failover 多跳 — 见 [`planning/audit-2026-07-27.md`](planning/audit-2026-07-27.md)。
- ~~Telegram 工具进度 / IM 通道~~ — **已关闭**。

---

## 6. 已拍板

| 决策 | 记录 |
|------|------|
| **不提供 / 不恢复 HTTP `anycode daemon`** | [ADR 003](adr/018-http-daemon-deprecated.md) |
| **MCP stdio 长驻会话不自动重连** | 子进程退出 / EOF / 超时后由用户修正命令或重启 CLI；见 [`mcp-stdio-lifecycle.md`](ops/mcp-stdio-lifecycle.md) |

---

## 7. 待决策

| 主题 | 备注 | ADR / 下一步 |
|------|------|----------------|
| **MCP stdio 受控重连（实现）** | 政策已 **Accepted**（ADR 007）；**代码层自动重连**仍待 flag + 原子工具表更新后再开 | [ADR 007](adr/007-mcp-session-reconnect-policy.md) |
| ~~通道 AskUserQuestion 扩展~~ | **关闭**：IM 已移除；Workbench HITL 为准 | [ADR 008](adr/008-channel-ask-user-question-phasing.md) — 归档 |
| 会话 **rewind** / 撤销展示 | 与 `sessions` 快照格式兼容性 | [ADR 004](adr/004-session-rewind.md)（Proposed）— **暂缓**：无实现排期前保持 Proposed，改快照前必读。 |
| **`/clear` vs 纯文本 transcript 缓冲** | 是否需独立于 agent messages 的视口重置 | [ADR 005](adr/005-repl-clear-vs-transcript.md)（Proposed）— **暂缓**：流式 REPL 已有 `turn_transcript_anchor` / `stream_exit_dump_anchor`，产品缺口再开。 |
| **virtual scroll** | 见 §5 Later | [ADR 006](adr/006-transcript-virtual-scroll-rfc.md)（Proposed）— **暂缓**：与 [`term-smoothness-baseline.md`](ops/term-smoothness-baseline.md) 负载模型挂钩后再审。 |

---

## 8. 相关链接

- [`closure-plan-2026-06.md`](planning/closure-plan-2026-06.md) — 2026-06 套件收口（Wave 0–4、G1–G12、Exit Criteria）
- [`openclaw-sync-brief-2026-05.md`](comparisons/openclaw-sync-brief-2026-05.md) — OpenClaw 对标（含 2026-06-13 增量）
- [`claude-reference-brief-2026-06.md`](comparisons/claude-reference-brief-2026-06.md) — Claude TS / claude-code-rust 参考（技术项 → §4 / 0.4）
- [`workbench/digital-workbench-next-steps-zh.md`](workbench/digital-workbench-next-steps-zh.md) — 0.3 网页控制台规划入口
- [`architecture.md`](../architecture.md) — 维护者分层与流式/TUI 会话表  
- [`docs/README.md`](README.md) — ADR 索引与文档地图  
- 仓库：<https://github.com/qingjiuzys/anycode>
