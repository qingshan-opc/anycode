---
name: cn-weekly-report
description: >-
  Turn a week's commits, tasks, and notes into a Chinese weekly report as branded
  .docx. Use for 周报, 周汇报. Not for daily reports (cn-daily-brief) or
  meeting notes (cn-meeting-minutes).
description_zh: >-
  将一周 git 记录、任务与笔记整理为中文周报，交付品牌 docx（附 HTML 预览）。
  适用于周报、周汇报；日报用 cn-daily-brief，会议纪要用 cn-meeting-minutes。
name_zh: 中文周报
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
    path: "reports/weekly/weekly-report.docx"
  - type: ooxml-openable
    path: "reports/weekly/weekly-report.docx"
---

# cn-weekly-report

> **中文**：根据 git 记录、任务列表、笔记或用户口述，生成标准中文周报并交付 **docx 终稿**。
> **English**: Turn commits, tasks, notes, or user input into a Chinese weekly report delivered as `.docx`.

## Inputs

- **报告周期**：默认为本周（周一至今）；用户可指定起止日期。
- **信息来源**（至少一项，全部可读则全部使用）：
  - `git log --since=<周一> --oneline`（Bash 收集，按主题分组）
  - 任务列表 / 计划树 / 笔记文件（Read）
  - 用户口述的完成项与下周计划
- **受众**：默认面向团队/上级的正式商务中文。

## Steps

1. 确认报告周期与信息来源；git 仓库不可用时标注 `[仅基于用户输入]` 并继续。
2. 用 **Bash**（`git log --since=... --oneline`）、**Glob**/**Grep**、**Read** 收集本周工作证据。
3. **Copy** `templates/weekly-report.md` → `reports/weekly/weekly-report.md`，只改内容：
   - 保留 H1 标题与四个 H2 章节（本周完成 / 进行中 / 下周计划 / 风险与需协调）
   - 每项完成工作必须能在证据中找到依据；不确定处标注「待确认」
   - 至少一条 `Action:` 行（负责人 + 日期），否则 validate 失败
4. `run reports/weekly/weekly-report.md` — validate → HTML 预览 → docx 一次跑完。
5. 在对话中贴出周报正文（便于粘贴飞书/钉钉/邮件），并交付 docx 文件。

## Output

- `reports/weekly/weekly-report.docx` — **终稿（必须）**
- `reports/weekly/weekly-report.preview.html` — 评审预览
- `reports/weekly/weekly-report.md` — 源文件（可 diff）

## Acceptance Checks

- `reports/weekly/weekly-report.docx` 存在且可作为 OOXML(zip) 打开——失败则修复后重跑 `run`，不要改交付路径。

## 禁止

- 禁止捏造未完成的工作项或虚假进度。
- 禁止 TBD / 待补充 / placeholder 占位。
- 禁止只交 Markdown 不交 docx；对话正文是副本，docx 才是交付物。

## 失败恢复

- 信息缺失时先生成「周报草稿」，单列缺失信息，不阻塞其余内容交付。
- 部分材料不可读时，单独列出并继续处理可读内容。

## DeepSeek 执行要点

Read `../_shared/deepseek-office.md` — copy-first：只做表单填写，模板填完必须 `run reports/weekly/weekly-report.md`。
