# ADR 017: 交付门禁分层与完成判定架构

- 状态:已接受(2026-08-13 落地)
- 决策者:anyCode 维护者
- 关联:ADR 000(AgentRuntime 唯一编排权威)、ADR 010(协作取消)

## 背景

主循环的「完成判定」存在一个根本问题:模型在无 tool_call 的回合宣称完成,
运行时是否应该、以及如何独立验证这一声明?

对标 Claude Code(v2.1.201 反编译 + 2026-03 source map 泄露分析)的结论:
其主循环停止**纯模型推断**,代码层零完成度校验;确定性只用于安全边界
(权限规则、caps、hooks 外壳);唯一内置质量门是 Managed Agents(beta)的
rubric + 独立上下文 grader。hooks(Stop exit 2)把确定性门禁留给用户脚本,
且 hook 超时 fail-open(官方明言 "don't count on a stalled hook to act as a gate")。

anyCode 选择相反方向:**完成判定由确定性代码兜底,模型只承担修复动作**。

## 决策

完成判定分四层,全部挂在「无 tool_call 回合」的守卫骨架
(`runtime/guard_verdict.rs`,两条主循环共用):

1. **申报点验收(delivery_acceptance)** — 产物一经显式申报即按类型验收,
   当场注入返修(早失败)。交付物只认显式声明(申报制),代码写的文件不出卡片。
2. **确定性完成守卫(CompletionGuard)** — `GatePolicy::plan` 从 TaskFamily +
   ExpectedArtifact 生成 GatePlan,ValidatorRegistry 执行(存在性/可打开性/
   结构/代码栈编译)。P0/P1 TaskFailed → Repair(注入诊断);环境失败
   (工具链缺失/超时)→ Partial,不进 repair 死循环。
3. **LLM grader(grader.rs)** — rubric 语义验收 + critic 对抗复核,独立
   上下文。rubric 由任务意图生成一次(每会话缓存);判定 default 立场为
   refuted。显式 refuted 注入返修(预算 1,再犯 → Partial 放行并明示理由);
   LLM 传输失败/输出无法形成判定 → Unavailable,记度量后放行
   (可用性优先;dashboard 手动发布闸 `llm:` 保持更严的 default-to-refuted)。
4. **项目自定义门禁(project_gates)** — `{workspace}/.anycode/gates/` 脚本,
   类 Stop hook 语义但**超时默认 fail-closed**(超时 = TaskFailed),
   显式 `project_gates_fail_open=1` 才放行。

### 反逃逸机制

- **产物反推 family 兜底(family_fallback)**:关键词 `infer_family` 误判时,
  从写工具路径反推 family 并就地合成 GatePlan。只看写工具痕迹——已申报产物
  归申报点验收,不重复兜底;markdown 不算交付信号。
- **栈相关验证(discoverable_verification)**:写了 `.rs` 必须见过成功的
  cargo 系命令才算验证过;代码类写法不依赖空洞短语命中即触发返修。
  确定性门禁已有真实通过项(非 Info 占位)时,evidence repair 旁路。
- **子任务结果门禁**:同步 Agent/Task 结果 status=failed/partial 时,
  完成判定前注入一次「不得作为完成证据」返修。
- **自适应 repair 预算**:失败 gate 集合不变(无进展)立即 Failed;
  有进展可续至 max_repairs=3。

### 度量(delivery_metrics)

全部门禁事件落 `~/.anycode/logs/delivery-gates.jsonl`:`declaration_check` /
`guard_verdict` / `guard_fallback`(family 误判分子)/ `guard_skipped`(应跑未跑)/
`verification_escape`(应拦未拦)/ `grader_verdict`。每周效能报告
(`efficiency_report.rs`)聚合出逃逸率面板。

## 与 Claude Code 的哲学差异(有意为之)

| 维度 | Claude Code | anyCode |
|---|---|---|
| 完成判定 | 纯模型推断 | 确定性代码兜底 + LLM grader 复核 |
| hook 超时 | fail-open | fail-closed(可显式配置 fail-open) |
| 交付物 | 模型自报 | 申报制 + 申报点验收 |
| 语义验收 | Managed Agents grader(beta) | 环内 grader(rubric + critic 融合) |

## 后果

- 正面:空洞完成被结构性拦截;family 关键词误判不再是逃逸口;门禁有效性
  可度量(逃逸率)。
- 代价:每次完成判定多 0–2 次 LLM 调用(grader);代码任务完成时会真实执行
  `cargo check` / `tsc --noEmit` / `py_compile`(有超时与环境失败降级)。
- 风险:grader 误判(refuted 误报)消耗一轮 repair 预算后 Partial 放行,
  不钉死任务;误判率由 `grader_verdict` 度量监控。
