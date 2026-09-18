# AnyCode × Pi Harness × 818cloud：源码调研与重构决策

## 交付与调研边界

本目录已经进入真实仓库集成流程，不再只是离线补丁。具体提交与已执行测试见 INTEGRATION_20260918.md。本报告记录基础版本的源码依据与设计决定，不声明整个产品已切换到新内核，也不声明已完成所有文件的安全审计。

锁定的调研基线：AnyCode `0411ea3a94fe6aa2d37326f9342cfc5d6a13f4aa`；818cloud `e8c9b49eafd12a69cf2007e59cd25b3d9b52683e`；Pi agent-loop.ts `7b4cfd6eb0fd490e3b54370ec9e9227b29717cb4`。部分大文件按核心行段读取，技能安装及治理目录的结构识别不等于完整实现审计。没有读取生产账户数据或将私有 818cloud 源码复制到本仓库。[S01–S14]

用户原文 grapl 按图工作流需求处理；当前明确实现原生 Graph DAG v1，不声称实现 Grapl 安全图平台、LangGraph Python API、循环状态图或其 checkpoint 兼容格式。新增适配器应放在独立边界，不能污染唯一执行内核。

## 1. 为什么借鉴 Pi，但不复制一个 TypeScript CLI

建议保留 Rust/Tokio、Tauri、内嵌 Dashboard、原生模型适配、工具注册和安全审批。Pi 值得借鉴的是小执行循环、内部消息与模型边界分离、事件、steering/follow-up、可组合宿主扩展，不是终端外壳。[S02,S13,S15]

新增内核独立 Rust 实现，没有内置 Pi subprocess，没有复制或打包 Pi TypeScript 源码。换语言会引入额外 IPC、分发和权限一致性工作，不是解决当前职责混杂的必要条件。原子工具暂保守串行；图和子代理在外层调度，不应以历史 Pi 的串行印象断言当前上游行为。

最终依赖方向：产品入口 → 可信宿主/身份/策略 → 单任务或图/子代理编排 → 唯一 Harness LLM-工具循环 → 原有安全工具链/设备后端。Graph 和 Subagent 可以组织多个任务，但不能各自再实现模型循环。

## 2. 必须保留的现有资产

`crates/bootstrap/src/runtime.rs::initialize_runtime` 是 composition root，装配模型、记忆、安全、profiles、技能白名单与媒体服务，不能另建一个绕过它的启动器。[S13]

`execute_task` 和 `execute_turn_from_messages` 已共享工具分发核，尚未共享全部循环生命周期。最终应变成薄入口，逐步将 compaction、memory、failover、language、artifact、delivery acceptance、session notifications 接入 Host 生命周期，而不是删掉这些功能。[S02,S03]

现有工具调用经过 security → gating → approval → execute → audit。新桥接继续调用 `execute_tool_call`，前面增加必须实现的 HarnessBoundary；没有从模型请求直接走裸 `Tool::execute` 的默认适配器。[S07]

Message metadata 包括 tool calls、reasoning、视觉输入、响应链信息。不能先压成 text 再声称兼容所有模型。新 Host 保留原生消息，要求流收到 Done，Failed 或断流的半截工具调用不执行。provider-private reasoning 不直接作为用户预览。[S08,S09]

Dashboard 已有 React/reactflow，新增画布复用既有依赖，没有引入第二套 UI 框架。[S14]

## 3. 已确认的缺陷与尚未复现的风险

### 图步骤 Partial 被当作成功

原 `graph_engine.rs` 同时接受 Success 与 Partial，然后 mark_passed。新图保留 Partial 独立状态并阻止后继；不能只改显示文字。旧 GraphEngine 尚未自动切换，本轮没有暗中改变所有旧工作流行为。[S04]

### required_gates 被自动置为 true

原成功分支对声明的 required_gates 写 true，没有在该分支调用可信 verifier。新图以独立 Gate 节点调用验证器，并要求证据摘要。模型说“已测试”不是测试结果；示例 verifier ID 不是已实现的真实测试服务。[S04]

### checkpoint 自动恢复与持久化失败

