#!/usr/bin/env bash
# Optional local DMG/NSIS staging, then slim account+portal image → ACR.
# Installers go to MinIO (`./scripts/upload-desktop-downloads-s3.sh`), never into the image.
#
# Prereqs:
#   - macOS + ~/.anycode/release.env (see scripts/release.env.example) if building DMG
#   - deploy/account-service/.env for local compose (WECHAT_PAY_* + secrets/*.pem)
#   - Production: K8s Secrets anycode-account-secrets + anycode-wechat-certs (see README)
#
# Usage:
#   ./scripts/build-account-image.sh              # slim image only
#   ./scripts/build-account-image.sh --with-dmg   # also build signed DMG first (still not baked)
#   TAG=0.2.4 ./scripts/build-account-image.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# shellcheck source=scripts/lib/build-target.sh
source "$ROOT/scripts/lib/build-target.sh"
export ANYCODE_BUILD_TARGET=cloud
anycode_apply_build_target_exports

WITH_DMG=0
for arg in "$@"; do
  case "$arg" in
    --skip-dmg)
      # kept for compatibility; image never includes installers
      ;;
    --with-dmg) WITH_DMG=1 ;;
    -h|--help)
      sed -n '2,14p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown arg: $arg (try --with-dmg)" >&2
      exit 1
      ;;
  esac
done

if [[ "$WITH_DMG" -eq 1 ]]; then
  if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "DMG build requires macOS." >&2
    exit 1
  fi
  echo "==> signed desktop DMG (stages locally; upload to MinIO separately)"
  "$ROOT/scripts/build-account-portal.sh"
  "$ROOT/scripts/release-desktop-local.sh"
fi

echo "==> docker build + push (account API + portal; no installers)"
exec "$ROOT/deploy/account-service/build-push.sh"
