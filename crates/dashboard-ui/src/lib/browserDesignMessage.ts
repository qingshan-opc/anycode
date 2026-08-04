/** Compact browser Design Mode message encoding for chat + agent. */

export type BrowserDesignHit = {
  tag: string;
  id?: string | null;
  classes?: string[];
  text?: string | null;
  css_selector: string;
  xpath?: string | null;
  url?: string | null;
};

const PREFIX = "@@browser-design@@";

export function chipLabel(hit: BrowserDesignHit): string {
  if (hit.id) return `${hit.tag}#${hit.id}`;
  if (hit.css_selector && hit.css_selector.length <= 42) return hit.css_selector;
  return hit.tag || "element";
}

/** Agent-facing prompt: short user line + structured context block. */
export function encodeBrowserDesignMessage(
  hit: BrowserDesignHit | null,
  instruction: string,
): string {
  const trimmed = instruction.trim();
  const payload = {
    selector: hit?.css_selector ?? "body",
    tag: hit?.tag ?? "page",
    xpath: hit?.xpath ?? null,
    url: hit?.url ?? null,
    text: hit?.text ?? null,
  };
  return [
    `${PREFIX}${JSON.stringify(payload)}`,
    trimmed,
    "",
    "Apply this UI change in the project source. Use selector/xpath above to locate the component.",
  ].join("\n");
}

export type ParsedBrowserDesignMessage = {
  hit: BrowserDesignHit;
  instruction: string;
};

/** Parse a user bubble body produced by encodeBrowserDesignMessage. */
export function parseBrowserDesignMessage(
  body: string,
): ParsedBrowserDesignMessage | null {
  const trimmed = body.trimStart();
  if (!trimmed.startsWith(PREFIX)) return null;
  const nl = trimmed.indexOf("\n");
  const header = nl >= 0 ? trimmed.slice(PREFIX.length, nl) : trimmed.slice(PREFIX.length);
  const rest = nl >= 0 ? trimmed.slice(nl + 1) : "";
  try {
    const raw = JSON.parse(header) as Record<string, unknown>;
    const hit: BrowserDesignHit = {
      tag: typeof raw.tag === "string" ? raw.tag : "element",
      css_selector: typeof raw.selector === "string" ? raw.selector : "",
      xpath: typeof raw.xpath === "string" ? raw.xpath : null,
      url: typeof raw.url === "string" ? raw.url : null,
      text: typeof raw.text === "string" ? raw.text : null,
    };
    // Drop trailing agent instruction line from display.
    const lines = rest.split("\n");
    while (
      lines.length > 0 &&
      (/^Apply this UI change/i.test(lines[lines.length - 1]!.trim()) ||
        lines[lines.length - 1]!.trim() === "")
    ) {
      lines.pop();
    }
    return { hit, instruction: lines.join("\n").trim() };
  } catch {
    return null;
  }
}
