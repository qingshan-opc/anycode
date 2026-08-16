/** Module store for the active Skill App in the workbench host. */

export type SkillAppFocus = {
  skillId: string;
  presentId?: string;
  slot?: string;
  waitBrief?: boolean;
  push?: unknown;
};

type Listener = () => void;

let focus: SkillAppFocus | null = null;
const listeners = new Set<Listener>();

function emit() {
  for (const l of listeners) l();
}

export const skillAppStore = {
  getFocus(): SkillAppFocus | null {
    return focus;
  },
  setFocus(next: SkillAppFocus | null) {
    focus = next;
    emit();
  },
  /** End the current HITL present cycle — clears focus so the iframe unmounts. */
  dismiss(): void {
    if (focus == null) return;
    focus = null;
    emit();
  },
  subscribe(listener: Listener): () => void {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};
