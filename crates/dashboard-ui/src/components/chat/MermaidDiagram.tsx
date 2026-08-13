import { useEffect, useId, useRef, useState } from "react";
import { useT } from "@/i18n/context";
import { DiagramCardShell } from "@/components/chat/DiagramCardShell";

type Props = {
  code: string;
  sessionId?: string | null;
};

export function MermaidDiagram({ code, sessionId = null }: Props) {
  const t = useT();
  const containerRef = useRef<HTMLDivElement>(null);
  const diagramId = useId().replace(/:/g, "");
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const render = async () => {
      try {
        const mermaid = await import("mermaid");
        mermaid.default.initialize({
          startOnLoad: false,
          theme: "neutral",
          securityLevel: "strict",
        });
        if (!containerRef.current || cancelled) return;
        const { svg } = await mermaid.default.render(`mmd-${diagramId}`, code);
        if (!cancelled && containerRef.current) {
          containerRef.current.innerHTML = svg;
          setError(null);
          setRendered(true);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    };
    void render();
    return () => {
      cancelled = true;
    };
  }, [code, diagramId]);

  if (error) {
    return (
      <pre className="dw-transcript-code">
        <code>{code}</code>
      </pre>
    );
  }

  return (
    <DiagramCardShell
      kind="mermaid"
      source={code}
      label={t("conversations.mermaid.label")}
      sessionId={sessionId}
      rendered={rendered}
    >
      <div ref={containerRef} className="dw-mermaid-card__canvas" />
    </DiagramCardShell>
  );
}
