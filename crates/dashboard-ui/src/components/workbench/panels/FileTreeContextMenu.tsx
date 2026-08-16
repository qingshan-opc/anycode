import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import {
  listOpenWithApps,
  openLocalPath,
  openPathWithApp,
  revealInFileManager,
  type OpenWithApp,
} from "@/lib/openExternal";
import { isTauriDesktop } from "@/lib/desktopShell";

export type FileContextMenuTarget = {
  x: number;
  y: number;
  /** Absolute path for open / reveal. */
  absPath: string;
  /** Project-relative path (for copy). */
  relPath: string;
  isDir: boolean;
};

type Props = {
  target: FileContextMenuTarget;
  onClose: () => void;
};

export function FileTreeContextMenu({ target, onClose }: Props) {
  const t = useT();
  const ref = useRef<HTMLDivElement | null>(null);
  const [apps, setApps] = useState<OpenWithApp[] | null>(null);
  const [openWithOpen, setOpenWithOpen] = useState(false);
  const desktop = isTauriDesktop();

  useEffect(() => {
    const onDoc = (event: MouseEvent) => {
      if (ref.current?.contains(event.target as Node)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  useEffect(() => {
    if (!desktop || target.isDir) {
      setApps([]);
      return;
    }
    let cancelled = false;
    void listOpenWithApps(target.absPath)
      .then((list) => {
        if (!cancelled) setApps(list);
      })
      .catch(() => {
        if (!cancelled) setApps([]);
      });
    return () => {
      cancelled = true;
    };
  }, [desktop, target.absPath, target.isDir]);

  const run = (fn: () => Promise<void>) => {
    onClose();
    void fn().catch((err) => {
      window.alert(err instanceof Error ? err.message : String(err));
    });
  };

  const copy = async (text: string) => {
    await navigator.clipboard.writeText(text);
  };

  return createPortal(
    <div
      ref={ref}
      className="dw-deliverable-context-menu"
      style={{ left: target.x, top: target.y }}
      role="menu"
    >
      {!target.isDir ? (
        <button
          type="button"
          role="menuitem"
          className="dw-deliverable-context-menu__item"
          onClick={() => run(() => openLocalPath(target.absPath))}
        >
          <Icon name="folder_open" size={16} />
          <span>{t("conversations.deliverable.open")}</span>
        </button>
      ) : null}

      {!target.isDir && desktop ? (
        <div className="dw-file-ctx-submenu">
          <button
            type="button"
            role="menuitem"
            className="dw-deliverable-context-menu__item"
            aria-expanded={openWithOpen}
            onClick={() => setOpenWithOpen((v) => !v)}
          >
            <Icon name="apps" size={16} />
            <span>{t("files.openWith")}</span>
            <Icon name={openWithOpen ? "expand_less" : "chevron_right"} size={14} className="ml-auto" />
          </button>
          {openWithOpen ? (
            <div className="dw-file-ctx-submenu__list">
              {apps === null ? (
                <p className="dw-file-ctx-submenu__hint m-0">{t("common.loading")}</p>
              ) : apps.length === 0 ? (
                <p className="dw-file-ctx-submenu__hint m-0">{t("files.openWithEmpty")}</p>
              ) : (
                apps.map((app) => (
                  <button
                    key={app.id}
                    type="button"
                    role="menuitem"
                    className="dw-deliverable-context-menu__item"
                    onClick={() =>
                      run(() => openPathWithApp(target.absPath, app.id))
                    }
                  >
                    <span className="truncate">
                      {app.name}
                      {app.is_default ? ` (${t("files.openWithDefault")})` : ""}
                    </span>
                  </button>
                ))
              )}
            </div>
          ) : null}
        </div>
      ) : null}

      <button
        type="button"
        role="menuitem"
        className="dw-deliverable-context-menu__item"
        onClick={() => run(() => revealInFileManager(target.absPath))}
      >
        <Icon name="folder_open" size={16} />
        <span>{t("conversations.openInFinder")}</span>
      </button>
      <button
        type="button"
        role="menuitem"
        className="dw-deliverable-context-menu__item"
        onClick={() => run(() => copy(target.absPath))}
      >
        <Icon name="content_copy" size={16} />
        <span>{t("files.copyAbsPath")}</span>
      </button>
      {target.relPath && target.relPath !== target.absPath ? (
        <button
          type="button"
          role="menuitem"
          className="dw-deliverable-context-menu__item"
          onClick={() => run(() => copy(target.relPath))}
        >
          <Icon name="content_copy" size={16} />
          <span>{t("files.copyRelPath")}</span>
        </button>
      ) : null}
    </div>,
    document.body,
  );
}
