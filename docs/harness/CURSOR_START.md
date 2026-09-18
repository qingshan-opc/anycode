# Cursor 接续开发入口：从已集成源码继续，不重复应用旧补丁

## 你拿到的是什么

AnyCode 的 Pi-inspired Rust Harness 第一阶段基础实现，不是已经替换全产品的成品。源码已导入集成分支，四处 Cargo/feature 接线和 Cargo.lock 由真实构建环境生成。先读本目录 INTEGRATION_20260918.md 的最新提交及测试状态，再读 RESEARCH.zh-CN.md、ARCHITECTURE.md 和原仓库 AGENTS.md。不要将历史报告里的“未编译/未推送”误认为本轮状态，也不要把本轮编译通过等同产品已切换。

源码基线 AnyCode：0411ea3a94fe6aa2d37326f9342cfc5d6a13f4aa。调研基线 818cloud：e8c9b49eafd12a69cf2007e59cd25b3d9b52683e。不要 reset 用户分支，不重复 apply v1/v1.1 patch。

## M0：复验实际提交

```sh
python3 tools/harness/verify.py --mode node
cargo fmt --all -- --check
cargo test --locked -p anycode-harness-core -p anycode-harness-extensions -p anycode-harness-cloud818 -p anycode-harness-host --all-targets
cargo clippy --locked -p anycode-harness-core -p anycode-harness-extensions -p anycode-harness-cloud818 -p anycode-harness-host --all-targets -- -D warnings
cargo check --locked -p anycode-agent --features harness-v1 --all-targets
cargo clippy --locked --workspace --all-targets
cargo test --locked --workspace
cd crates/dashboard-ui && npm ci && npm test && npm run build
```

使用已提交的 Cargo.lock；禁止为了添加 path dependencies 不必要地全量 cargo update。默认 feature 关闭和开启都要验证。缺失 Cargo 或真实服务时明确记录，不能把 skipped 算作 passed。

## M1：只读真实 Host Pilot

在 initialize_runtime 的 composition root 或真实测试宿主中使用 AgentRuntime::harness_host。提供合法 RunContext、相同的项目根路径、只含 FileRead 的 bindings、ReadOnlyPilotBoundary 和原生模型配置，调用 Kernel.run。不能直接 Tool.execute。

execute_task 和 execute_turn_from_messages 默认仍走旧路径。增加显式实验开关，经回归批准后只切一个只读入口。图组件仅在宿主回调真实接通时启用执行按钮，禁止 mock API 返回固定成功。

验收：真实读文件任务、Done 前无工具执行、流中断失败、父取消、真实 token usage、原生 reasoning/tool IDs 保留。provider-private reasoning 不直接展示给用户。

## M2：两个旧入口收敛到同一内核

重点 runtime/execute_task.rs、execute_turn.rs、agentic_turn.rs、tool_dispatch.rs、execute_turn_finalize.rs。先建立 golden transcripts 和预算/错误/取消基准。

将 compaction、memory recall/save、profile prompt、failover、language、artifact、delivery acceptance、session notify 接到 Host 的 transform/inference/completion 生命周期。修改调用层而不是删除功能。最终 execute_task、execute_turn、graph node、subagent 都进入同一循环；同一任务不能嵌套新旧循环。完成后更新 ADR 的唯一权威约定。

## M3：完整 subagent 接线

复用 Supervisor 与 KernelNodeExecutor。可信 Agent profiles 解析为能力子集；child RunContext 继承 scope/root/budget/deadline，不能读取环境变量另发一份预算。迁走旧全局 sub_agent_depth 和 parent_task_tool_deny。

写任务使用独立 worktree 或执行容器。默认 concurrency_key=exclusive，确认真实资源隔离才并发。添加兄弟权限不串扰、父子取消、预算合计、容量与深度限制、异常释放槽位测试，再提供 AgentSpawn/Join/Cancel 工具及 UI 事件。

## M4：Graph API 与 UI 接到真实执行

已有执行器和 ReactFlow 组件，尚未挂载产品路由。新路由要检查认证、项目 ACL、CSRF、运行 lease、输入大小、revision 和事件 scope。Start 与 Resume 不同语义。

AgentHostFactory::build 接现有 profile/模型/安全服务；verify 接真实测试命令、产物 hash、报告存储。示例 verifier ID 不是已实现验证器。Human 决策需授权审批人，不能让 LLM 调宿主 resolve_human 自批。

验收：条件分支、all/any、人工暂停、跨租户/改定义恢复拒绝、坏 checkpoint、进程崩溃 Running→Uncertain、Partial 不放行、真实 gate 失败。需要循环时新增有界迭代语义，不删除 cycle 检查。

## M5：电脑与技能成为可发现工具

ComputerBroker 使用宿主授权的 device/lease，模型只见 Observe/Act 的受限 schema；票据由可信 UI 发放。先在专用 X11 虚拟桌面测试，再实现 macOS Accessibility/ScreenCapture、Windows UIAutomation。当前 X11 后端未实机验证，不能宣传支持全部桌面。

复用原 SkillCatalog/SkillsGovernance，把 metadata/search/activate 接现有 install/review/project allowlist。12 个示例技能不自动安装，不授予权限。增加 prompt injection、symlink、内容变化与越权激活用例。

## M6：818cloud 原生接入

可信服务端通过 harness-cloud818 调 /api/v2/sso/introspect，随后 ProductAcl。Desktop 禁止包含 PRODUCT_SSO_CLIENT_SECRET；浏览器复用 product_sso 网关，桌面 BFF pairing 依 PROPOSED 契约另实现。

复用 Accounts UUID 与产品 external_tenant_id，不建第二身份真源、不合并钱包。真实 PostgreSQL/Redis 测试覆盖 revoke、member/grant version 变化、个人/企业切换、其他产品 aud、错误项目和设备撤销。

## M7：持久化与部署强化

文件 checkpoint 不是集群数据库。实现 RunStore、CAS/fencing、节点 lease、budget ledger、transactional outbox。用量事实与钱包结算分开，补齐 cache token/cost；未知副作用对账而非重放。设备 token 入 OS keychain，服务凭证入服务器 secret store。

原生进程树需要 cgroups/job objects 等隔离；当前 subprocess helper 不能当通用不可信 shell sandbox。此前提完成前不开放云多租户任意命令执行。

## M8：最终切换

真实模型、Workbench、scheduler、graph、subagent、desktop、818cloud 端到端验证完成后，逐项目/租户灰度，保留回滚开关与存量数据兼容。构建、模型执行、镜像推送、生产切流分别报告。

## 前端 v1.1 约束

前端需要完整 Checkpoint（version、budget、精确节点集合）。不得把部分 SSE delta 直接当完整快照；在宿主合并后传入。canResolve 需要 graph；更换 run/canonical graph 会重挂载编辑器；onStart 成功后须把正确 graph/checkpoint 成对交回 UI。

## 不得绕过

不删除断言消除失败，不伪造 endpoint/token/gate/usage，不自动给用户 admin，不部署现网、不复制用户凭证、不按页面指令提权。保留旧账户、钱包、订单与本地 session。实机开发前检查未提交改动，只清理本轮创建的终端与进程。
