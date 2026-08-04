import { memo, useMemo, type MouseEvent } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import "highlight.js/styles/github.css";
import { MermaidDiagram } from "@/components/chat/MermaidDiagram";
import { TableCard } from "@/components/chat/TableCard";
import { useDeliverableProject } from "@/components/deliverables/DeliverableProjectContext";
import { splitMarkdownWithTables } from "@/lib/markdownTable";
import { boundMarkdownAutolinks } from "@/lib/markdownAutolink";
import {
  isExternalHref,
  isLocalPathHref,
  resolveMarkdownLocalPath,
} from "@/lib/markdownLinkPath";
import { openExternal, openLocalPath, revealInFileManager } from "@/lib/openExternal";

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("html", xml);

type Props = {
  text: string;
  className?: string;
  /**
   * While streaming: still render markdown/tables, but skip syntax highlight
   * so incomplete fences stay cheap and stable.
   */
  live?: boolean;
  /** Project root for resolving relative markdown links. */
  projectRoot?: string | null;
};

function MarkdownBlock({
  content,
  components,
}: {
  content: string;
  components: React.ComponentProps<typeof ReactMarkdown>["components"];
}) {
  if (!content.trim()) return null;
  return (
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
      {content}
    </ReactMarkdown>
  );
}

async function openMarkdownHref(
  href: string,
  projectRoot?: string | null,
): Promise<void> {
  const target = href.trim();
  if (!target) return;
  if (isExternalHref(target)) {
    await openExternal(target);
    return;
  }
  if (isLocalPathHref(target)) {
    const abs = resolveMarkdownLocalPath(target, projectRoot);
    try {
      await openLocalPath(abs);
    } catch {
      try {
        await revealInFileManager(abs);
      } catch {
        /* ignore */
      }
    }
  }
}

export const TranscriptMarkdown = memo(function TranscriptMarkdown({
  text,
  className = "",
  live = false,
  projectRoot: projectRootProp = null,
}: Props) {
  const { projectRoot: projectRootFromCtx } = useDeliverableProject();
  const projectRoot = projectRootProp ?? projectRootFromCtx ?? null;
  const displayText = useMemo(() => {
    const stripped = text.replace(/^[ \t]*(\*{3,}|-{3,}|_{3,})[ \t]*$/gm, "");
    return boundMarkdownAutolinks(stripped);
  }, [text]);
  const components = useMemo(
    () => ({
      code({ className: codeClass, children, ...props }: React.ComponentProps<"code">) {
        const match = /language-(\w+)/.exec(codeClass ?? "");
        const raw = String(children).replace(/\n$/, "");
        if (match?.[1] === "mermaid") {
          return <MermaidDiagram code={raw} />;
        }
        if (match) {
          const lang = match[1];
          let highlighted = raw;
          if (!live) {
            try {
              if (hljs.getLanguage(lang)) {
                highlighted = hljs.highlight(raw, { language: lang }).value;
              }
            } catch {
              /* keep raw */
            }
          }
          return (
            <pre className="dw-transcript-code">
              {live ? (
                <code className={codeClass} {...props}>
                  {raw}
                </code>
              ) : (
                <code
                  className={codeClass}
                  dangerouslySetInnerHTML={{ __html: highlighted }}
                  {...props}
                />
              )}
            </pre>
          );
        }
        if (raw.includes("\n")) {
          return (
            <pre className="dw-transcript-code">
              <code {...props}>{raw}</code>
            </pre>
          );
        }
        return (
          <code className="dw-transcript-inline-code" {...props}>
            {children}
          </code>
        );
      },
      a({ href, children, ...props }: React.ComponentProps<"a">) {
        const h = href?.trim() ?? "";
        const local = h.length > 0 && isLocalPathHref(h);
        const external = h.length > 0 && isExternalHref(h);
        return (
          <a
            href={h || undefined}
            target={external ? "_blank" : undefined}
            rel={external ? "noreferrer" : undefined}
            className={local ? "dw-transcript-path-link" : undefined}
            title={local ? resolveMarkdownLocalPath(h, projectRoot) : undefined}
            onClick={(e: MouseEvent<HTMLAnchorElement>) => {
              if (!h) return;
              if (local || external) {
                e.preventDefault();
                void openMarkdownHref(h, projectRoot);
              }
            }}
            {...props}
          >
            {children}
          </a>
        );
      },
    }),
    [live, projectRoot],
  );

  const segments = useMemo(
    () => splitMarkdownWithTables(displayText),
    [displayText],
  );

  return (
    <div className={`dw-transcript-markdown ${className}`}>
      {segments.map((segment, index) =>
        segment.type === "table" ? (
          <TableCard key={`table-${index}`} table={segment.table} />
        ) : (
          <MarkdownBlock key={`md-${index}`} content={segment.content} components={components} />
        ),
      )}
    </div>
  );
});
