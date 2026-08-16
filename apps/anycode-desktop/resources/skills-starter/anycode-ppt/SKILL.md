---
name: anycode-ppt
description: >-
  anyCode HTML slides. User locks a VisualBrief skin (bg/ink/accent/fonts) in the
  PPT studio. Infer outline and page count from the topic; create original main
  visuals (SVG, canvas, local ECharts). Do NOT copy all templates or pad to 12
  pages. Use for ppt, slides, 幻灯片, 演示文稿. No pptx export.
description_zh: >-
  anyCode HTML 幻灯片：用户只锁定皮肤（背景/文字/强调色），模型按题目推断页数与大纲，
  用 SVG / canvas / 本地 ECharts 做原创主视觉。禁止为凑模板硬凑 12 页。不导出 pptx。
name_zh: anyCode HTML 幻灯片
category: office
version: 3.1.0
mode: executable
approval: writes-workspace
channel_capabilities: [files, artifacts]
provides_capabilities: [presentation.author]
priority: 125
platforms: [darwin, linux]
permissions:
  read_dirs: [workspace]
  write_dirs: [workspace]
  network: false
ui: ui/surface.yaml
---

# anycode-ppt

**皮肤锁定 · 创意放开** — 用户在 PPT 视觉工作台选 **风格家族**（Open Design 版式签名或色板）。  
Agent **按题目推断大纲与页数**；色板家族用原创主视觉，`od-*` 家族锁定视觉签名只填文案。

> 不导出 `.pptx`。默认 FDE 三色 **仅当没有 VisualBrief**。有 brief 必须跟 `brief.tokens`。

## 禁止

- **禁止**为凑组件表硬凑 12 页；**禁止**把全部 `templates/` 当 checklist 拷完
- **禁止** CDN（`https://…echarts…` 等）；图表用 skill 包内 `vendor/echarts.min.js`
- **禁止** lingqi 企业蓝 `#1B3A5C` / 绿 `#00B050`、footer 写 `lingqi`
- **禁止** `presentation-commercial-delivery` / `fill_potx` / 生成 `.pptx`（除非用户另外明确要求）
- **禁止**在已锁定 `brief.family` 后仍刷 FDE 默认色（除非 family 就是 `fde-editorial`）
- **禁止**只有标题、没有主视觉的空页

## 必须工作流

0. **Skill App（LLM 驱动）**：消息里还没有 VisualBrief 时，先调 `SkillAppPresent(skill_id="anycode-ppt", wait="brief")`，等用户选好风格并点「交给 Agent」。  
   - 消息里已有 `[Host VisualBrief …]` 或工具已返回 `brief`：立刻开跑，不要再 Present，不要回「已锁定」。
1. **先写大纲**（可写进回复或 `slides/OUTLINE.md`）：按题目决定页数与每页叙事角色。
   - 短 briefing：约 **5–8** 页
   - 培训 / 投标 / 研究报告：约 **8–14** 页
   - 只在叙事需要时加页；同一结构可重复，也可完全不用旧模板
2. **Read** skill 包：`families.md`、`creative-visuals.md`、可选 `templates/`（参考，非必拷）
   - 若 `brief.family` 以 `od-` 开头：**锁定视觉签名**（构图/字体/强调色位置），像视频模板一样只填文案，不要套 FDE ladder 通用版式
3. **Write** `slides/NN-slug.html`：
   - 画布 1920×1080；每页 `:root` 写 `brief.tokens`（`--bg` `--ink` `--accent` 与字体）
   - **鼓励**原创 CSS、inline SVG（可动画）、canvas（银河/粒子）、本地 ECharts
   - `templates/` 仅在版式碰巧合适时复制改写；`od-*` 家族优先用对应 `templates/od-*.html`
4. 若用到 ECharts：把 `vendor/echarts.min.js` **Copy** 到 `slides/vendor/echarts.min.js`，页面用相对路径引用
5. `run slides/` — design + validate + 生成无侧栏 `index.html`
6. 交付：`slides/*.html` + `index.html` + `slide_manifest.json`（+ 可选 evidence）

**路径**：skill 说明与 `vendor/` 在 skill 包；`slides/` 写在项目工作区。

## 视觉契约

| 来源 | 管什么 |
|------|--------|
| `families.md` / `brief.tokens` | 背景、文字、强调色、字体 |
| `od-*` 家族 + `templates/od-*.html` | 锁定版式签名（Open Design） |
| 模型 | 页数、大纲、排版、主视觉创意（色板家族） |
| `creative-visuals.md` | SVG / ECharts / canvas 写法与无网约定 |
| `diagram-density.md` | 每页须有主视觉（禁止空页） |

## 预览

- **`index.html`** — 无侧栏；1920×1080 contain 缩放；`←` `→` 翻页
- **终稿** = `slides/` 下 HTML 分页

## DeepSeek

Read `../_shared/deepseek-office.md` — 先大纲再写页；允许自写 CSS/SVG/ECharts；禁止 12 模板全拷；改完必须 `run slides/`。
