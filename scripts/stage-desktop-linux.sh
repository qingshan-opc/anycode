#!/usr/bin/env bash
# Stage Linux AppImage (+ updater signature) into account-portal/public/downloads.
#
# Linux bundles are produced by CI (desktop-release.yml `desktop-linux` job,
# deb+appimage). Download the `anycode-desktop-linux` CI artifact, then:
#
#   ./scripts/stage-desktop-linux.sh /path/to/anyCode_0.40.0_amd64.AppImage \
#                                    /path/to/anyCode_0.40.0_amd64.AppImage.sig
#
# With no args the script auto-locates the newest AppImage under
# target/release/bundle/appimage/ (local Linux build).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

VERSION="$(awk '/^\[workspace\.package\]/{f=1;next} /^\[/{f=0} f&&/^version/{gsub(/.*version = "/,""); gsub(/".*/,""); print; exit}' Cargo.toml)"
DOWNLOAD_DIR="${ANYCODE_DOWNLOAD_DIR:-$ROOT/crates/account-portal/public/downloads}"
mkdir -p "$DOWNLOAD_DIR"

APPIMAGE="${1:-}"
SIG="${2:-}"

if [[ -z "$APPIMAGE" ]]; then
  APPIMAGE="$(ls -1t "$ROOT"/target/release/bundle/appimage/*.AppImage 2>/dev/null | head -1 || true)"
fi
if [[ -z "$SIG" && -n "$APPIMAGE" && -f "$APPIMAGE.sig" ]]; then
  SIG="$APPIMAGE.sig"
fi

if [[ -z "$APPIMAGE" || ! -f "$APPIMAGE" ]]; then
  echo "No AppImage found. Pass the CI artifact path explicitly:" >&2
  echo "  ./scripts/stage-desktop-linux.sh <AppImage> [AppImage.sig]" >&2
  exit 1
fi

DEST="$DOWNLOAD_DIR/anyCode_${VERSION}_x86_64.AppImage"
cp -f "$APPIMAGE" "$DEST"
cp -f "$APPIMAGE" "$DOWNLOAD_DIR/anyCode_latest_x86_64.AppImage"
echo "Staged AppImage: $DEST"

UPDATE_DIR="$DOWNLOAD_DIR/update"
mkdir -p "$UPDATE_DIR"
if [[ -n "$SIG" && -f "$SIG" ]]; then
  cp -f "$SIG" "$UPDATE_DIR/anyCode_${VERSION}_x86_64.AppImage.sig"
  echo "Staged AppImage sig: $UPDATE_DIR/anyCode_${VERSION}_x86_64.AppImage.sig"
else
  echo "WARNING: AppImage .sig missing — update manifest will skip linux-x86_64" >&2
fi

python3 "$ROOT/scripts/lib/regen-downloads-manifest.py" "$DOWNLOAD_DIR"
echo "Updated $DOWNLOAD_DIR/releases.json"
