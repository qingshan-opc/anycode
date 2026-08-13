import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Transformer } from "markmap-lib";
import { Markmap } from "markmap-view";
import katex from "katex";
import "katex/dist/katex.min.css";
import { useT } from "@/i18n/context";
import {
  diagramShareUrl,
  fetchDiagramSource,
  type DiagramSourceResponse,
} from "@/lib/diagramShare";

type Props = {
  diagramId: string;
};

function MermaidCanvas({ source }: { source: string }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const mermaid = await import("mermaid");
        mermaid.default.initialize({
          startOnLoad: false,
          theme: "neutral",
          securityLevel: "strict",
        });
        if (!containerRef.current || cancelled) return;
        const { svg } = await mermaid.default.render("mmd-share", source);
        if (!cancelled && containerRef.current) {
          containerRef.current.innerHTML = svg;
        }
      } catch (err) {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [source]);

  if (error) return <pre className="dw-transcript-code">{source}</pre>;
  return <div ref={containerRef} className="diagram-share__canvas" />;
}

function MindmapCanvas({ source }: { source: string }) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const mmRef = useRef<Markmap | null>(null);

  useEffect(() => {
    if (!source.trim() || !svgRef.current) return;
    const { root } = new Transformer().transform(source);
    if (!mmRef.current) {
      mmRef.current = Markmap.create(svgRef.current, { autoFit: true, duration: 0 });
    }
    void mmRef.current.setData(root).then(() => mmRef.current?.fit());
  }, [source]);

  return <svg ref={svgRef} className="w-full h-[min(70vh,720px)]" role="img" />;
}

function MathCanvas({ source }: { source: string }) {
  const html = (() => {
    try {
      return katex.renderToString(source, {
        displayMode: true,
        throwOnError: false,
        output: "html",
      });
    } catch {
      return null;
    }
  })();
  if (html === null) return <pre className="dw-transcript-code">{source}</pre>;
  return (
    <div
      className="diagram-share__canvas overflow-x-auto text-center text-lg"
      // KaTeX output is generated locally (output: "html", no raw HTML pass-through).
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

function DiagramBody({ diagram }: { diagram: DiagramSourceResponse }) {
  if (diagram.kind === "mermaid") return <MermaidCanvas source={diagram.source} />;
  if (diagram.kind === "mindmap") return <MindmapCanvas source={diagram.source} />;
  return <MathCanvas source={diagram.source} />;
}

/**
 * Standalone full-address page for a persisted diagram — no workbench chrome,
 * client-side render only. Reachable without auth so LAN colleagues with the
 * link can open, print, or screenshot it.
 */
export function DiagramSharePage({ diagramId }: Props) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const diagram = useQuery({
    queryKey: ["diagram-share", diagramId],
    queryFn: () => fetchDiagramSource(diagramId),
    staleTime: Infinity,
    retry: false,
  });

  const copyLink = () => {
    void navigator.clipboard?.writeText(diagramShareUrl(diagramId)).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="diagram-share min-h-screen bg-surface text-on-surface">
      <header className="diagram-share__bar">
        <div className="min-w-0">
          <h1 className="m-0 text-sm font-semibold truncate">
            {diagram.data?.title?.trim() || t(`conversations.diagram.kind.${diagram.data?.kind ?? "mermaid"}`)}
          </h1>
          <p className="m-0 mt-0.5 text-[11px] text-secondary truncate">
            {diagramShareUrl(diagramId)}
          </p>
        </div>
        <div className="flex shrink-0 gap-1.5">
          <button type="button" className="dw-btn-ghost text-xs py-1 px-2" onClick={copyLink}>
            {copied ? t("conversations.diagram.copied") : t("conversations.diagram.copyLink")}
          </button>
          <button
            type="button"
            className="dw-btn-ghost text-xs py-1 px-2"
            onClick={() => window.print()}
          >
            {t("conversations.diagram.print")}
          </button>
        </div>
      </header>
      <main className="diagram-share__body">
        {diagram.isPending ? (
          <p className="p-6 text-sm text-secondary">{t("common.loading")}</p>
        ) : diagram.isError ? (
          <p className="p-6 text-sm text-error">
            {t("conversations.diagram.notFound")}
          </p>
        ) : (
          <DiagramBody diagram={diagram.data} />
        )}
      </main>
    </div>
  );
}
