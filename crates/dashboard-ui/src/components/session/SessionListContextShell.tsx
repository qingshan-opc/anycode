import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useT } from "@/i18n/context";

type MenuState = {
  sessionId: string;
  x: number;
  y: number;
};

type Props = {
  onRename?: (sessionId: string, title: string) => void;
  onArchive?: (sessionId: string) => void;
  onHandoffToColleague?: (sessionId: string) => void;
  children: (handlers: {
    onContextMenu: (sessionId: string, event: React.MouseEvent) => void;
    renamingSessionId: string | null;
    onRenameSave: (sessionId: string, title: string) => void;
    onRenameCancel: () => void;
  }) => React.ReactNode;
};

export function SessionListContextShell({
  onRename,
  onArchive,
  onHandoffToColleague,
  children,
}: Props) {
  const t = useT();
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [renamingSessionId, setRenamingSessionId] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menu) return;
    const onPointerDown = (event: MouseEvent) => {
      if (menuRef.current?.contains(event.target as Node)) {
        return;
      }
      setMenu(null);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMenu(null);
      }
    };
    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [menu]);

  const onContextMenu = (sessionId: string, event: React.MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    const menuWidth = 176;
    const menuHeight = 120;
    let x = event.clientX;
    let y = event.clientY;
    if (x + menuWidth > window.innerWidth - 8) {
      x = Math.max(8, window.innerWidth - menuWidth - 8);
    }
    if (y + menuHeight > window.innerHeight - 8) {
      y = Math.max(8, window.innerHeight - menuHeight - 8);
    }
    setMenu({ sessionId, x, y });
  };

  return (
    <>
      {children({
        onContextMenu,
        renamingSessionId,
        onRenameSave: (sessionId, title) => {
          onRename?.(sessionId, title);
          setRenamingSessionId(null);
        },
        onRenameCancel: () => setRenamingSessionId(null),
      })}
      {menu &&
        createPortal(
          <div
            ref={menuRef}
            className="dw-project-menu dw-no-drag"
            style={{ left: menu.x, top: menu.y }}
            role="menu"
          >
            {onRename ? (
              <button
                type="button"
                role="menuitem"
                className="dw-project-menu__item"
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  setRenamingSessionId(menu.sessionId);
                  setMenu(null);
                }}
              >
                <span className="dw-project-menu__label">
                  {t("conversations.renameSession")}
                </span>
              </button>
            ) : null}
            {onArchive ? (
              <button
                type="button"
                role="menuitem"
                className="dw-project-menu__item"
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  const sessionId = menu.sessionId;
                  setMenu(null);
                  onArchive(sessionId);
                }}
              >
                <span className="dw-project-menu__label">
                  {t("conversations.archiveSession")}
                </span>
              </button>
            ) : null}
            {onHandoffToColleague ? (
              <button
                type="button"
                role="menuitem"
                className="dw-project-menu__item"
                onClick={(event) => {
                  event.preventDefault();
                  event.stopPropagation();
                  const sessionId = menu.sessionId;
                  setMenu(null);
                  onHandoffToColleague(sessionId);
                }}
              >
                <span className="dw-project-menu__label">
                  {t("conversations.handoffToColleague")}
                </span>
              </button>
            ) : null}
          </div>,
          document.body,
        )}
    </>
  );
}

export function SessionRenameInput({
  initialTitle,
  label,
  onSave,
  onCancel,
}: {
  initialTitle: string;
  label: string;
  onSave: (title: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initialTitle);
  const cancelledRef = useRef(false);

  return (
    <input
      // eslint-disable-next-line jsx-a11y/no-autofocus
      autoFocus
      className="dw-input text-sm w-full"
      value={draft}
      maxLength={120}
      aria-label={label}
      onChange={(e) => setDraft(e.target.value)}
      onFocus={(e) => e.target.select()}
      onBlur={() => {
        if (cancelledRef.current) {
          cancelledRef.current = false;
          onCancel();
          return;
        }
        const trimmed = draft.trim();
        if (trimmed) {
          onSave(trimmed);
        } else {
          onCancel();
        }
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.currentTarget.blur();
        } else if (e.key === "Escape") {
          cancelledRef.current = true;
          e.currentTarget.blur();
        }
      }}
      onClick={(e) => e.stopPropagation()}
    />
  );
}