原路径按 workflow.name 找文件，存在就尝试恢复，坏 JSON 回退 fresh；定义与项目身份未绑定，持久化 helper 忽略写错误。新图 Start/Resume 分开，绑定 run/definition/scope/budget，坏文件硬失败，Running 恢复为 Uncertain 而非自动重放。文件 lease/原子替换仍不是多机数据库事务。[S04]

### 有拓扑分层不等于实际并行

检查到的旧 GraphEngine 在层内逐节点 await。新图使用 FuturesUnordered，但默认宿主 concurrency_key=exclusive；只有真实只读资源或独立 worktree 才可并发，JSON 中的 max_parallel 不授予并发写权限。[S04]

### 全局父工具限制与子代理深度

ToolServices 中的 `parent_task_tool_deny` 是共享槽位，`sub_agent_depth` 是全局原子计数。恢复 previous 对单线嵌套有帮助，但不足以证明并发兄弟隔离。这是已确认结构与待复现并发风险，不是宣称已复现线上越权。[S06]

新 RunContext 携带 root/parent/depth、scope、capabilities、共享预算和取消链。迁走旧共享服务前，不能开启共享工作区的并发写任务。Supervisor 不做无归属 detached spawn；容量不足快速返回而不形成递归等待死锁。

### 子代理身份和预算没有云端继承链

原 nested_task 新建 session、user_id=None，预算从环境变量读取。它不能直接当作多租户身份隔离或父任务总预算控制。新子任务共享根 BudgetPool，能力只能收窄；请求发出后用量未知，保守计入预留，而不是记零。[S05]

原后台 state.json 标记 diagnostic_only，不是可恢复执行事实。新基础库也不宣称已经实现跨机调度服务。[S06]

## 4. 本轮落地的代码边界

- harness-core：唯一循环、运行上下文、预算预留/结算、取消、操作绑定审批、事件日志、会话树、steering/follow-up。
- harness-extensions：DAG/条件/合流/Human/verifier/checkpoint、结构化子代理调度、worktree、受限宿主进程、电脑 broker、技能目录。
- harness-host：原生 AnyCode 消息与 LLMClient 适配；图 Work 节点仍进入同一 Kernel。
- harness-cloud818：真实 SSO v2 introspection 客户端、身份契约、ProductAcl 接口与用量事实。
- agent/harness_bridge：feature-gated 接缝，继续原有 SecurityLayer 链；现有聊天和 scheduler 默认不切换。
- dashboard-ui/features/harness：ReactFlow 组件、严格 JSON/checkpoint 验证，宿主未接线时不能伪装执行。

新模块编译、测试、Clippy 通过不等于上述所有产品路由已接通。入口切换顺序见 CURSOR_START.md。

## 5. 恢复、预算与副作用

结果未知不是一个普通失败。工具可能已写文件、点击或提交，随后网络/进程中断。新内核先写 intent，未确认结果保留 Uncertain，不能把自动重试当作安全恢复。

日志序号与哈希链用于发现损坏，不抵抗有管理员文件权限者重写整个文件。journal/checkpoint 必须放在宿主私有不可被代理修改的目录中；canonicalize 不是完整 OS 沙箱。

预算覆盖同一进程根任务与后代，保留实测超支而不是截断；未知已发请求保守收费。该实现不是分布式预算账本，更不是货币钱包。持久化 usage outbox、cache token、费用对账和多 worker fencing 是下一阶段。

## 6. 电脑与技能

ComputerBroker 要求已登记 device、独占 lease、当前 run/scope、最新 frame、目标窗口、具体动作参数和一次性审批。云账号登录不能替代本机操作授权。画面含敏感信息，默认不作为普通事件日志上传。

实际后端代码仅 opt-in X11；尚未验证真实显示环境，尚未实现 macOS Accessibility/ScreenCapture、Windows UIAutomation、Wayland。焦点检查不能消除所有竞态，必须使用专用桌面/执行容器。

Process helper 只面向可信宿主，绝对可执行文件白名单、无 shell 插值、清理环境、限制输出与时间。直接子进程 kill 不是进程树隔离；不得作为云租户任意 shell 沙箱。

