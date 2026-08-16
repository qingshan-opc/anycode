# anycode-ppt 组件索引（可选参考）

**v3**：用户只锁皮肤；页数与主视觉由模型发挥。  
`templates/` **不是**必拷清单。版式碰巧合适时再 Copy 改写；否则从零写 HTML + CSS/SVG/ECharts。

**交付**：分页 HTML + `run` 生成 `index.html`。**不导出 pptx。**

## 可选页型骨架

| 文件 | 用途 | data-type |
|------|------|-----------|
| `cover.html` | 封面参考 | cover |
| `section.html` | 章节参考 | section |
| `closing.html` | 收尾参考 | closing |
| `diagram-image.html` | 插图页参考 | content |
| `ladder-flow.html` 等 | 旧讲解图参考 | content |

完整旧类名见历史模板；**新 deck 不必出现这些 class**。

## 创意主视觉（优先）

见 `creative-visuals.md`：

- SVG 星空 / 轨道 / 架构动效
- canvas 粒子场
- 本地 ECharts（`slides/vendor/echarts.min.js`）

## 选用原则

| 内容 | 做法 |
|------|------|
| 市场曲线 / 份额 | 本地 ECharts |
| 抽象愿景 / 生态 | SVG 或 canvas，跟 `--accent` |
| 流程 / 对比 / KPI | 可原创排版，或参考旧模板 |
| 短 briefing | 5–8 页即可，勿硬凑 12 |

## 预览

```bash
~/.anycode/skills/anycode-ppt/run slides/
open slides/index.html
```

## 禁止

- 全拷 12 模板凑数
- CDN、lingqi 色、空 content 页、导出 pptx
