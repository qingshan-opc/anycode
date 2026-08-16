/**
 * Host-side bridge for Skill App iframes (postMessage JSON-RPC).
 * The mini-app speaks methods under `anycode.*`; we never let it call REST.
 */

export type SkillAppBridgeHandlers = {
  onStateGet: () => Promise<unknown>;
  onStateSet: (patch: unknown) => Promise<unknown>;
  onBriefSubmit: (brief: unknown) => Promise<unknown>;
  onAgentPrompt: (payload: { text?: string; brief?: unknown }) => Promise<unknown>;
  onBackToChat?: () => Promise<unknown> | unknown;
  onFilesRead?: (relPath: string) => Promise<unknown>;
};

type RpcMessage = {
  jsonrpc?: string;
  id?: string | number;
  method?: string;
  params?: unknown;
  result?: unknown;
  error?: { message: string };
};

const ALLOWED = new Set([
  "anycode.ready",
  "anycode.state.get",
  "anycode.state.set",
  "anycode.brief.submit",
  "anycode.agent.prompt",
  "anycode.files.read",
  "anycode.ui.backToChat",
]);

export function attachSkillAppBridge(
  iframe: HTMLIFrameElement,
  handlers: SkillAppBridgeHandlers,
): () => void {
  const onMessage = (ev: MessageEvent) => {
    if (ev.source !== iframe.contentWindow) return;
    const data = ev.data as RpcMessage;
    if (!data || typeof data !== "object" || !data.method) return;
    if (!ALLOWED.has(data.method)) return;
    void (async () => {
      try {
        let result: unknown = null;
        switch (data.method) {
          case "anycode.ready":
            result = { ok: true };
            break;
          case "anycode.state.get":
            result = await handlers.onStateGet();
            break;
          case "anycode.state.set":
            result = await handlers.onStateSet(data.params);
            break;
          case "anycode.brief.submit":
            result = await handlers.onBriefSubmit(data.params);
            break;
          case "anycode.agent.prompt":
            result = await handlers.onAgentPrompt(
              (data.params ?? {}) as { text?: string; brief?: unknown },
            );
            break;
          case "anycode.ui.backToChat":
            result = (await handlers.onBackToChat?.()) ?? { ok: true };
            break;
          case "anycode.files.read": {
            const rel =
              typeof data.params === "string"
                ? data.params
                : ((data.params as { path?: string } | undefined)?.path ?? "");
            result = handlers.onFilesRead
              ? await handlers.onFilesRead(rel)
              : { error: "files.read not permitted" };
            break;
          }
          default:
            break;
        }
        if (data.id != null && iframe.contentWindow) {
          iframe.contentWindow.postMessage(
            { jsonrpc: "2.0", id: data.id, result },
            "*",
          );
        }
      } catch (e) {
        if (data.id != null && iframe.contentWindow) {
          iframe.contentWindow.postMessage(
            {
              jsonrpc: "2.0",
              id: data.id,
              error: { message: e instanceof Error ? e.message : String(e) },
            },
            "*",
          );
        }
      }
    })();
  };
  window.addEventListener("message", onMessage);
  return () => window.removeEventListener("message", onMessage);
}

/** Push payload into the mini-app (agent → app). */
export function pushToSkillApp(iframe: HTMLIFrameElement | null, payload: unknown): void {
  iframe?.contentWindow?.postMessage(
    { jsonrpc: "2.0", method: "anycode.agent.push", params: payload },
    "*",
  );
}

/** Tiny SDK snippet skill authors can copy; also injectable via srcdoc bootstrap. */
export const SKILL_APP_SDK_SNIPPET = `
window.anycode = (function () {
  let seq = 0;
  const pending = new Map();
  const pushListeners = [];
  window.addEventListener("message", (ev) => {
    const d = ev.data;
    if (!d || typeof d !== "object") return;
    if (d.method === "anycode.agent.push") {
      pushListeners.forEach((fn) => fn(d.params));
      return;
    }
    if (d.id != null && pending.has(d.id)) {
      const { resolve, reject } = pending.get(d.id);
      pending.delete(d.id);
      if (d.error) reject(new Error(d.error.message || "bridge error"));
      else resolve(d.result);
    }
  });
  function call(method, params) {
    return new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, { resolve, reject });
      parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*");
    });
  }
  return {
    ready: () => call("anycode.ready"),
    state: {
      get: () => call("anycode.state.get"),
      set: (patch) => call("anycode.state.set", patch),
    },
    brief: { submit: (brief) => call("anycode.brief.submit", brief) },
    agent: {
      prompt: (payload) => call("anycode.agent.prompt", payload),
      onPush: (fn) => { pushListeners.push(fn); },
    },
    ui: { backToChat: () => call("anycode.ui.backToChat") },
    files: { read: (path) => call("anycode.files.read", { path }) },
  };
})();
`;
