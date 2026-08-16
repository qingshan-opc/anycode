import { useCallback, useState } from "react";
import { Icon } from "@/components/Icon";
import { DeliverableContextMenu } from "@/components/deliverables/DeliverableContextMenu";
import { useDeliverableProject } from "@/components/deliverables/DeliverableProjectContext";
import { useClipboard } from "@/hooks/useClipboard";
import { useT } from "@/i18n/context";
import { resolveDeliverableAbsPath } from "@/lib/deliverablePath";
import { openLocalPath, revealInFileManager } from "@/lib/openExternal";

type Props = {
  path: string;
  projectId?: string;
  projectRoot?: string | null;
  /** Open this file instead of `path` (e.g. deck index.html). */
  openPath?: string;
  downloadUrl?: string;
  downloadName?: string;
  copyImageUrl?: string;
  compact?: boolean;
};

export function DeliverableFileActions({
  path,
  projectId: _projectIdProp,
  projectRoot: projectRootProp,
  openPath,
  downloadUrl,
  downloadName,
  copyImageUrl,
  compact = false,
}: Props) {
  const t = useT();
  const ctx = useDeliverableProject();
  const projectRoot = projectRootProp ?? ctx.projectRoot;
  const { copy, copyImage, copied, copiedImage } = useClipboard();
  const absPath = resolveDeliverableAbsPath(path, projectRoot);
  const absOpenPath = resolveDeliverableAbsPath(openPath?.trim() || path, projectRoot);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const onOpen = useCallback(() => {
    void (async () => {
      try {
        await openLocalPath(absOpenPath);
      } catch (err) {
        try {
          await revealInFileManager(absOpenPath);
        } catch {
          window.alert(
            err instanceof Error ? err.message : t("conversations.openInFinderFailed"),
          );
        }
      }
    })();
  }, [absOpenPath, t]);

  const onReveal = useCallback(() => {
    void revealInFileManager(absPath).catch((err) => {
      window.alert(
        err instanceof Error ? err.message : t("conversations.openInFinderFailed"),
      );
    });
  }, [absPath, t]);

  const onCopyPath = useCallback(() => {
    void copy(absPath || path);
  }, [absPath, copy, path]);

  const btnClass = compact ? "dw-btn-ghost text-xs py-1 px-2" : "dw-btn-secondary text-xs";

  return (
    <div
      className={`flex flex-wrap items-center gap-1.5 ${compact ? "" : "mt-3"}`}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        setMenu({ x: event.clientX, y: event.clientY });
      }}
    >
      <button type="button" className={btnClass} onClick={onOpen}>
        <Icon name="folder_open" size={14} className="inline mr-1" />
        {t("conversations.deliverable.open")}
      </button>
      <button type="button" className={btnClass} onClick={onCopyPath}>
        <Icon name="content_copy" size={14} className="inline mr-1" />
        {copied ? t("common.copied") : t("conversations.deliverable.copyPath")}
      </button>
      {copyImageUrl ? (
        <button
          type="button"
          className={btnClass}
          onClick={() => void copyImage(copyImageUrl)}
        >
          <Icon name="image" size={14} className="inline mr-1" />
          {copiedImage ? t("common.copied") : t("conversations.deliverable.copyImage")}
        </button>
      ) : null}
      {downloadUrl ? (
        <a
          href={downloadUrl}
          download={downloadName}
          className={`${btnClass} no-underline inline-flex items-center`}
        >
          <Icon name="download" size={14} className="inline mr-1" />
          {t("conversations.deliverable.download")}
        </a>
      ) : null}

      {menu ? (
        <DeliverableContextMenu
          x={menu.x}
          y={menu.y}
          absPath={absOpenPath}
          onClose={() => setMenu(null)}
          onReveal={onReveal}
          onCopyPath={onCopyPath}
          onOpen={onOpen}
        />
      ) : null}
    </div>
  );
}
