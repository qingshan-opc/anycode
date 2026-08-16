import { useEffect, useRef, useState } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/i18n/context";
import { isTauriDesktop } from "@/lib/desktopShell";
import {
  listOpenWithApps,
  openPathWithApp,
  type OpenWithApp,
} from "@/lib/openExternal";

type Props = {
  x: number;
  y: number;
  absPath?: string;
  onClose: () => void;
  onReveal: () => void;
  onCopyPath: () => void;
  onOpen?: () => void;
};

export function DeliverableContextMenu({
  x,
  y,
  absPath,
  onClose,
  onReveal,
  onCopyPath,
  onOpen,
}: Props) {
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
    if (!desktop || !absPath) {
      setApps([]);
      return;
    }
    let cancelled = false;
    void listOpenWithApps(absPath)
      .then((list) => {
        if (!cancelled) setApps(list);
      })
      .catch(() => {
        if (!cancelled) setApps([]);
      });
    return () => {
      cancelled = true;
    };
  }, [desktop, absPath]);

  return (
    <div
      ref={ref}
      className="dw-deliverable-context-menu"
      style={{ left: x, top: y }}
      role="menu"
    >
      {onOpen ? (
        <button
          type="button"
          role="menuitem"
          className="dw-deliverable-context-menu__item"
          onClick={() => {
            onClose();
            onOpen();
          }}
        >
          <Icon name="folder_open" size={16} />
          <span>{t("conversations.deliverable.open")}</span>
        </button>
      ) : null}

      {desktop && absPath ? (
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
            <Icon
              name={openWithOpen ? "expand_less" : "chevron_right"}
              size={14}
              className="ml-auto"
            />
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
                    onClick={() => {
                      onClose();
                      void openPathWithApp(absPath, app.id).catch((err) => {
                        window.alert(err instanceof Error ? err.message : String(err));
                      });
                    }}
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
        onClick={() => {
          onClose();
          onReveal();
        }}
      >
        <Icon name="folder_open" size={16} />
        <span>{t("conversations.openInFinder")}</span>
      </button>
      <button
        type="button"
        role="menuitem"
        className="dw-deliverable-context-menu__item"
        onClick={() => {
          onClose();
          onCopyPath();
        }}
      >
        <Icon name="content_copy" size={16} />
        <span>{t("conversations.deliverable.copyPath")}</span>
      </button>
    </div>
  );
}
