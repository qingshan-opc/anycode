import type { DeliverableCardProps } from "@/components/deliverables/DeliverableCard";
import {
  markerShouldInline,
  parseArtifactMarkers,
  type ParsedArtifactMarker,
} from "@/lib/artifactMarker";
import { isProcessArtifactPath } from "@/lib/deliverablePath";
import { type TurnReplyItem } from "@/lib/transcriptGrouping";
import type { TranscriptBlock } from "@/api/types";

// 申报制：对话内交付物卡片只来自显式声明——
// 后端持久化的 deliverable blocks，以及助手正文里的 `ANYCODE_ARTIFACT:{...}` marker。
// FileWrite/Edit 写的文件、散文里提到的路径一律不推断（代码不会成为交付物）。

type ReplyItem = TurnReplyItem;

function deliverablePropsFromPath(
  marker: ParsedArtifactMarker,
  projectId?: string,
): DeliverableCardProps {
  return {
    path: marker.path,
    title: marker.title,
    kind: marker.kind,
    mime: marker.mime,
    projectId,
    previewPath: marker.previewPath,
    bytes: marker.bytes,
  };
}

function deliverablePathFromBlock(block: TranscriptBlock): string | null {
  const meta = block.meta ?? {};
  const path = typeof meta.path === "string" ? meta.path.trim() : "";
  return path || null;
}

/** Collect inline deliverables from explicit artifact markers in assistant messages. */
export function collectInlineDeliverables(
  replyItems: ReplyItem[],
  projectId?: string,
): Map<string, DeliverableCardProps[]> {
  const existingBasenames = new Set<string>();
  for (const item of replyItems) {
    if (item.kind !== "block" || item.block.block_type !== "deliverable") continue;
    const path = deliverablePathFromBlock(item.block);
    if (!path) continue;
    existingBasenames.add(path.split(/[/\\]/).pop()?.toLowerCase() ?? path.toLowerCase());
  }

  const byBlockId = new Map<string, DeliverableCardProps[]>();
  const push = (blockId: string, marker: ParsedArtifactMarker) => {
    if (isProcessArtifactPath(marker.path)) return;
    const base = marker.path.split(/[/\\]/).pop()?.toLowerCase() ?? marker.path.toLowerCase();
    if (existingBasenames.has(base)) return;
    existingBasenames.add(base);
    const list = byBlockId.get(blockId) ?? [];
    list.push(deliverablePropsFromPath(marker, projectId));
    byBlockId.set(blockId, list);
  };

  for (const item of replyItems) {
    if (item.kind !== "block" || item.block.block_type !== "assistant_message") continue;
    for (const marker of parseArtifactMarkers(item.block.body).filter(markerShouldInline)) {
      push(item.block.id, marker);
    }
  }

  return byBlockId;
}
