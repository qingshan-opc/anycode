import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { api } from "@/api/client";
import { Icon } from "@/components/Icon";
import { DeliverableFileActions } from "@/components/deliverables/DeliverableFileActions";
import { useDeliverableProject } from "@/components/deliverables/DeliverableProjectContext";
import { useProjectFsList } from "@/components/workbench/hooks/useProjectFileTree";
import { useDeliverableFileMeta } from "@/hooks/useDeliverableFileMeta";
import { useT } from "@/i18n/context";
import {
  deckIndexPath,
  initialSlideIndex,
  isDeckIndexName,
  parentDir,
  resolveDeckSlides,
  type SlideManifest,
} from "@/lib/htmlSlideDeck";
import { basename } from "@/lib/pathUtils";
import { projectFsRawUrl } from "@/lib/projectFsUrl";

const SLIDE_W = 1920;
const SLIDE_H = 1080;
const STAGE_PAD = 24;

type Props = {
  path: string;
  projectId: string;
  projectRoot?: string | null;
  onShowFiles?: () => void;
  filesVisible?: boolean;
};

export function HtmlImmersiveViewer({
  path,
  projectId,
  projectRoot: projectRootProp,
  onShowFiles,
  filesVisible = false,
}: Props) {
  const t = useT();
  const ctx = useDeliverableProject();
  const projectRoot = projectRootProp ?? ctx.projectRoot ?? null;
  const stageRef = useRef<HTMLDivElement | null>(null);
  const [scale, setScale] = useState(1);
  const dir = parentDir(path);
  const dirList = useProjectFsList(projectId, dir);
  const nestedList = useProjectFsList(projectId, "slides", {
    enabled: isDeckIndexName(basename(path)) && !dir,
  });
  const manifestPath = dir ? `${dir}/slide_manifest.json` : "slide_manifest.json";
  const manifest = useQuery({
    queryKey: ["slide-manifest", projectId, manifestPath],
    queryFn: async () => {
      const res = await api.readProjectFs(projectId, manifestPath, 256 * 1024);
      return JSON.parse(res.file.content ?? "{}") as SlideManifest;
    },
    enabled: Boolean(projectId),
    staleTime: 60_000,
    retry: false,
  });

  const slides = useMemo(
    () =>
      resolveDeckSlides({
        path,
        entries: dirList.data?.entries ?? [],
        nestedEntries: nestedList.data?.entries,
        manifest: manifest.data,
      }),
    [dirList.data?.entries, manifest.data, nestedList.data?.entries, path],
  );

  const [index, setIndex] = useState(() => initialSlideIndex(slides, path));
  useEffect(() => {
    setIndex(initialSlideIndex(slides, path));
  }, [path, slides]);

  const waitingForSlides =
    isDeckIndexName(basename(path)) && slides.length === 0 && dirList.isPending;
  const currentPath = slides[index] ?? path;
  const openPath = deckIndexPath({
    selectedPath: path,
    entries: dirList.data?.entries ?? [],
    nestedEntries: nestedList.data?.entries,
  });
  const { fileName, downloadUrl } = useDeliverableFileMeta(projectId, openPath);
  const previewUrl = projectFsRawUrl(projectId, currentPath, projectRoot);
  const total = slides.length;
  const canPrev = total > 1 && index > 0;
  const canNext = total > 1 && index < total - 1;

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const tag = (event.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      if (event.key === "ArrowLeft" && canPrev) {
        event.preventDefault();
        setIndex((value) => Math.max(0, value - 1));
      } else if (event.key === "ArrowRight" && canNext) {
        event.preventDefault();
        setIndex((value) => Math.min(total - 1, value + 1));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [canNext, canPrev, total]);

  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const fit = () => {
      const sw = Math.max(0, el.clientWidth - STAGE_PAD);
      const sh = Math.max(0, el.clientHeight - STAGE_PAD);
      setScale(Math.min(sw / SLIDE_W, sh / SLIDE_H) || 1);
    };
    fit();
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <div className="html-immersive">
      <div className="html-immersive__stage" ref={stageRef}>
        {waitingForSlides ? (
          <p className="html-immersive__count m-0 p-6">{t("common.loading")}</p>
        ) : (
          <div
            className="html-immersive__scaler"
            style={{
              width: SLIDE_W,
              height: SLIDE_H,
              transform: `translate(-50%, -50%) scale(${scale})`,
            }}
          >
            <iframe
              key={previewUrl}
              src={previewUrl}
              title={basename(currentPath)}
              className="html-immersive__iframe"
              sandbox="allow-scripts allow-same-origin allow-popups"
            />
          </div>
        )}
      </div>
      <div className="html-immersive__bar">
        {onShowFiles ? (
          <button
            type="button"
            className="html-immersive__btn"
            onClick={onShowFiles}
            title={t("workbench.tabFiles")}
          >
            <Icon name="folder" size={16} />
            {filesVisible ? t("workbench.hideFiles") : t("workbench.showFiles")}
          </button>
        ) : null}
        {total > 1 ? (
          <>
            <button
              type="button"
              className="html-immersive__btn"
              disabled={!canPrev}
              onClick={() => setIndex((value) => Math.max(0, value - 1))}
            >
              <Icon name="chevron_left" size={16} />
              {t("common.previous")}
            </button>
            <span className="html-immersive__count">
              {index + 1} / {total}
            </span>
            <button
              type="button"
              className="html-immersive__btn"
              disabled={!canNext}
              onClick={() => setIndex((value) => Math.min(total - 1, value + 1))}
            >
              {t("common.next")}
              <Icon name="chevron_right" size={16} />
            </button>
          </>
        ) : (
          <span className="html-immersive__count truncate">{basename(currentPath)}</span>
        )}
        <span className="html-immersive__spacer" />
        <DeliverableFileActions
          path={openPath}
          openPath={openPath}
          projectId={projectId}
          projectRoot={projectRoot}
          downloadUrl={downloadUrl}
          downloadName={fileName}
          compact
        />
      </div>
    </div>
  );
}
