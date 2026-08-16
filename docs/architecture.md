# anyCode 架构说明

面向维护者：分层、依赖方向与扩展点，避免「为抽象而抽象」。

**文档站**（中英）：[`https://anycode.work/docs/guide/architecture.md`](../docs/user/guide/architecture)；扩展清单 [`contributing-extensions.md`](../https://anycode.work/docs/guide/contributing-extensions)。**ADR** 在 [`docs/adr/`](adr/)。运行流程见 [`ops/run-flow.md`](ops/run-flow.md)。

## 分层与数据流

```text
anycode-desktop / anycode-daemon   ← 产品入口
    ↓
anycode-bootstrap                ← initialize_runtime 组合根
    ↓
anycode-agent                    ← AgentRuntime
    ↓
anycode-core                     ← 领域类型 + trait
    ↑
anycode-tools / llm / security / memory
```

**依赖规则**

- `core` 不依赖 agent / bootstrap / tools。
- `agent` 编排多轮循环；不实现具体工具。
- `bootstrap` 构造 `AgentRuntime`；Desktop、dashboard 内嵌聊天、daemon scheduler 共用。

## 扩展点（优先使用顺序）

1. **新工具**：`anycode-tools` `registry.rs` + `catalog`。
2. **新 LLM 提供商**：`anycode-llm`。
3. **新 Agent 类型**：`Agent` trait + `register_agent`。

## Crate 要点

| Crate | 职责 |
|--------|------|
| `bootstrap` | `initialize_runtime`、工具/安全/记忆组装 |
| `dashboard` | Workbench HTTP、SQLite、**进程内** Agent 执行 |
| `channel-bridge` | `anycode-daemon` 二进制（**仅** `scheduler`） |
| `agent` | `AgentRuntime`、`execute_task` / `execute_turn_from_messages` |
| `core` | `Message` / `Task`、trait |
| `tools` | 工具实现与注册表 |

## 设计原则

- 请求从 Workbench 或 daemon scheduler 进入 `AgentRuntime` 后，在 agent crate 内完成 LLM + 工具循环。
- 子模块拆分优先于新抽象。

## 已移除

- 终端 `anycode` CLI（REPL/TUI/`run`/`setup`/`dashboard` 子命令）
- HTTP `anycode daemon`（见 [`adr/018-http-daemon-deprecated.md`](adr/018-http-daemon-deprecated.md)）
- Dashboard spawn CLI / Agent **sidecar** 子进程
- 第三方 IM 通道桥（WeChat / Telegram / Discord）

## 定时任务（Cron）

- 工具：`CronCreate` / `CronDelete` / `CronList` → `~/.anycode/tasks/orchestration.json`
- 执行：`anycode-daemon scheduler`（`crates/channel-bridge/src/scheduler.rs`）
- 单实例：`~/.anycode/tasks/scheduler.lock`

## Digital Workbench（Dashboard）与 Desktop

详见 [`ops/run-flow.md`](ops/run-flow.md)：

- **`anyCode Workbench`**：Axum HTTP（Desktop 使用 ephemeral `127.0.0.1:0`；独立 `anycode-dashboard-serve` 默认 `:43180` 仅开发）。SQLite `~/.anycode/projects.db`，嵌入 `dashboard-ui`。
- **Agent 在 dashboard 进程内执行**：`ChatRuntimeHost` → `initialize_runtime` → `AgentRuntime::execute_turn_from_messages` / `execute_task`。审批/取消经 `dashboard-ipc`（文件 IPC，兼容历史路径）。
- **Desktop（Tauri）**：`apps/anycode-desktop` **进程内**启动 dashboard，WebView 导航到 loopback；**不** spawn Workbench sidecar。
- **Project**：工作台「项目」= 磁盘工作区 + DB 元数据；模板、Gate Runner、知识库见 `crates/dashboard/` 与 `crates/tools/`。

关键 crate：`crates/dashboard`、`crates/dashboard-ui`（React，非 Cargo member，由 build 嵌入）。

迭代任务与决策状态见 **[`docs/roadmap.md`](roadmap.md)**（SSOT）。
