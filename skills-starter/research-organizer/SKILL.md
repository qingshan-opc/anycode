---
name: research-organizer
description: >-
  Organize scattered research materials into a structured materials/ collection
  with a branded index .docx. Use for 资料整理. Not for file tidying
  (file-organizer) or new research (deep-research).
description_zh: >-
  将散落的调研资料（文件、笔记、链接）整理为结构化 materials/ 资料库，
  并交付品牌索引 docx。适用于资料整理、素材归档；通用文件整理用
  file-organizer，新调研用 deep-research。
name_zh: 资料整理
category: research
version: 1.0.0
mode: executable
approval: writes-workspace
channel_capabilities: [files, artifacts]
provides_capabilities: [document.author, document.export.docx]
priority: 115
platforms: [darwin, linux]
permissions:
  read_dirs: [workspace]
  write_dirs: [workspace]
  network: false
acceptance:
  - type: file-exists
    path: "materials/index.docx"
  - type: ooxml-openable
    path: "materials/index.docx"
---

# research-organizer

> **中文**：把散落的调研资料整理成结构化 `materials/` 资料库，交付 **索引 docx 终稿**。
> **English**: Organize scattered research materials into a structured `materials/` collection and deliver an index `.docx`.

## Inputs

- **资料来源**（至少一项）：待整理的文件/目录路径、笔记、链接清单，或用户描述的 topic。
- **主题名**：资料库目录名（默认 `materials/<topic>/`；单主题可省略子目录直接用 `materials/`）。
- **整理策略**：默认按类型分桶；用户可指定按主题/时间分桶。

## Steps

1. 盘点资料：**Glob**/**Read** 列出全部候选文件，记录来源路径；不可读的单列。
2. 建立结构（**Bash** `mkdir -p` + `cp`，**不移动原文件**，只复制）：
   ```
   materials/
     papers/    # 论文、报告、长文
     notes/     # 笔记、纪要、摘要
     links/     # 链接清单（links.md）
     data/      # 数据表、csv、附件
   ```
3. **Copy** `templates/materials-index.md` → `materials/index.md`，只改内容：
   - 每个条目一行：标题、来源路径、一句话说明
   - 条目归入对应 H2 分桶；空分桶删除该章节
   - 至少一条 `Action:` 行（如"待读"、"待核实"的下一步），否则 validate 失败
4. `run materials/index.md` — validate → HTML 预览 → docx 一次跑完。
5. 在对话中给出整理摘要（各分桶条目数 + 遗漏/不可读清单），并交付索引 docx。

## Output

- `materials/index.docx` — **索引终稿（必须）**
- `materials/index.preview.html` — 评审预览
- `materials/index.md` — 索引源文件（随资料库一起可移交）
- `materials/{papers,notes,links,data}/` — 分桶后的资料副本

## Acceptance Checks

- `materials/index.docx` 存在且可作为 OOXML(zip) 打开——失败则修复后重跑 `run`，不要改交付路径。

## 禁止

- 禁止移动或删除用户的原始文件；一律复制进 `materials/`。
- 禁止捏造条目说明；不确定的条目标注「待确认」。
- 禁止 TBD / 待补充 / placeholder 占位。
- 禁止只交索引 Markdown 不交 docx。

## 失败恢复

- 重复文件按内容去重并在索引中注明"重复已合并"。
- 来源缺失的条目进 `notes/` 并在索引标注 `[来源缺失]`。

## DeepSeek 执行要点

Read `../_shared/deepseek-office.md` — copy-first：只做表单填写，模板填完必须 `run materials/index.md`。
