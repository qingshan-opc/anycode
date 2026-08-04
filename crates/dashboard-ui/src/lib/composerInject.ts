/**
 * One-shot / queued prompts injected into the conversation composer
 * (skills, OCR, etc.). Browser Design Mode sends via its own bar — not here.
 */

export type ComposerInjectPayload = {
  /** Markdown / plain text appended (or replacing empty draft). */
  text: string;
  /** When true, focus the composer textarea after inject. */
  focus?: boolean;
  /** Optional source tag for analytics / dedupe. */
  source?: string;
};

type Listener = () => void;

let pending: ComposerInjectPayload | null = null;
const listeners = new Set<Listener>();

function emit() {
  for (const l of listeners) l();
}

/** Queue text for the active conversation composer (consume-once). */
export function injectComposerText(payload: ComposerInjectPayload): void {
  const text = payload.text.trim();
  if (!text) return;
  pending = { ...payload, text };
  emit();
}

/** Read and clear the pending inject (called by ConversationComposer). */
export function consumeComposerInject(): ComposerInjectPayload | null {
  const next = pending;
  pending = null;
  if (next) emit();
  return next;
}

export function getComposerInjectSnapshot(): ComposerInjectPayload | null {
  return pending;
}

export function subscribeComposerInject(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
