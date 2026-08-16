/**
 * Suppress WKWebView / Chromium default context menu (Back / Reload / Inspect)
 * while allowing editable fields and elements that opt in via data-allow-native-context-menu.
 */
export function initSuppressNativeContextMenu(): void {
  if (typeof document === "undefined") return;
  document.addEventListener(
    "contextmenu",
    (event) => {
      const target = event.target;
      if (!(target instanceof Element)) {
        event.preventDefault();
        return;
      }
      if (target.closest("[data-allow-native-context-menu]")) {
        return;
      }
      if (target.closest("input, textarea, select, [contenteditable='true'], [contenteditable='']")) {
        return;
      }
      event.preventDefault();
    },
    true,
  );
}
