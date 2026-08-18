---
name: anycode-ppt
description: >-
  anyCode HTML slides. User locks a VisualBrief skin (bg/ink/accent/fonts) in the
  PPT studio. Infer outline and page count from the topic; prefer slides.json +
  local compile (or one short Agent/Task per page). Do NOT copy all templates or
  pad to 12 pages. Use for ppt, slides, 幻灯片, 演示文稿. No pptx export.
description_zh: >-
  anyCode HTML 幻灯片：用户只锁定皮肤（背景/文字/强调色），模型按题目推断页数与大纲，
  优先 slides.json 本地编译，或每页短上下文 Agent/Task。禁止为凑模板硬凑 12 页。不导出 pptx。
name_zh: anyCode HTML 幻灯片
category: office
version: 3.2.0
mode: executable
approval: writes-workspace
channel_capabilities: [files, artifacts]
provides_capabilities: [presentation.author]
priority: 125
platforms: [darwin, linux, win32]
permissions:
  read_dirs: [workspace]
  write_dirs: [workspace]
  network: false
ui: ui/surface.yaml
---

# anycode-ppt

**皮肤锁定 · 上下文隔离** — 用户在 PPT 视觉工作台选 **风格家族**。  
Agent **按题目推断大纲与页数**；**父会话只保留大纲**，不要在同一会话里连写 5–14 页整页 HTML。

> 不导出 `.pptx`。默认 FDE 三色 **仅当没有 VisualBrief**。有 brief 必须跟 `brief.tokens`。

## 禁止

- **禁止**为凑组件表硬凑 12 页；**禁止**把全部 `templates/` 当 checklist 拷完 / Read 进上下文
- **禁止** Read `vendor/echarts.min.js`（由 `run` 复制）
- **禁止** CDN（`https://…echarts…` 等）；图表用 skill 包内 `vendor/echarts.min.js`
- **禁止** lingqi 企业蓝 `#1B3A5C` / 绿 `#00B050`、footer 写 `lingqi`
- **禁止** `presentation-commercial-delivery` / `fill_potx` / 生成 `.pptx`（除非用户另外明确要求）
- **禁止**在已锁定 `brief.family` 后仍刷 FDE 默认色（除非 family 就是 `fde-editorial`）
- **禁止**只有标题、没有主视觉的空页
- **禁止**在父会话里对每一页做 FileWrite 整页 HTML 后继续堆下一页（token 爆炸）

## 必须工作流（省 token）

0. **Skill App**：消息里还没有 VisualBrief 时，先调 `SkillAppPresent(skill_id="anycode-ppt", wait="brief")`。  
   - 已有 `[Host VisualBrief …]`：立刻开跑，不要再 Present。
1. **父会话只写大纲** → `slides/OUTLINE.md`（页数、每页角色、标题/要点一行）。
   - 短 briefing：约 **5–8** 页；培训/投标：约 **8–14** 页。
2. **优先路径 A — JSON 编译（推荐）**  
   - Write `slides/slides.json`（见下方 schema），**不要**把 templates 读进模型。  
   - `Bash`：`run compile slides/slides.json` → 生成 `slides/NN-*.html`  
   - 再 `Bash`：`run slides/`（整本一次 validate + index；**不要**一页一验证）
3. **路径 B — 分页短上下文（创意色板 / 需原创 SVG 时）**  
   - 对每一页开一个短 `Agent`/`Task`（或等价子任务），prompt 只含：大纲该页一行 + `brief.tokens` + 目标路径。  
   - 子任务 Write 单页后结束；父会话只收「path ok」摘要，**不要**把 HTML 回灌父上下文。  
   - 全部写完后父会话 **一次** `run slides/`。
4. `od-*` 家族：用 JSON `layout` 指向对应模板槽位，或子任务只填文案，不要套 FDE ladder。
5. 交付：`slides/*.html` + `index.html` + `slide_manifest.json`（+ 可选 evidence）

**路径**：skill 说明与 `vendor/` 在 skill 包；`slides/` 写在项目工作区。

## slides.json（路径 A）

```json
{
  "family": "fde-editorial",
  "tokens": { "bg": "#f2f5f0", "ink": "#231f20", "accent": "#1400ff", "serif": "...", "sans": "..." },
  "slides": [
    { "id": "01-cover", "layout": "cover", "title": "...", "subtitle": "...", "statement": "...", "bullets": [] },
    { "id": "02-ladder", "layout": "ladder", "title": "...", "bullets": ["...", "..."] }
  ]
}
```

`layout` 常用：`cover` | `ladder` | `metrics` | `od-build-minimal` | `od-bold-poster` | 其它 `templates/` 基名（不含 `.html`）。

## 视觉契约

| 来源 | 管什么 |
|------|--------|
| `families.md` / `brief.tokens` | 背景、文字、强调色、字体（写入 JSON tokens，勿整文件灌会话） |
| `od-*` + compile | 锁定版式签名 |
| 子任务 / JSON 字段 | 页数、大纲、文案 |
| `creative-visuals.md` | 仅路径 B 需要原创 SVG 时由**子任务**按需读，父会话不读 |

## DeepSeek

Read `../_shared/deepseek-office.md` — PPT：**整本写完再一次 `run`**；优先 `compile`；禁止 12 模板全拷。
