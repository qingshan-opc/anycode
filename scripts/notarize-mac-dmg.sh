#!/usr/bin/env bash
# Sign + notarize + staple a macOS DMG. Gatekeeper treats an unsigned
# downloaded DMG as "damaged" even when the inner .app is notarized.
set -euo pipefail

DMG="${1:-}"
if [[ -z "$DMG" || ! -f "$DMG" ]]; then
  echo "usage: notarize-mac-dmg.sh /path/to/anyCode_VERSION_ARCH.dmg" >&2
  exit 1
fi

REL_ENV="${ANYCODE_RELEASE_ENV:-$HOME/.anycode/release.env}"
if [[ -f "$REL_ENV" ]]; then
  # shellcheck source=/dev/null
  source "$REL_ENV"
fi

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  APPLE_SIGNING_IDENTITY="$(security find-identity -v -p codesigning | awk -F'"' '/Developer ID Application/ { print $2; exit }')"
fi
if [[ -z "${APPLE_PASSWORD:-}" && -n "${APP_SPECIFIC_PASSWORD:-}" ]]; then
  APPLE_PASSWORD="$APP_SPECIFIC_PASSWORD"
fi

for var in APPLE_SIGNING_IDENTITY APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  if [[ -z "${!var:-}" ]]; then
    echo "Set $var for DMG notarization" >&2
    exit 1
  fi
done

if [[ -d /Applications/Xcode.app/Contents/Developer ]]; then
  export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

echo "==> codesign DMG $(basename "$DMG")"
codesign --force --sign "$APPLE_SIGNING_IDENTITY" --timestamp "$DMG"
codesign --verify --verbose=2 "$DMG"

echo "==> notarytool submit DMG"
SUBMIT_OUT="$(mktemp)"
if ! xcrun notarytool submit "$DMG" \
  --apple-id "$APPLE_ID" \
  --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" \
  --wait 2>&1 | tee "$SUBMIT_OUT"; then
  rm -f "$SUBMIT_OUT"
  echo "notarytool submit failed" >&2
  exit 1
fi
if grep -q "status: Invalid" "$SUBMIT_OUT"; then
  rm -f "$SUBMIT_OUT"
  echo "DMG notarization Invalid" >&2
  exit 1
fi
rm -f "$SUBMIT_OUT"

echo "==> staple DMG"
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"
spctl -a -t open --context context:primary-signature -vv "$DMG" 2>&1 | tee /tmp/anycode-spctl-dmg.log || true
echo "DMG notarization OK: $DMG"
