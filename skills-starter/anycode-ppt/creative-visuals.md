# 创意主视觉（无网）

皮肤令牌来自 `brief.tokens`：`--bg` `--ink` `--accent`（及 serif/sans）。  
主视觉要把这些色用进 SVG / canvas / ECharts，而不是另起一套企业蓝。

## 禁止 CDN

skill `network: false`，且 `file://` 打开 index。**禁止** `https://cdn…/echarts…`。

## 本地 ECharts

1. 从 skill 包 Copy：`vendor/echarts.min.js` → 工作区 `slides/vendor/echarts.min.js`
2. 页面：

```html
<div id="chart" class="chart" style="width:1100px;height:560px"></div>
<script src="vendor/echarts.min.js"></script>
<script>
  const style = getComputedStyle(document.documentElement);
  const accent = style.getPropertyValue('--accent').trim() || '#1400ff';
  const ink = style.getPropertyValue('--ink').trim() || '#231f20';
  const chart = echarts.init(document.getElementById('chart'));
  chart.setOption({
    color: [accent],
    textStyle: { color: ink },
    /* …series… */
  });
</script>
```

没有图表的页不必引用 ECharts。

## SVG

- 用 `currentColor` 或 `var(--accent)` / `var(--ink)`
- 可用 CSS `@keyframes` 或 SVG `<animate>`（轻量循环即可）
- 示例主题：星空、轨道、星系、网络节点、分层弧线——跟题目相关即可

## Canvas

- 画布尺寸跟 1920×1080 页内区域走；resize 可省略（固定画布）
- 粒子 / 银河：用 `--accent` 做高光，`--ink` 做星点，背景透明露出 `--bg`

## 密度

每页至少一块主视觉区域（图占版心大半），避免「标题 + 三行字」。
