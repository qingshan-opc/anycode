/** Video attachment state for the composer (roadmap P1.2). */

export type VideoAttachment = {
  video_ref: string;
  name: string;
  duration_secs?: number;
  frame_count?: number;
  has_transcript?: boolean;
};

export const MAX_VIDEO_BYTES = 200 * 1024 * 1024;
export const VIDEO_ACCEPT = "video/mp4,video/quicktime,video/x-m4v,video/webm";

export function isVideoFile(file: File): boolean {
  if (file.type.startsWith("video/")) return true;
  return /\.(mp4|mov|m4v|webm)$/i.test(file.name);
}

export function formatVideoMeta(video: VideoAttachment): string {
  const parts: string[] = [];
  if (typeof video.duration_secs === "number" && video.duration_secs > 0) {
    parts.push(`${Math.round(video.duration_secs)}s`);
  }
  if (typeof video.frame_count === "number" && video.frame_count > 0) {
    parts.push(`${video.frame_count} frames`);
  }
  return parts.join(" · ");
}
