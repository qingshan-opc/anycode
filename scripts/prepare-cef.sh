#!/usr/bin/env bash
# Stage Chromium Embedded Framework + Helper apps into anyCode.app (macOS).
#
# Prerequisites:
#   brew install ninja cmake
#   cargo install export-cef-dir   # once
#   export-cef-dir --force "$HOME/.local/share/cef"
#
# Usage:
#   ./scripts/prepare-cef.sh
#   ./scripts/prepare-cef.sh /Applications/anyCode.app
#
# Env:
#   CEF_PATH                 default: $HOME/.local/share/cef
#   ANYCODE_DESKTOP_PROFILE  default: release-local
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL="${1:-${ANYCODE_DESKTOP_INSTALL:-/Applications/anyCode.app}}"
CEF_PATH="${CEF_PATH:-$HOME/.local/share/cef}"
PROFILE="${ANYCODE_DESKTOP_PROFILE:-release-local}"
DESKTOP_TARGET="$ROOT/apps/anycode-desktop/target"
HELPER_BIN_NAME="anycode-cef-helper"
APP_HELPER_NAME="anyCode Helper"
FW_DST="$INSTALL/Contents/Frameworks"
CEF_FW_SRC="$CEF_PATH/Chromium Embedded Framework.framework"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "prepare-cef.sh is macOS-only (skipping)" >&2
  exit 0
fi

if [[ ! -d "$CEF_FW_SRC" ]]; then
  echo "CEF framework missing at $CEF_FW_SRC" >&2
  echo "Install: cargo install export-cef-dir && export-cef-dir --force \"\$HOME/.local/share/cef\"" >&2
  exit 1
fi

if [[ ! -d "$INSTALL/Contents" ]]; then
  echo "missing app bundle: $INSTALL" >&2
  echo "Build once: ./scripts/build-desktop-local.sh or ./scripts/sync-desktop-dev.sh --rust" >&2
  exit 1
fi

step() {
  local label="$1"
  shift
  local start=$SECONDS
  echo "==> $label"
  "$@"
  echo "    ($((SECONDS - start))s)"
}

write_helper_plist() {
  local plist="$1"
  local bundle_name="$2"
  local exe_name="$3"
  local identifier="$4"
  cat >"$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>${exe_name}</string>
  <key>CFBundleIdentifier</key>
  <string>${identifier}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>${bundle_name}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.40.0</string>
  <key>CFBundleVersion</key>
  <string>0.40.0</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key>
  <true/>
</dict>
</plist>
EOF
}

# Helper is built from the root workspace (release profile — nested desktop
# workspace does not own anycode-browser-cef).
step "build CEF helper (release)" bash -ec "
  export CEF_PATH=\"$CEF_PATH\"
  export DYLD_FALLBACK_LIBRARY_PATH=\"\${DYLD_FALLBACK_LIBRARY_PATH:-}:$CEF_PATH:$CEF_PATH/Chromium Embedded Framework.framework/Libraries\"
  cargo build --release -p anycode-browser-cef --features helper --bin anycode-cef-helper --manifest-path \"$ROOT/Cargo.toml\"
"

HELPER_SRC=""
for cand in \
  "$ROOT/target/release/$HELPER_BIN_NAME" \
  "$ROOT/target/$PROFILE/$HELPER_BIN_NAME" \
  "$DESKTOP_TARGET/$PROFILE/$HELPER_BIN_NAME"
do
  if [[ -x "$cand" ]]; then
    HELPER_SRC="$cand"
    break
  fi
done

if [[ -z "$HELPER_SRC" ]]; then
  HELPER_SRC="$(find "$ROOT/target" -name "$HELPER_BIN_NAME" -type f 2>/dev/null | head -1 || true)"
fi

if [[ -z "$HELPER_SRC" || ! -x "$HELPER_SRC" ]]; then
  echo "helper binary not found after build" >&2
  exit 1
fi
echo "    helper: $HELPER_SRC"

step "install CEF framework into app" bash -ec "
  mkdir -p \"$FW_DST\"
  rsync -a --delete \"$CEF_FW_SRC/\" \"$FW_DST/Chromium Embedded Framework.framework/\"
"

echo "==> install CEF Helper apps"
for suffix in "" " (GPU)" " (Plugin)" " (Renderer)"; do
  display="${APP_HELPER_NAME}${suffix}"
  app_dir="$FW_DST/${display}.app"
  macos_dir="$app_dir/Contents/MacOS"
  mkdir -p "$macos_dir"
  cp -f "$HELPER_SRC" "$macos_dir/$display"
  chmod +x "$macos_dir/$display"
  id="work.anycode.desktop.helper${suffix}"
  id="$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9._-')"
  write_helper_plist "$app_dir/Contents/Info.plist" "$display" "$display" "$id"
done

cp -f "$HELPER_SRC" "$INSTALL/Contents/MacOS/$HELPER_BIN_NAME"
chmod +x "$INSTALL/Contents/MacOS/$HELPER_BIN_NAME"

# Clear quarantine / Finder xattrs that break codesign, then ad-hoc sign for local runs.
if command -v xattr >/dev/null 2>&1; then
  xattr -cr "$FW_DST" 2>/dev/null || true
  xattr -cr "$INSTALL/Contents/MacOS/$HELPER_BIN_NAME" 2>/dev/null || true
  xattr -cr "$INSTALL/Contents/MacOS/anycode-desktop" 2>/dev/null || true
fi

if command -v codesign >/dev/null 2>&1; then
  echo "==> ad-hoc codesign CEF + helpers (local unsigned)"
  codesign --force --sign - --timestamp=none \
    "$FW_DST/Chromium Embedded Framework.framework/Chromium Embedded Framework" 2>/dev/null || true
  codesign --force --sign - --timestamp=none \
    "$FW_DST/Chromium Embedded Framework.framework" 2>/dev/null || true
  for suffix in "" " (GPU)" " (Plugin)" " (Renderer)"; do
    display="${APP_HELPER_NAME}${suffix}"
    codesign --force --sign - --timestamp=none \
      "$FW_DST/${display}.app/Contents/MacOS/${display}" 2>/dev/null || true
    codesign --force --sign - --timestamp=none \
      "$FW_DST/${display}.app" 2>/dev/null || true
  done
  codesign --force --sign - --timestamp=none \
    "$INSTALL/Contents/MacOS/$HELPER_BIN_NAME" 2>/dev/null || true
  codesign --force --sign - --timestamp=none \
    "$INSTALL/Contents/MacOS/anycode-desktop" 2>/dev/null || true
  codesign --force --deep --sign - --timestamp=none "$INSTALL" 2>/dev/null || true
fi

SIZE="$(du -sh "$FW_DST/Chromium Embedded Framework.framework" | awk '{print $1}')"
echo "Done. CEF Framework ≈ $SIZE in $FW_DST"
echo "  Helper: $FW_DST/${APP_HELPER_NAME}.app"
echo "  Note: desktop DMG/app size grows by hundreds of MB with CEF."
