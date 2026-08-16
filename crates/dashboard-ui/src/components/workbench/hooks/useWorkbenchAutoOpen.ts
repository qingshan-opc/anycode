import {
  useBrowserAutoOpen,
  type BrowserAutoOpenCtx,
} from "../panels/BrowserPanel.autoOpen";
import { usePlanAutoOpen, type PlanAutoOpenCtx } from "../panels/PlanTreePanel.autoOpen";
import {
  useSkillAppAutoOpen,
  type SkillAppAutoOpenCtx,
} from "../panels/SkillAppPanel.autoOpen";

export type WorkbenchAutoOpenCtx = BrowserAutoOpenCtx & PlanAutoOpenCtx & SkillAppAutoOpenCtx;

/**
 * Composes every panel's auto-open hook. Hooks are called in a fixed order by
 * name (rules-of-hooks) — registering a new auto-open behavior means adding
 * one line here next to the panel's `*.autoOpen.ts` implementation.
 */
export function useWorkbenchAutoOpen(ctx: WorkbenchAutoOpenCtx): void {
  useBrowserAutoOpen(ctx);
  usePlanAutoOpen(ctx);
  useSkillAppAutoOpen(ctx);
}
