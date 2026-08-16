---
name: anycode-video
description: >-
  anyCode short-form HTML motion video. User picks one html-video template and
  aspect in the Skill App studio. Fill template inputs only — keep the visual
  signature. Preview animated HTML in workbench; run exports MP4 via Playwright
  + ffmpeg. Use for 短视频, html-video, 抖音, 快手, reels, tiktok, 动画标题,
  数据动画, 产品 promo, motion graphics. NOT GenerateVideo / Kling / Sora cloud clips.
description_zh: >-
  anyCode 短视频：在 Skill App 画布点选 html-video 模板与画幅，只填内容、保持视觉签名；
  工作台预览 HTML 动画，run 用 Playwright + ffmpeg 导出 MP4。不是云端 GenerateVideo。
name_zh: anyCode 短视频
category: design
version: 1.0.0
mode: executable
approval: writes-workspace
channel_capabilities: [files, artifacts]
provides_capabilities: [video.html_motion]
priority: 110
platforms: [darwin, linux]
permissions:
  read_dirs: [workspace]
  write_dirs: [workspace]
  network: true
ui: ui/surface.yaml
---

# anycode-video

**模板锁定 · 内容填入** — 用户在短视频工作台点选 **一个** html-video 模板 + 画幅（默认 9:16）。  
Agent **Copy** 该模板源 HTML 到工作区 `video/`，只改 yaml 声明的 inputs，再 `run video/` 导出 MP4。

> 不是 `GenerateVideo`（Kling/Sora）。动画来自模板 HTML/CSS/JS，本地 Chromium 录屏。

## 禁止

- **禁止**在已有 `[Host VisualBrief …]` 时再调 `SkillAppPresent`
- **禁止**改模板颜色、动效、字体栈、构图（视觉签名）
- **禁止**换模板 id（除非用户明确要求）
- **禁止**走 `GenerateVideo`，除非用户明确要云端成片
- **禁止** Remotion 模板（`templates/*/UNSUPPORTED.txt`）

## Inputs

- 用户题目 / 文案 / 数据（来自对话）
- VisualBrief（`SkillAppPresent` 返回或 `[Host VisualBrief]`）：`templates[]` 有序模板 id；`family` = 第一个；`extra.aspect` / `extra.width` / `extra.height` / `extra.duration_sec` / `extra.template_ids`

## Steps

0. **Skill App（LLM 驱动）**：消息里还没有 VisualBrief 时，先调 `SkillAppPresent(skill_id="anycode-video", wait="brief")`，等用户在工作台选好模板（可多选）并点「交给 Agent」。  
   - 消息里已有 `[Host VisualBrief …]` 或工具已返回 `brief`：立刻开跑，不要再 Present，不要回「已锁定」。
1. **Read** 每个选中 id 的 `templates/<id>/template.html-video.yaml` 与 `templates/<id>/SKILL.md`——确认 inputs schema、时长范围。
2. **Copy** 模板源到工作区：
   - **单选**：`templates/<id>/source/`（或根下 `index.html`）→ **`video/`**
   - **多选**：每个 id → **`video/scenes/<id>/`**（有序列表 = 分镜顺序）
3. **Edit** 只替换 yaml `inputs` 字段；按 `brief.extra` 设画布尺寸：
   - `9:16` → 1080×1920  
   - `16:9` → 1920×1080  
   - `1:1` → 1080×1080  
4. **`run video/`** — doctor → 录制 → `video/out.mp4`。多场景时脚本会逐个渲染再 ffmpeg concat。缺依赖时非 0 退出。  
   - 首次本机：在 skill 目录执行 `npm install && npx playwright install chromium`。
5. 交付：申报两行 `ANYCODE_ARTIFACT`（见 Output）。

## Output

工作区至少：

- `video/index.html` — 可在工作台 iframe 预览的动画 HTML  
- `video/out.mp4` — 本地导出成品  

末行示例：

```text
ANYCODE_ARTIFACT:{"path":"<abs>/video/index.html","kind":"html","title":"短视频预览"}
ANYCODE_ARTIFACT:{"path":"<abs>/video/out.mp4","kind":"video","title":"短视频 MP4"}
```

## Acceptance Checks

- [ ] VisualBrief 模板 id 存在且非 `UNSUPPORTED`
- [ ] `video/index.html` 存在且内容非上游 lorem / 占位标题（已换成用户文案）
- [ ] 画布宽高与 `brief.extra` 一致
- [ ] `run video/` 退出码 0 且 `video/out.mp4` 存在、大小 > 0
- [ ] 未调用 `GenerateVideo` / `SkillAppPresent`（有 Host VisualBrief 时）

## 路径

- Skill 包：`templates/`、`scripts/render.mjs`、`run`、`ui/`
- 工作区产物：`video/`
