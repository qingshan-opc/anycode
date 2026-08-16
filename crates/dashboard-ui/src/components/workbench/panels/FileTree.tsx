import { useState } from "react";
import type { FsEntry } from "@/api/types/workbench";
import { Icon } from "@/components/Icon";
import { kindForPath } from "@/lib/artifactKind";
import { resolveDeliverableAbsPath } from "@/lib/deliverablePath";
import { useProjectFsList } from "../hooks/useProjectFileTree";
import {
  FileTreeContextMenu,
  type FileContextMenuTarget,
} from "./FileTreeContextMenu";

function fileIconForPath(path: string): string {
  switch (kindForPath(path)) {
    case "image":
    case "media":
      return "image";
    case "video":
      return "movie";
    case "audio":
      return "mic";
    case "pdf":
    case "report":
      return "description";
    case "spreadsheet":
      return "table_chart";
    case "presentation":
      return "slideshow";
    case "mindmap":
      return "psychology";
    case "document":
      return "article";
    default:
      return "code";
  }
}

type Props = {
  projectId: string;
  projectRoot?: string | null;
  selectedPath: string | null;
  onSelectPath: (path: string | null) => void;
};

type TreeNodeProps = {
  projectId: string;
  projectRoot?: string | null;
  entry: FsEntry;
  depth: number;
  selectedPath: string | null;
  onSelectPath: (path: string | null) => void;
  onContextMenu: (entry: FsEntry, event: React.MouseEvent) => void;
};

function TreeNode({
  projectId,
  projectRoot,
  entry,
  depth,
  selectedPath,
  onSelectPath,
  onContextMenu,
}: TreeNodeProps) {
  const [expanded, setExpanded] = useState(false);
  const isDir = entry.kind === "dir";
  const isSelected = selectedPath === entry.path;

  const children = useProjectFsList(projectId, expanded && isDir ? entry.path : "", {
    enabled: expanded && isDir,
  });
  const childEntries =
    expanded && isDir && children.data?.entries ? children.data.entries : [];

  return (
    <div>
      <button
        type="button"
        className={`conv-file-tree w-full flex items-center gap-1.5 py-1 pr-2 text-left text-sm border-0 bg-transparent cursor-pointer hover:bg-surface-container-low ${
          isSelected ? "bg-surface-container-high text-primary" : "text-on-surface"
        }`}
        style={{ paddingLeft: `${depth * 12 + 8}px` }}
        onClick={() => {
          if (isDir) {
            setExpanded((v) => !v);
          }
          onSelectPath(entry.path);
        }}
        onContextMenu={(event) => onContextMenu(entry, event)}
      >
        {isDir ? (
          <Icon name={expanded ? "expand_more" : "chevron_right"} size={16} className="shrink-0 text-secondary" />
        ) : (
          <span className="w-4 shrink-0" />
        )}
        <Icon
          name={isDir ? "folder" : fileIconForPath(entry.path)}
          size={16}
          className="shrink-0 text-secondary"
        />
        <span className="truncate">{entry.name}</span>
      </button>
      {expanded &&
        isDir &&
        childEntries.map((child) => (
          <TreeNode
            key={child.path}
            projectId={projectId}
            projectRoot={projectRoot}
            entry={child}
            depth={depth + 1}
            selectedPath={selectedPath}
            onSelectPath={onSelectPath}
            onContextMenu={onContextMenu}
          />
        ))}
    </div>
  );
}

export function FileTree({ projectId, projectRoot, selectedPath, onSelectPath }: Props) {
  const root = useProjectFsList(projectId, "");
  const [menu, setMenu] = useState<FileContextMenuTarget | null>(null);

  if (root.isPending) {
    return <p className="text-xs text-secondary px-3 py-2 m-0">Loading…</p>;
  }
  if (root.error) {
    return (
      <p className="text-xs text-error px-3 py-2 m-0">
        {(root.error as Error).message}
      </p>
    );
  }

  return (
    <div className="py-1 min-h-0 overflow-y-auto">
      {root.data?.entries.map((entry) => (
        <TreeNode
          key={entry.path}
          projectId={projectId}
          projectRoot={projectRoot}
          entry={entry}
          depth={0}
          selectedPath={selectedPath}
          onSelectPath={onSelectPath}
          onContextMenu={(node, event) => {
            event.preventDefault();
            event.stopPropagation();
            const absPath = resolveDeliverableAbsPath(node.path, projectRoot);
            setMenu({
              x: event.clientX,
              y: event.clientY,
              absPath: absPath || node.path,
              relPath: node.path,
              isDir: node.kind === "dir",
            });
          }}
        />
      ))}
      {menu ? <FileTreeContextMenu target={menu} onClose={() => setMenu(null)} /> : null}
    </div>
  );
}
