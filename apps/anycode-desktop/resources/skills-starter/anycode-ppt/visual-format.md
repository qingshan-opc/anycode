# anyCode PPT 视觉格式

用户锁定 **皮肤**（`brief.family` / `brief.tokens`）；**页数与主视觉由模型推断**。  
`templates/` 是可选参考，不是必拷清单。

本 skill 交付 **HTML 分页幻灯片**（1920×1080）。可含 CSS 动效、SVG、canvas、本地 ECharts。不含 `<video>` / `<audio>` / 口播时间轴。不导出 pptx。

## 风格 vs 内容

| | 谁定 |
|--|--|
| 背景 / 文字 / 强调色 / 字体 | VisualBrief（工作台点风格） |
| 页数、大纲、排版、图表与插画 | 模型按题目发挥 |

工作台 **不要** 再让用户勾页型。`brief.templates` 应为空。

## 默认令牌（仅无 VisualBrief 时）

```css
:root {
  --bg: #f2f5f0;
  --ink: #231f20;
  --ink-60: rgba(35, 31, 32, 0.72);
  --ink-40: rgba(35, 31, 32, 0.55);
  --ink-08: rgba(35, 31, 32, 0.08);
  --accent: #1400ff;
  --serif: "Noto Serif SC", "Songti SC", serif;
  --sans: "Noto Sans SC", "PingFang SC", sans-serif;
  --mono: "JetBrains Mono", "SF Mono", monospace;
}
```

有 `brief.tokens` 时，以上全部让位给 brief（含渐变底、大字号、留白等创意排版，只要 token 一致）。

## 画布

- 1920×1080，一页一 HTML
- 可用 padding / 全出血 / 分栏；不必固定 `72px 96px`
- 可选左下品牌小字

## 页数（推断，勿硬凑）

- ≥2 页；**禁止**为凑旧模板表硬凑到 12
- 短 briefing：约 5–8；市场/投标研究：约 8–14；更长仅在叙事需要时

## 主视觉（每页至少一类）

- inline `<svg>`（可 CSS/`<animate>`）
- `<canvas>`（粒子、轨道、银河等）
- 本地 ECharts（`vendor/echarts.min.js`，禁止 CDN）
- `<img>` 或原有组件 class（ladder / metrics / …）作参考

## 禁忌

- 空页（只有标题）
- lingqi 企业蓝/绿、CDN、导出 pptx
- 无视 brief 仍刷 FDE 三色
