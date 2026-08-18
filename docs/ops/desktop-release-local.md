# macOS desktop release (local)

GitHub Actions desktop builds are **optional / manual only** (slow). Ship DMG from your Mac.

## One-time setup

1. **Developer ID Application** cert in Keychain (not Apple Development).
2. Copy `scripts/release.env.example` → `~/.anycode/release.env` and fill:
   - `APPLE_SIGNING_IDENTITY`
   - `APPLE_TEAM_ID`
   - `APPLE_ID`
   - `APPLE_PASSWORD` (app-specific password)

## Ship a version

```bash
# 1. Bump workspace version in Cargo.toml, then:
./scripts/sync-workspace-version.sh

# 2. Build signed + notarized DMG and stage for portal
chmod +x scripts/release-desktop-local.sh
./scripts/release-desktop-local.sh                 # Apple Silicon (host)
./scripts/release-desktop-local.sh --arch x86_64   # Intel (cross from Apple Silicon)

# Windows NSIS cross-compile from this Mac (experimental; MSI still needs Windows):
./scripts/setup-windows-cross.sh                 # once: nsis + llvm + cargo-xwin
./scripts/release-desktop-windows-cross.sh       # build NSIS + stage

# Or on a Windows host (MSI + NSIS):
./scripts/build-desktop-release.sh && ./scripts/stage-desktop-windows.sh
```

Artifacts land in `crates/account-portal/public/downloads/`:

| File | Purpose |
|------|---------|
| `anyCode_<version>_aarch64.dmg` | macOS Apple Silicon |
| `anyCode_<version>_x86_64.dmg` | macOS Intel |
| `anyCode_<version>_x64.msi` / `.exe` | Windows (MSI = Windows host; `.exe` = NSIS, also via Mac cross) |
| `anyCode_latest_<arch>.dmg` | Stable “latest” link per arch |
| `latest.json` | Latest metadata (+ `platforms` map) |
| `releases.json` | Multi-version / multi-platform catalog |
| `SHA256SUMS.txt` | Checksums for all staged files |

Public URLs (after deploy):

- https://anycode.work/downloads/anyCode_latest_aarch64.dmg
- https://anycode.work/downloads/anyCode_latest_x86_64.dmg
- https://anycode.work/downloads/releases.json

## Deploy to anycode.work (recommended)

Installers live on **cluster MinIO** (same S3 as FDE courses, bucket `downloads`).
Do **not** bake `*.dmg` / `*.exe` / updater tarballs into the account image.

```bash
# 1. Stage artifacts locally (already done by the release scripts above)
# 2. Upload to MinIO (port-forwards svc/minio)
./scripts/upload-desktop-downloads-s3.sh

# 3. Slim account+portal image (no installers)
./scripts/build-account-image.sh
# or: TAG=0.42.4 ./scripts/build-account-image.sh

kubectl -n dis-cloud set image deployment/anycode \
  anycode=registry.cn-zhangjiakou.aliyuncs.com/818cloud/anycode:0.42.4
kubectl -n dis-cloud rollout status deployment/anycode
```

Public URLs stay `https://anycode.work/downloads/...` via Ingress regex `/downloads/.+` → MinIO.

First-time cluster wiring: `kubectl -n dis-cloud apply -f deploy/account-service/k8s/downloads-minio-proxy.yaml`

If downloads 502: ingress-nginx cannot resolve ExternalName; the manifest pins MinIO ClusterIP in Endpoints `minio-downloads`. Refresh with `kubectl -n base-service get svc minio -o jsonpath='{.spec.clusterIP}'`.

## Verify on a clean Mac

```bash
spctl -a -vv -t install target/release/bundle/macos/anyCode.app
# expect: accepted / Notarized Developer ID

xcrun stapler validate crates/account-portal/public/downloads/anyCode_*_aarch64.dmg
spctl -a -t open --context context:primary-signature -vv \
  crates/account-portal/public/downloads/anyCode_*_aarch64.dmg
# expect: accepted / Notarized Developer ID
# An unsigned DMG is rejected as “damaged” after a browser download.
```

The release pipeline signs, notarizes, and staples **both** the `.app` and the DMG (`scripts/notarize-mac-dmg.sh`). Playwright Chromium is still omitted from signed builds; CEF is deep-signed with the app.

`createUpdaterArtifacts` is off until `TAURI_SIGNING_PRIVATE_KEY` is configured; updates ship via new DMG on MinIO.

## GitHub Releases (optional)

If you still want a GitHub asset mirror:

```bash
gh release create v0.2.4 crates/account-portal/public/downloads/anyCode_0.2.4_aarch64.dmg --title v0.2.4
```

Primary download channel remains **anycode.work**.
