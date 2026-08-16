# 讲解图信息密度（anycode-ppt）

**页数不限**（≥2）。按题目推断大纲，**禁止**为凑 12 个旧模板硬凑页数。

每页 **content** 须至少有 **一类主视觉**；禁止「只有标题」空页。

## 主视觉（validate 认以下任一）

| 形态 | 说明 |
|------|------|
| `<svg` | 原创图示、星空、架构、动效 |
| `<canvas` | 粒子 / 轨道 / 银河等 |
| `echarts` / `#chart` / `class="chart"` | 本地 ECharts |
| `<img` | 插图 / 截图 |
| 旧组件 class | `.ladder` `.layer-stack` `.agent-cycle` `.duo` `.trio` `.metrics` `.timeline` `.checklist` `.quote` `.diagram-box`（可选参考） |

## 页型

- cover / section / closing：**可选**，不必每份 deck 都有齐全一套
- content：按叙事需要；鼓励原创排版，不必套模板 class

## 验收

- 每 content 页有主视觉
- `run slides/` → validate 全绿 + 生成 `index.html`