Skills 采用 metadata-first 搜索再激活，显式宿主 allowlist、内容摘要与根路径检查。不执行安装钩子，不接受技能正文的自授权限。12 个 SKILL.md 是指令示例，不代表已实现 12 套执行引擎。现有安装、审核、项目白名单仍需正式接通。

## 7. 818cloud 原生集成

SSO v2 实际已有 authorize/token/introspect/revoke，采用 confidential client、精确回调、S256、一次性 code 和实时授权有效性验证。源码明确不把它宣传为完整 OIDC provider，不能凭空构造 JWT/JWKS 方案冒充接入。[S10,S11]

身份与授权分层：Accounts 用户 UUID → 组织成员 → 产品 tenant/external_tenant_id → AnyCode 项目 ACL → 本机设备和动作审批。企业 role 不自动授权所有项目，组织不等同产品租户。

AccountsClient 仅服务端持有产品 secret；桌面/Tauri/浏览器/技能中禁止嵌入。ProductAcl 没有默认放行实现，需查询实际用户/项目/租户映射。不得按邮箱自动合并旧用户。[S10–S12]

网页可复用 product_sso 网关。桌面配对、短期设备 token、keychain、撤销、BFF transport 标记为 PROPOSED；没有声称接口已存在。UsageReceipt 只提供用量事实，不扣钱包、不新建资金真源。本轮未修改 818cloud 仓库、生产存储、用户、订单或钱包。

## 8. 验收路线

M0 复验锁定提交和 full workspace；M1 只读真实模型 Host Pilot；M2 两入口生命周期迁移；M3 子代理接线/隔离；M4 图 API、UI、真实 verifier；M5 电脑和技能工具；M6 818cloud 真实身份与数据库；M7 持久化和部署强化；M8 灰度切换与回滚。

本轮已由 GitHub Actions 执行 Rust/前端验证，不再沿用历史离线报告的“没有 Rust 工具链所以未编译”作为当前结论。但用户 Cloud Mac 仍被会话层拒绝，不能用 CI runner 冒充用户 Mac，也不能把 scripted demo 当作真实模型/桌面/云端联调。

## 来源索引

以下路径均固定到上述调研基线。

S01 AnyCode Cargo.toml，blob b58a41a6a1c0b9a04624055d57d25ab7e732ddc4。
S02 AGENTS.md，blob 5cf7588ced081697ca59a0585e7489206bd07e9b。
S03 crates/agent/src/runtime/{mod.rs,agentic_loop.rs,agentic_turn.rs} 与 agent/src/lib.rs。
S04 crates/agent/src/graph_engine.rs，blob b0267172732127fbe9416e586ee1b1d423874a8e。
S05 runtime/nested_task.rs，blob 6811a328f63f1071a9c4ca9629bffae112218ecf。
S06 crates/tools/src/services.rs，blob db70c9dc68cd6be519a302b80330e5572073b541。
S07 tools/src/registry.rs、runtime/execute_tool.rs、runtime/tool_invocation.rs；调用管线 blob 4f6a43b8c71cda4a209e1c7c803e2f86c2bb6b6e。
S08 crates/core/src/{traits.rs,message.rs,lib.rs}。
S09 core/src/llm_types.rs，blob d710cb1012096fc8f8a932845a593c40204994a1。
S10 818cloud lingxi-accounts/src/product_sso.rs，blob 595cad646edf01159a9e7d298e115794968c8c50。
S11 818cloud lingxi-accounts/src/platform/sso.rs，blob 624da9b40b8d2fbd780ff2e15ada00c391cc0a39。
S12 818cloud README.md 与 Accounts/platform 目录结构；未读取生产数据。
S13 AnyCode crates/bootstrap/src/runtime.rs，blob 3325d3754fa7822e4f87e715b0d65c151ea413b4。
S14 crates/dashboard-ui/package.json，blob 6f12f14e14b6a34193c9bf576acb8ab8c70e1b38。
S15 Pi packages/agent/src/agent-loop.ts，earendil-works/pi@7b4cfd6eb0fd490e3b54370ec9e9227b29717cb4。
