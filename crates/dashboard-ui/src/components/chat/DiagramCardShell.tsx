import { useEffect, useRef, useState, type ReactNode } from "react";
import { useT } from "@/i18n/context";
import { diagramShareUrl, persistDiagram, type DiagramKind } from "@/lib/diagramShare";
import { openExternal } from "@/lib/openExternal";

type Props = {
  kind: DiagramKind;
  source: string;
  label: string;
  sessionId?: string | null;
  /**
   * True once the diagram rendered successfully — persistence (and therefore
   * the share buttons) only happens for diagrams that actually draw.
   */
  rendered: boolean;
  children: ReactNode;
};

/**
 * Shared card chrome for transcript diagrams: label row + canvas + share
 * toolbar. Persist-on-first-render gives every drawn diagram a stable
 * `/diagram/{id}` full address without any user action.
 */
export function DiagramCardShell({
  kind,
  source,
  label,
  sessionId = null,
  rendered,
  children,
}: Props) {
  const t = useT();
  const [shareId, setShareId] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!rendered || shareId) return;
    let cancelled = false;
    void persistDiagram(kind, source, { sessionId, title: label }).then((id) => {
      if (!cancelled && id) setShareId(id);
    });
    return () => {
      cancelled = true;
    };
  }, [rendered, shareId, kind, source, sessionId, label]);

  useEffect(
    () => () => {
      if (copyTimer.current) clearTimeout(copyTimer.current);
    },
    [],
  );

  const copyLink = () => {
    if (!shareId) return;
    void navigator.clipboard?.writeText(diagramShareUrl(shareId)).then(() => {
      setCopied(true);
      if (copyTimer.current) clearTimeout(copyTimer.current);
      copyTimer.current = setTimeout(() => setCopied(false), 1500);
    });
  };

  return (
    <div className="dw-mermaid-card" data-diagram-kind={kind}>
      <div className="dw-mermaid-card__label flex items-center justify-between gap-2">
        <span className="min-w-0 truncate">{label}</span>
        <span className="flex shrink-0 gap-1">
          <button
            type="button"
            className="dw-btn-ghost text-[11px] py-0.5 px-1.5"
            disabled={!shareId}
            title={shareId ? diagramShareUrl(shareId) : undefined}
            onClick={() => shareId && void openExternal(diagramShareUrl(shareId))}
          >
            {t("conversations.diagram.openFull")}
          </button>
          <button
            type="button"
            className="dw-btn-ghost text-[11px] py-0.5 px-1.5"
            disabled={!shareId}
            onClick={copyLink}
          >
            {copied ? t("conversations.diagram.copied") : t("conversations.diagram.copyLink")}
          </button>
        </span>
      </div>
      {children}
    </div>
  );
}
