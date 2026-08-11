/**
 * Extract `@`-mentioned skill ids from composer text, keeping first-mention
 * order and dropping unknown ids. Replaces the removed skills picker: typing
 * `@skill-id` in the message is how users pin a skill for the run.
 */
export function extractMentionedSkillIds(
  text: string,
  knownIds: readonly string[],
): string[] {
  if (!text.includes("@") || knownIds.length === 0) return [];
  const known = new Set(knownIds);
  const out: string[] = [];
  for (const match of text.matchAll(/@([\w.-]+)/g)) {
    const id = match[1] ?? "";
    if (known.has(id) && !out.includes(id)) out.push(id);
  }
  return out;
}
