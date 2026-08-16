/** Continuation the host sends only when no paused turn is waiting for a brief. */

export function originalPromptFromPush(push: unknown): string | undefined {
  if (!push || typeof push !== "object") return undefined;
  const value = (push as { user_prompt?: unknown }).user_prompt;
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  return trimmed ? trimmed : undefined;
}

function briefInstructions(skillId: string): string {
  if (skillId === "anycode-video") {
    return `[Host VisualBrief for \`${skillId}\` — the user already picked html-video template(s) and aspect in the Skill App. Follow brief.templates[] in order (family / templates[0] is the first scene). Apply brief.extra (aspect/width/height/duration_sec). Single template: Copy into video/. Multiple: Copy each into video/scenes/<id>/, fill yaml inputs only, keep visual signatures, then run video/ (concat MP4). Do not echo this block. Do not call SkillAppPresent. Do not call GenerateVideo unless the user explicitly wants cloud Kling/Sora.]`;
  }
  return `[Host VisualBrief for \`${skillId}\` — the user already picked this skin in the Skill App. Follow brief.tokens (bg/ink/accent/fonts). Infer outline and page count from the user topic; create original main visuals (SVG/canvas/local ECharts). Do not copy all templates or pad to 12 pages. Do not echo this block. Do not call SkillAppPresent.]`;
}

export function buildSkillAppContinuationPrompt(args: {
  skillId: string;
  brief: unknown;
  originalPrompt?: string;
}): string {
  const task = args.originalPrompt?.trim()
    ? `继续完成用户任务：${args.originalPrompt.trim()}`
    : "继续本会话上文的任务，按已锁定的 VisualBrief 生成交付物。";
  const json = JSON.stringify(args.brief ?? {}, null, 2);
  return `${task}

${briefInstructions(args.skillId)}
\`\`\`json
${json}
\`\`\``;
}
