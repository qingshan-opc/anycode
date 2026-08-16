---
title: Agent 技能
description: SKILL.md 布局、~/.anycode/skills 发现、skills.* 配置与 Skill 工具。
summary: anyCode 如何发现技能、注入系统提示，以及如何运行可选的 run 脚本。
---

# Agent 技能

anyCode 对齐常见 **Agent Skills** 约定：每个技能是一个目录，内含 **`SKILL.md`**（YAML frontmatter 含 **`name`**、**`description`**）。可选可执行文件 **`run`** 由 **`Skill`** 工具调用（风险级别接近 **Bash**，走审批策略）。

## 布局

- **用户根目录：** `~/.anycode/skills/<skill_id>/`
- **项目覆盖：** `<cwd>/skills/<skill_id>/` 或 `<cwd>/.anycode/skills/<skill_id>/`

可选 **`run`**：在**任务工作目录**下执行（不是技能目录）。设置环境变量 `ANYCODE_SKILL_DIR`、`ANYCODE_WORKING_DIR`。默认 **`skills.minimal_env=true`**。

若 frontmatter 写 **`permissions.network: false`**，主机**拒绝执行 `run`**（失败即拒绝）。可无 `args` 只加载说明。

## 配置

见英文版表格（`docs/user/en/guide/skills.md`）。默认 `minimal_env: true`。

## 工作台

在 **设置 → 技能** 管理。终端 `anycode skills` CLI 已随 CLI 产品面移除。

## Skill App（技能小程序）

Skill 可在 `ui/` 下提供沙箱 HTML 小程序（见 [ADR 020](https://github.com/qingjiuzys/anycode/blob/main/docs/adr/020-skill-apps.md)），挂到右侧 dock、会话主区或项目钉。Agent 工具：`SkillAppPresent` / `SkillAppPush` / `SkillAppRead`。示例：`skills-starter/skill-app-hello`、`anycode-ppt/ui`、`anycode-video/ui`。

内置 **anycode-ppt** 工作台：选一种皮肤（Open Design 版式或色板），再点「交给 Agent」。由模型调用 `SkillAppPresent` 打开工作台；页数与主视觉由模型推断，不要硬凑 12 页。

内置 **anycode-video** 工作台：锁定画幅并多选 html-video 模板，再点「交给 Agent」；Agent 只填 inputs、保持视觉签名，`run` 导出 MP4（不是云端 `GenerateVideo`）。
