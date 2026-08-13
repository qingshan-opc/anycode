import { useEffect, useRef, useState } from "react";
import { Transformer } from "markmap-lib";
import { Markmap } from "markmap-view";
import { useT } from "@/i18n/context";
import { DiagramCardShell } from "@/components/chat/DiagramCardShell";

type Props = {
  code: string;
  sessionId?: string | null;
};

const transformer = new Transformer();

/**
 * Inline ```mindmap code block: markdown outline → interactive markmap,
 * reusing the deliverable viewer's engine (markmap-lib / markmap-view).
 */
export function MindmapDiagram({ code, sessionId = null }: Props) {
  const t = useT();
  const svgRef = useRef<SVGSVGElement | null>(null);
  const mmRef = useRef<Markmap | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendered, setRendered] = useState(false);

  useEffect(() => {
    if (!code.trim() || !svgRef.current) return;
    let cancelled = false;
    try {
      const { root } = transformer.transform(code);
      if (!mmRef.current) {
        mmRef.current = Markmap.create(svgRef.current, {
          autoFit: true,
          duration: 200,
          paddingX: 12,
        });
      }
      void mmRef.current.setData(root).then(() => {
        if (cancelled) return;
        mmRef.current?.fit();
        setError(null);
        setRendered(true);
      });
    } catch (err) {
      if (!cancelled) {
        setError(err instanceof Error ? err.message : String(err));
      }
    }
    return () => {
      cancelled = true;
    };
  }, [code]);

  if (error) {
    return (
      <pre className="dw-transcript-code">
        <code>{code}</code>
      </pre>
    );
  }

  return (
    <DiagramCardShell
      kind="mindmap"
      source={code}
      label={t("conversations.diagram.mindmapLabel")}
      sessionId={sessionId}
      rendered={rendered}
    >
      <div className="dw-mermaid-card__canvas">
        <svg ref={svgRef} className="w-full h-[min(320px,50vh)]" role="img" />
      </div>
    </DiagramCardShell>
  );
}
