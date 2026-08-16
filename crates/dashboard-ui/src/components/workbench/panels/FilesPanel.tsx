import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { DeliverableProjectProvider } from "@/components/deliverables/DeliverableProjectContext";
import { isHtmlPath } from "@/lib/htmlSlideDeck";
import { FileTree } from "./FileTree";
import { FilePreview } from "./FilePreview";

type Props = {
  projectId: string;
};

/** Left file tree + right preview. HTML decks hide the tree for an immersive view. */
export function FilesPanel({ projectId }: Props) {
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const htmlSelected = Boolean(selectedPath && isHtmlPath(selectedPath));
  const [showTree, setShowTree] = useState(true);

  useEffect(() => {
    if (htmlSelected) {
      setShowTree(false);
    } else {
      setShowTree(true);
    }
  }, [htmlSelected]);

  const project = useQuery({
    queryKey: ["project", projectId],
    queryFn: () => api.project(projectId),
    enabled: Boolean(projectId),
    staleTime: 60_000,
  });
  const projectRoot = project.data?.project.root_path ?? null;

  return (
    <DeliverableProjectProvider projectId={projectId} projectRoot={projectRoot}>
      <div className="flex h-full min-h-0 min-w-0">
        {showTree ? (
          <div className="w-[min(40%,18rem)] shrink-0 overflow-hidden flex flex-col border-r border-outline-variant/60 bg-surface-container-low/40">
            <FileTree
              projectId={projectId}
              projectRoot={projectRoot}
              selectedPath={selectedPath}
              onSelectPath={setSelectedPath}
            />
          </div>
        ) : null}
        <div className="flex-1 min-w-0 min-h-0">
          <FilePreview
            projectId={projectId}
            filePath={selectedPath}
            onToggleFiles={htmlSelected ? () => setShowTree((value) => !value) : undefined}
            filesVisible={showTree}
          />
        </div>
      </div>
    </DeliverableProjectProvider>
  );
}
