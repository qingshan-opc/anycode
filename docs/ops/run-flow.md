# anyCode 运行流程总览

面向维护者与高级用户：从进程启动到 Agent 执行、工作台观测的完整链路。

**相关文档**

- 分层与扩展点：[`architecture.md`](../architecture.md)
- 用户向工作台说明：[`https://anycode.work/docs/guide/workbench.md`](../docs/user/guide/workbench)
- ADR 000（编排权威）：[`adr/000-runtime-orchestration.md`](../adr/000-runtime-orchestration.md)

## 核心结论

1. **两个产品入口**：**anyCode.app**（进程内 dashboard + WebView）、**`anycode-daemon scheduler`**（无头 cron）。
2. **Agent 执行内嵌**：Desktop 与 dashboard HTTP 在同一进程内通过 **`anycode-bootstrap::initialize_runtime`** 构造 `AgentRuntime`；**不再** sidecar / CLI 子进程。
3. **配置统一**：`~/.anycode/config.json`（`anycode-config`）；首次配置走 Workbench **`/setup`**。

## 进程拓扑

```text
┌─────────────────────────────────────────────────────────────────┐
│  anyCode Desktop（apps/anycode-desktop）                         │
│    进程内 Axum（127.0.0.1:0 ephemeral）                          │
│    WebView → http://127.0.0.1:{port}/api/auth/desktop-bootstrap │
│    ANYCODE_DASHBOARD_EMBEDDED_DESKTOP=1                         │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│  anycode-dashboard（Axum，crates/dashboard）                     │
│    SQLite: ~/.anycode/projects.db                               │
│    静态 UI: dashboard-ui                                        │
│    Chat → ChatRuntimeHost → AgentRuntime（bootstrap）           │
│    审批/取消: dashboard-ipc                                     │
└────────────────────────────┬────────────────────────────────────┘
                             │ in-process
┌────────────────────────────▼────────────────────────────────────┐
│  anycode-bootstrap → AgentRuntime → tools / LLM / memory        │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  anycode-daemon（crates/channel-bridge）                         │
│    仅子命令：scheduler                                          │
│    同样 initialize_runtime；与 Desktop 共享 config.json          │
└─────────────────────────────────────────────────────────────────┘
```

## 入口对照

| 场景 | 入口 | AgentRuntime |
|------|------|----------------|
| macOS 日常使用 | anyCode.app（DMG） | 进程内 |
| Workbench 对话 | HTTP chat / trigger | 进程内 |
| Cron / 自动化 | `anycode-daemon scheduler` | 调度循环内 |
| 开发调试 | `cargo tauri dev` / Playwright | 进程内 |
| 浏览器开发壳 | `anycode-dashboard-serve`（默认 :43180） | 进程内；**非发货路径** |

## UI 触发任务（简化）

1. 用户在 Workbench 发送消息  
2. `web_chat_dispatch` → `ChatRuntimeHost`（`execute_turn_from_messages`）  
3. 流式事件经 SSE 推送到前端；写入 `projects.db` / `chat_turn_events`

## 审批路径

- **Workbench**：Settings → Security；进行中审批走 Web inbox + `dashboard-ipc`

## 已移除

- 终端 `anycode` 二进制（REPL/TUI/`run`/`setup`/`dashboard` 子命令）  
- HTTP `anycode daemon`（POST `/v1/tasks`）— 见 ADR 003-http-daemon-deprecated  
- Dashboard / Desktop spawn Agent sidecar  
- WeChat / Telegram / Discord 长驻桥  

## 代码锚点

| 区域 | 路径 |
|------|------|
| Desktop 内嵌 dashboard | `apps/anycode-desktop/src/dashboard_backend.rs` |
| Runtime 组装 | `crates/bootstrap/src/runtime.rs` |
| Web 聊天 | `crates/dashboard/src/control/chat_runtime/` + `web_chat_dispatch.rs` |
| Cron | `crates/channel-bridge/src/scheduler.rs` |
