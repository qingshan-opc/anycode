# DeepSeek 办公技能执行手册

> 适用于 `deepseek-v4-flash` / `deepseek-v4-pro` 及兼容 API。目标：**少猜测、多复制、可验证**。

## 通用原则

1. **先读 skill 契约，再 Write** — docx/xlsx/pdf 仍优先 Copy 模板；**anycode-ppt** 先大纲再写页（优先 `slides.json` + `run compile`），**禁止**把 `templates/` 整夹读进上下文。
2. **验证节奏** — docx/xlsx/pdf：填完源文件后必须跑 skill 的 `run`；失败则修源文件重跑，不要手改终稿。  
   **anycode-ppt 例外：整本页写完（或 compile 完）后再一次 `run slides/`**，不要一页一验证。
3. **终稿清单** — 回复里列绝对路径：必须交付物 + 可选预览物。
4. **禁止占位** — 无 TBD / lorem / 待填 / xxx；数字要具体。
5. **外部数据** — 统计、政策、行情必须先查再写；表格加 `Source Name` + `Source URL` 列（plain text，不用 HYPERLINK）。

## 按技能

| 技能 | 源 | 终稿 | run 后必查 |
|------|-----|------|-----------|
| anycode-ppt | 先大纲；皮肤用 `brief.tokens`；优先 `slides.json` + `compile`；可选每页短 Agent；**勿拷全 templates/** | `slides/*.html` + `index.html` | 整本一次 validate、≥2 页、**勿硬凑 12** |
| anycode-docx | 复制 `templates/*.md` | `.docx` + `.preview.html` | Decision/Action 行存在 |
| anycode-xlsx | 复制 `templates/workbook-*.json` | `.xlsx` | recheck 无公式错误、≥3 sheet |
| anycode-pdf | 复制 `templates/*.md` | `.pdf` + `.preview.html` | PDF 非空、中文文档用 GB/T 7714 引用格式 |

## 常见失败 recovery

- **validate 报错** → 对照 `components.md` 补章节/表头，勿删门禁字段。
- **xlsx recheck 报 #REF! / #NAME?** → 修 workbook.json 公式后重新 `run`。
- **pdf 引擎缺失** → 安装 `playwright` + chromium；或交付 preview.html 并说明。
- **PPT token 爆炸** → 立刻停父会话连写 HTML；改 `slides.json` + `run compile`，或每页独立短 Task。
