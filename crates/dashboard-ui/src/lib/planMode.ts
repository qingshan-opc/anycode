export const PLAN_COMPOSER_MODE = "plan" as const;

const STORAGE_PREFIX = "anycode-plan-mode:";

export function planSlashCommand(locale: "zh" | "en"): string {
  return locale === "zh" ? "计划" : "plan";
}

export function isPlanSlashToken(token: string): boolean {
  const t = token.trim().toLowerCase();
  return t === "计划" || t === "plan" || t === "plan-mode";
}

export function loadPlanMode(sessionKey: string | undefined): boolean {
  if (!sessionKey || typeof sessionStorage === "undefined") return false;
  return sessionStorage.getItem(`${STORAGE_PREFIX}${sessionKey}`) === "1";
}

export function savePlanMode(sessionKey: string | undefined, on: boolean): void {
  if (!sessionKey || typeof sessionStorage === "undefined") return;
  const key = `${STORAGE_PREFIX}${sessionKey}`;
  if (on) sessionStorage.setItem(key, "1");
  else sessionStorage.removeItem(key);
}

/** User phrases that end plan mode after send (user approved the plan). */
export function shouldExitPlanMode(text: string): boolean {
  const s = text.trim();
  if (!s) return false;
  return (
    /开始执行/.test(s) ||
    /按计划/.test(s) ||
    /实施吧/.test(s) ||
    /动手吧/.test(s) ||
    /退出计划/.test(s) ||
    /退出\s*plan/i.test(s) ||
    /\bstart implementing\b/i.test(s) ||
    /\bexecute the plan\b/i.test(s) ||
    /\bgo ahead\b/i.test(s) ||
    /\bbuild\b/i.test(s)
  );
}
