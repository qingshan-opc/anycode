import { useMemo } from "react";
import katex from "katex";
import "katex/dist/katex.min.css";
import { useT } from "@/i18n/context";
import { DiagramCardShell } from "@/components/chat/DiagramCardShell";

type Props = {
  code: string;
  sessionId?: string | null;
};

/** Explicit ```math fenced block — display-mode KaTeX with share chrome. */
export function MathDiagram({ code, sessionId = null }: Props) {
  const t = useT();
  const html = useMemo(() => {
    try {
      return katex.renderToString(code, {
        displayMode: true,
        throwOnError: true,
        output: "html",
      });
    } catch {
      return null;
    }
  }, [code]);

  if (html === null) {
    return (
      <pre className="dw-transcript-code">
        <code>{code}</code>
      </pre>
    );
  }

  return (
    <DiagramCardShell
      kind="math"
      source={code}
      label={t("conversations.diagram.mathLabel")}
      sessionId={sessionId}
      rendered
    >
      <div
        className="dw-mermaid-card__canvas overflow-x-auto text-center"
        // KaTeX output is generated locally from the fenced source (strict
        // subset of TeX, no raw HTML pass-through with output: "html").
        dangerouslySetInnerHTML={{ __html: html }}
      />
    </DiagramCardShell>
  );
}
