---
name: doc-summary
description: >-
  Summarize local Markdown/text/PDF-like documents in batch, delivered as branded
  .docx. Use for 文档摘要, batch summaries. Not for conversation summaries or
  remote URLs the user did not provide.
description_zh: >-
  批量摘要本地 Markdown/文本/PDF 类文档，汇总交付品牌 docx（附对话摘要）。
  适用于文档摘要、批量总结；不用于会话摘要或用户未提供的远程 URL。
name_zh: 文档摘要
category: office
version: 2.0.0
mode: executable
approval: writes-workspace
channel_capabilities: [files, artifacts]
provides_capabilities: [document.author, document.export.docx]
priority: 120
platforms: [darwin, linux]
permissions:
  read_dirs: [workspace]
  write_dirs: [workspace]
  network: false
acceptance:
  - type: file-exists
    path: "reports/summaries/doc-summary.docx"
  - type: ooxml-openable
    path: "reports/summaries/doc-summary.docx"
---

# doc-summary

> **中文**：批量总结本地 Markdown/文本/类 PDF 文档，汇总交付 **docx 终稿**。
> **English**: Summarize local Markdown/text/PDF-like documents in batch and deliver a compiled `.docx`.

## Inputs

- **文档集**（必填）：至少一个具体文件路径或已上传附件。
  - 用户只说"总结一下"而没有路径时，先询问路径；若摘要的是对话本身，明确标注为**会话摘要**而非文档摘要。
  - 不要主动抓取远程 URL；仅当用户显式给出 URL 时才使用网络能力。
- **摘要深度**：默认每文档 Purpose / Key points / Decisions / Risks / Actions 五段式。

## Steps

1. 确认文档集与摘要深度；逐个用 **Read** 有界分块读取，保留来源路径与标题。
2. **Copy** `templates/doc-summary.md` → `reports/summaries/doc-summary.md`，只改内容：
   - 每个文档一个 H2 章节，五段式填写
   - 多文档时增加"跨文档对比与矛盾点"H2 章节
   - 至少一条 `Action:` 行（负责人 + 日期，来自文档本身的行动项），否则 validate 失败
   - 不可读文件单列一个 H2 章节说明，继续处理可读内容
3. `run reports/summaries/doc-summary.md` — validate → HTML 预览 → docx 一次跑完。
4. 在对话中给出各文档一句话摘要 + 关键风险清单，并交付 docx 文件。

## Output

- `reports/summaries/doc-summary.docx` — **终稿（必须）**
- `reports/summaries/doc-summary.preview.html` — 评审预览
- `reports/summaries/doc-summary.md` — 源文件（可 diff）

## Acceptance Checks

- `reports/summaries/doc-summary.docx` 存在且可作为 OOXML(zip) 打开——失败则修复后重跑 `run`，不要改交付路径。

## 禁止

- 禁止捏造文档内容或声称摘要了未读取的文件。
- 禁止 TBD / 待补充 / placeholder 占位。
- 禁止只交 Markdown 不交 docx；对话摘要是副本，docx 才是交付物。

## 失败恢复

- 超大输入分批摘要，最后做综合章节。
- 区分直接事实与推断；引用本地来源路径与页码/章节标签（如有）。

## DeepSeek 执行要点

Read `../_shared/deepseek-office.md` — copy-first：只做表单填写，模板填完必须 `run reports/summaries/doc-summary.md`。
