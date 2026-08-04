/** Host display size in CSS pixels + DPR — used as Chromium layout viewport. */
export function systemBrowserViewport(): {
  width: number;
  height: number;
  device_scale_factor: number;
} {
  if (typeof window === "undefined" || !window.screen) {
    return { width: 1920, height: 1080, device_scale_factor: 1 };
  }
  const width = Math.floor(window.screen.availWidth || window.screen.width || 1920);
  const height = Math.floor(window.screen.availHeight || window.screen.height || 1080);
  const dpr = window.devicePixelRatio || 1;
  return {
    width: Math.max(800, Math.min(3840, width)),
    height: Math.max(600, Math.min(2160, height)),
    // Cap at 2× — enough for Retina sharpness without huge screencast bitmaps.
    device_scale_factor: Math.max(1, Math.min(2, dpr)),
  };
}
