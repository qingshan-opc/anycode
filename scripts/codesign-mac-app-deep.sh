#!/usr/bin/env bash
# Deep-sign all Mach-O binaries inside anyCode.app (Playwright Chromium, ffmpeg, etc.)
set -euo pipefail

APP="${1:-}"
IDENTITY="${APPLE_SIGNING_IDENTITY:-}"

if [[ -z "$APP" || ! -d "$APP" ]]; then
  echo "usage: codesign-mac-app-deep.sh /path/to/anyCode.app" >&2
  exit 1
fi
if [[ -z "$IDENTITY" ]]; then
  IDENTITY="$(security find-identity -v -p codesigning | awk -F'"' '/Developer ID Application/ { print $2; exit }')"
  export APPLE_SIGNING_IDENTITY="$IDENTITY"
fi
if [[ -z "$IDENTITY" ]]; then
  echo "APPLE_SIGNING_IDENTITY not set" >&2
  exit 1
fi

echo "==> deep sign nested binaries in $APP"
xattr -cr "$APP" 2>/dev/null || true

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$SCRIPT_DIR/../apps/anycode-desktop"
MAIN_ENTITLEMENTS="$DESKTOP_DIR/Entitlements.plist"
HELPER_ENTITLEMENTS="$DESKTOP_DIR/Entitlements.helper.plist"

is_macho() {
  file -b "$1" 2>/dev/null | grep -qE 'Mach-O|executable'
}

# Innermost files first (dylibs, helpers, executables)
while IFS= read -r -d '' f; do
  if is_macho "$f"; then
    codesign --force --options runtime --timestamp --sign "$IDENTITY" "$f" 2>/dev/null || true
  fi
done < <(find "$APP" -type f \( -name '*.dylib' -o -name '*.so' -o -perm -111 \) -print0)

# Nested .app bundles (CEF helpers, Chrome for Testing, etc.) — deepest first.
# CEF Helper*.app get the helper entitlements: hardened runtime without
# allow-jit kills every renderer (V8 "Failed to reserve virtual memory for
# CodeRange" → blank embedded view).
while IFS= read -r nested; do
  [[ "$nested" == "$APP" ]] && continue
  ENT_ARGS=()
  case "$(basename "$nested")" in
    "anyCode Helper"*.app)
      [[ -f "$HELPER_ENTITLEMENTS" ]] && ENT_ARGS=(--entitlements "$HELPER_ENTITLEMENTS")
      ;;
  esac
  codesign --force --options runtime --timestamp "${ENT_ARGS[@]}" --sign "$IDENTITY" "$nested" 2>/dev/null || true
done < <(find "$APP" -name '*.app' -type d | awk '{ print length, $0 }' | sort -rn | cut -d' ' -f2-)

# Main app: re-apply its declared entitlements (mic etc.) — signing without
# --entitlements here would strip what tauri.conf.json declares.
MAIN_ENT_ARGS=()
[[ -f "$MAIN_ENTITLEMENTS" ]] && MAIN_ENT_ARGS=(--entitlements "$MAIN_ENTITLEMENTS")
codesign --force --options runtime --timestamp "${MAIN_ENT_ARGS[@]}" --sign "$IDENTITY" "$APP"
echo "==> verify codesign"
codesign --verify --deep --strict --verbose=2 "$APP"
echo "deep sign OK"
