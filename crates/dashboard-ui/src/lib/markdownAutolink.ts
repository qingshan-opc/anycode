/**
 * Bound bare URLs before remark-gfm so CJK punctuation after www./https://
 * does not get swallowed into a huge autolink.
 *
 * Rewrites matches to explicit markdown links: `[label](https://…)`.
 */

/** Characters that always end a bare URL (not part of host/path). */
function isUrlStopChar(ch: string): boolean {
  // Whitespace
  if (/\s/.test(ch)) return true;
  // ASCII wrappers / CJK punctuation that should not be in bare URLs
  return '<>"\'`()[]{}（）【】「」『』、，。；！？'.includes(ch);
}

function takeUrl(input: string, start: number): { raw: string; end: number } | null {
  let i = start;
  while (i < input.length && !isUrlStopChar(input[i]!)) {
    i += 1;
  }
  // Trim trailing ASCII punctuation commonly not part of the URL.
  let end = i;
  while (end > start && /[.,;:!?)\]}'"]/.test(input[end - 1]!)) {
    end -= 1;
  }
  if (end <= start) return null;
  return { raw: input.slice(start, end), end };
}

function alreadyInsideMarkdownLink(input: string, index: number): boolean {
  const before = input.slice(Math.max(0, index - 3), index);
  if (before.endsWith("](")) return true;
  const lookback = input.slice(Math.max(0, index - 200), index);
  const open = lookback.lastIndexOf("](");
  if (open >= 0) {
    const after = lookback.slice(open + 2);
    if (!after.includes(")") && !after.includes("\n")) return true;
  }
  return false;
}

function normalizeHref(raw: string): string {
  if (/^https?:\/\//i.test(raw)) return raw;
  if (/^www\./i.test(raw)) return `https://${raw}`;
  return raw;
}

function linkLabel(href: string, raw: string): string {
  try {
    const u = new URL(href);
    return u.host || raw;
  } catch {
    return raw.replace(/^https?:\/\//i, "");
  }
}

/** Replace bare http(s)/www URLs with bounded markdown links. */
export function boundMarkdownAutolinks(text: string): string {
  if (!text) return text;
  let out = "";
  let i = 0;
  while (i < text.length) {
    if (text.startsWith("```", i)) {
      const close = text.indexOf("```", i + 3);
      if (close < 0) {
        out += text.slice(i);
        break;
      }
      out += text.slice(i, close + 3);
      i = close + 3;
      continue;
    }
    if (text[i] === "`") {
      const close = text.indexOf("`", i + 1);
      if (close < 0) {
        out += text.slice(i);
        break;
      }
      out += text.slice(i, close + 1);
      i = close + 1;
      continue;
    }

    const httpsMatch = text.slice(i).match(/^(https?:\/\/)/i);
    const wwwMatch = !httpsMatch ? text.slice(i).match(/^(www\.)/i) : null;
    if ((httpsMatch || wwwMatch) && !alreadyInsideMarkdownLink(text, i)) {
      const taken = takeUrl(text, i);
      if (taken && (taken.raw.includes(".") || taken.raw.includes("/"))) {
        const href = normalizeHref(taken.raw);
        const label = linkLabel(href, taken.raw);
        out += `[${label}](${href})`;
        i = taken.end;
        continue;
      }
    }

    out += text[i];
    i += 1;
  }
  return out;
}
