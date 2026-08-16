# PPT 风格家族

工作台锁定的 `brief.family` 对应下面一行令牌。

**不要**在已锁定家族后继续用 FDE 三色。

## Open Design（版式签名锁定）

`brief.family` 以 `od-` 开头时：**锁定视觉签名**（构图 / 字体 / 强调色只出现在规定位置），优先参考 `templates/od-*.html`，按题目填文案与页数。不要套 FDE ladder 通用版式。

| family | 观感 | bg | ink | accent | 标题字体 |
|---|---|---|---|---|---|
| `od-bold-poster` | 纸白 · 番红巨型倾斜大字 | `#F5F2EF` | `#1C1410` | `#D8000F` | Georgia / Shrikhand-like |
| `od-bold-signal` | 暗底色块章节卡 | `#0e1117` | `#f5f5f7` | `#ff7a3d` | PingFang SC |
| `od-creative-voltage` | 电蓝分屏 + 手写强调 | `#0a0f1c` | `#e8eefc` | `#5eead4` | cursive / PingFang |
| `od-electric-studio` | 上下分屏 quote | `#05060a` | `#111111` | `#1d4ed8` | PingFang SC |
| `od-build-minimal` | 奢华留白细线 | `#faf8f4` | `#222222` | `#c9a227` | Helvetica thin |
| `od-pentagram-stat` | 瑞士网格巨数字 | `#111111` | `#ffffff` | `#ffffff` | Helvetica |
| `od-vignelli` | 红强调无衬线 | `#ffffff` | `#111111` | `#e30613` | Helvetica |

## 色板家族（只锁颜色）

拷贝 `templates/*.html` 后，把每页 `:root` 写成该行（`--ink-60` / `--ink-40` / `--ink-08` 用 ink 的 alpha）。

| family | 观感 | bg | ink | accent | 标题字体 |
|---|---|---|---|---|---|
| `fde-editorial` | 杂志衬线 · 电光蓝（默认） | `#f2f5f0` | `#231f20` | `#1400ff` | Songti / Noto Serif SC |
| `apple-keynote` | 黑底大字 · WWDC | `#000000` | `#ffffff` | `#0a84ff` | -apple-system |
| `swiss-grid` | Helvetica · 信号红 | `#ffffff` | `#111111` | `#e30613` | Helvetica |
| `night-engineering` | 终端数据 · 冷青 | `#0e1117` | `#e6edf3` | `#79c0ff` | ui-monospace |
| `paper-serif` | 奶油纸 · 酒红 | `#f4efe6` | `#3b2a22` | `#8b3a3a` | Georgia / Noto Serif SC |
| `magazine-bold` | 超大刊头 · 黄条 | `#f7f4ee` | `#111111` | `#f5c518` | 超粗无衬线 |
| `nord-frost` | 北极蓝绿 | `#eceff4` | `#2e3440` | `#5e81ac` | PingFang SC |
| `ink-mono` | 高对比黑白 | `#ffffff` | `#000000` | `#000000` | Helvetica |
| `coral-warm` | 暖珊瑚侧栏 | `#faf6f4` | `#231f20` | `#e8826b` | PingFang SC |
| `blueprint` | 海军底线框 | `#0b1c3a` | `#d6e4ff` | `#7ec8ff` | ui-monospace |
| `academic` | 象牙纸双栏 | `#fbfaf7` | `#1a2744` | `#1a365d` | Palatino |
| `tokyo-night` | 暗紫霓虹 | `#1a1b26` | `#c0caf5` | `#7aa2f7` | PingFang SC |
| `neo-brutalism` | 厚边 · 明黄 | `#fff45c` | `#111111` | `#111111` | Arial Black |
| `xiaohongshu` | 小红书白 · 暖红 | `#fffdfa` | `#2b2b2b` | `#fe2c55` | Noto Serif SC |
| `pitch-deck-vc` | 融资路演留白 | `#ffffff` | `#0f172a` | `#6366f1` | PingFang SC |
| `japanese-minimal` | 象牙 · 朱红 · 留白 | `#f6f1e7` | `#1a1a1a` | `#c41e3a` | Noto Serif SC |
| `bauhaus` | 红黄蓝几何 | `#f4f1ea` | `#1a1a1a` | `#e30613` | Helvetica |
| `terminal-green` | 绿屏终端 | `#0b120c` | `#b6f5c0` | `#3dff7a` | ui-monospace |

深色家族（`apple-keynote` / `night-engineering` / `blueprint` / `tokyo-night` / `terminal-green` / `od-bold-signal` / `od-creative-voltage` / `od-electric-studio` / `od-pentagram-stat`）：标题用 `--sans`，不要衬线大字。
