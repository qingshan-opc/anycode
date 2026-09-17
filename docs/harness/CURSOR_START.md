# 接续开发：先迁移真实执行路径

源码已在仓库，不要再次应用旧 v1/v1.1 补丁。先查看 PR #1、当前 HEAD、README 和最新 Actions 结果。`harness-v1` 默认关闭；普通聊天和 scheduler 仍由原 AgentRuntime 执行。

## M0：可复现基线

执行 `python3 tools/harness/verify.py --mode integrated-rust` 和原仓库 CI 命令。独立成功不等于全产品回归。记录实际 commit、工具版本、退出码及未执行项目；不可删除断言、禁用现有测试或以静默跳过伪装成功。

## M1：真实只读 Host Pilot

在 `crates/agent/src/runtime/harness_bridge.rs` 装配经本地用户授权的工作区，仅允许 FileRead。连接真实模型、既有安全审批链及磁盘 journal。验证允许路径、逃逸路径、超限文件、流中断、取消和消息元数据回传。密钥不能进入日志，先通过这条链再开放写操作。

## M2：旧生命周期迁移

逐项接入原 prompt/task compiler、retry/failover、compaction、memory、delivery acceptance、tool result rendering、审计及取消。用现有聊天和 scheduler 的相同输入做行为回归。一条入口一个开关，保留回滚；不能因新单测通过删除旧生命周期。

## M3：受监管子代理

将 Agent 工具接到 Supervisor + Kernel，携带 RunContext。替换旧全局深度/工具限制槽位；验证并行兄弟不会互串身份、权限、预算和 trace。写任务使用独立 worktree，宿主审查后合并。技能声明不得提升权限。

## M4：图路由与界面

将 HarnessGraphWorkbench 挂到实际路由，连接账号、项目 ACL、CSRF 检查后的 Start/Resume/Human API。服务端核对 definition/scope/run/revision，不信任浏览器校验。测试分支、合流、Partial、Gate 失败、暂停恢复、崩溃与预算恢复。任意循环图或 LangGraph Python API 尚不支持。

## M5：电脑后端

保留设备登记、独占租约、帧有效期、目标绑定、动作审批与 unknown-effect 语义。当前只有可选 X11 后端。macOS 需要独立原生后端和系统授权验证；Browser CDP 不等于原生电脑控制。先在专用测试桌面验收，不自动支付、发布或修改敏感系统配置。

## M6：技能和资源

复用原有安装审核体系，按项目和 agent 筛选元数据，按需读取正文并核对摘要。12 份示例技能是说明包，不是 12 个底层引擎，不自动运行安装钩子或授予权限。

## M7：818cloud

以真实 SSO v2 opaque token/introspection 契约实现产品本地用户、租户和项目 ACL 映射。组织、产品租户、项目、设备权限分离。桌面不得携带 confidential client secret，配对 JSON 仅是提案。现网账号、钱包和订单不得清空、重建或自动迁移。

## M8：交付

真实 UI、模型、电脑、取消恢复与权限验收后，逐入口启用。每轮审查、测试并提交 main；只清理自己创建的测试进程和终端。代码合并、镜像推送、生产部署和公开发行分别处理，不混同状态。
