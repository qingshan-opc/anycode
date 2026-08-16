---
name: skill-app-hello
description: >-
  Minimal Skill App fixture that proves the three host slots and persistent
  counter. Use when testing Skill Apps, visual workbench host, or ADR 020.
description_zh: Skill App 三槽宿主与保活计数的最小演示小程序。
name_zh: Skill App 你好
category: engineering
version: 1.0.0
mode: instructions
priority: 10
ui: ui/surface.yaml
permissions:
  network: false
---

# skill-app-hello

Demo Skill App. Call `SkillAppPresent` with `skill_id: skill-app-hello` and
optional `wait: brief` to open the mini-app. Increment the counter, switch slots,
then submit a brief — the count must survive slot moves.
