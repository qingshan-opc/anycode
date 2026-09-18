# 2026-09-18：真实仓库集成与验证证据

## 与历史补丁报告的区别

本轮已经通过 GitHub connector 写入真实 AnyCode 仓库，并通过 GitHub Actions 编译测试。不是再次离线打包，也不是用户 Cloud Mac 实测。Mac 的 run_command/doctor 均被会话层 FORBIDDEN 拒绝；未读取用户电脑文件、创建终端或进程。GitHub 是另一条明确授权的仓库通道。

基线：`0411ea3a94fe6aa2d37326f9342cfc5d6a13f4aa`。
源码导入：`3e4008c7be94f7512052cec6ee4bb07f0d4bf87a`。
实际接线/格式化/依赖锁提交：`f3ce83b96669f70e76975d5e82c6e208db4c57d8`。
集成分支：`refactor/harness-v1.1-integration-20260918`。

## 已完成的真实测试，准确对应 f3ce83b9

Workflow run：`35314315841`。
准备 job：`105502608882`；Rust job：`105502687350`；Dashboard job：`105502687243`。
三个 job 均 completed/success。准备 job 只向集成分支写入四处核验过的接线、新源码格式与 Cargo.lock，没有对 main 写权限操作；后续测试 checkout 明确的 tested_sha=f3ce83b9。

| 项目 | 实际结果 |
|---|---|
| 新增 Harness Rust 测试 | 42 passed, 0 failed（cloud818 4 + core 11/8 + extensions 7/3/8 + host 1） |
| Harness Rustfmt | 通过 |
| Harness Clippy --all-targets -- -D warnings | 通过 |
| cargo check --locked -p anycode-agent --features harness-v1 --all-targets | 通过，真实旧运行时桥接已编译 |
| 既有 anycode-core --lib | 223 passed |
| 既有 anycode-security --lib | 60 passed |
| 既有 anycode-agent --lib | 360 passed |
| Harness scripted_run | 通过；2 个模型 fixture 回合、工具 intent/end、预算用量14，明确不是在线模型 |
| 图模型 node:test | 52 passed, 0 failed, 0 skipped |
| 既有 Dashboard Vitest | 83 files / 441 tests passed |
| Dashboard tsc -b && vite build | 通过 |

本轮聊天沙盒另复跑原离线包 Python 安装器21/21、Node52/52和 ZIP 完整性。它们不是远端 Rust 测试的替代证据。

## 合入范围与默认行为

四个新 crate、受特性控制的 Runtime 桥接、图组件和严格输入校验、测试、12个技能说明、图/身份契约及文档。`harness-v1` 默认关闭，原 Workbench/scheduler 调用路径没有被自动替换；原 SecurityLayer 审批与审计链仍保留。

首次导入使用了只允许该集成分支的准备 workflow。接线完成后移除一次性写入准备逻辑，保留 read-only 的 Harness 检查。既有 ci.yml 不被放宽或删除；最终 PR 的最新提交检查状态应与上面的 f3ce83b9 证据分别查看，不能混为同一次运行。

后续补充的技能/示例/文档不改变已验证 Rust 源码。最终合并提交与 PR 门禁以 GitHub PR/commit 状态为准，本文件不预言尚未发生的 main 合并或完整仓库检查。

## 必须保留的限制与告警

- 尚未执行：用户 Mac 启动/电脑输入、真实模型、真实 818cloud PostgreSQL/Redis/SSO、图产品路由 E2E、分布式恢复与计费结算。
- 新图 UI 是可编译组件，尚未挂到产品路由；技能是说明，不是12个已完工后端；X11电脑后端不是 macOS 实现。
- Rust CI 设置 ANYCODE_SKIP_BROWSER_SMOKE=1；因此本次通过不代表真实桌面浏览器烟测。
- npm ci 的审计输出有12项依赖漏洞（1低、6中、4高、1严重），Dashboard package.json/package-lock.json 未被本次补丁修改。未运行 audit fix --force，未把这些告警当作已修复。
- Vite 提示部分 chunk >500 kB，构建仍成功；没有把警告抑制成不存在。
- no production deploy；未改用户、钱包、订单、数据库或 818cloud 源码；没有强推、reset/stash/clean 用户工作目录。

## 复验与接续

`docs/harness/CURSOR_START.md` 提供命令和 M0–M8 接续工作。已经在仓库的源码不要再次应用旧 ZIP 补丁。读取当前 PR 的新 head SHA 后，针对该 SHA 重跑门禁；编译成功不等于迁移完成，也不等于生产验收。
