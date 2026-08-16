#!/usr/bin/env bash
# Desktop release smoke: prerequisite checks + optional cold-start of a built .app.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

require_bundle="${ANYCODE_SMOKE_REQUIRE_BUNDLE:-0}"

echo "== smoke: daemon binary (scheduler) =="
cargo build --release -p anycode-channel-bridge
test -x target/release/anycode-daemon
target/release/anycode-daemon --version >/dev/null 2>&1 || true

echo "== smoke: desktop app tree =="
test -f apps/anycode-desktop/tauri.conf.json
test -f apps/anycode-desktop/assets/anycode-logo.png
test -f apps/anycode-desktop/assets/anycode-logo-app-icon.png
test -f apps/anycode-desktop/icons/icon.icns
test -f scripts/build-desktop-release.sh

if command -v cargo-tauri >/dev/null 2>&1; then
  echo "== smoke: tauri info =="
  (cd apps/anycode-desktop && cargo tauri info >/dev/null)
else
  echo "skip: cargo-tauri not installed (install via 'cargo install tauri-cli' for full desktop build)"
fi

find_desktop_app() {
  local candidates=(
    "$ROOT/target/release/bundle/macos/anyCode.app"
    "$ROOT/target/release-local/bundle/macos/anyCode.app"
    "/Applications/anyCode.app"
  )
  if [[ -n "${ANYCODE_TAURI_TARGET:-}" ]]; then
    candidates+=(
      "$ROOT/target/${ANYCODE_TAURI_TARGET}/release/bundle/macos/anyCode.app"
      "$ROOT/target/${ANYCODE_TAURI_TARGET}/release-local/bundle/macos/anyCode.app"
    )
  fi
  local c
  for c in "${candidates[@]}"; do
    if [[ -d "$c/Contents/MacOS" ]]; then
      printf '%s\n' "$c"
      return 0
    fi
  done
  return 1
}

find_desktop_dmg() {
  local dmg
  for dmg in "$ROOT"/target/release/bundle/dmg/anyCode_*.dmg \
    "$ROOT"/target/release-local/bundle/dmg/anyCode_*.dmg; do
    if [[ -f "$dmg" ]]; then
      printf '%s\n' "$dmg"
      return 0
    fi
  done
  return 1
}

wait_for_pid() {
  local deadline=$((SECONDS + 45))
  while ((SECONDS < deadline)); do
    local pid
    pid="$(pgrep -x anycode-desktop 2>/dev/null | head -1 || true)"
    if [[ -z "$pid" ]]; then
      pid="$(pgrep -x anyCode 2>/dev/null | head -1 || true)"
    fi
    if [[ -n "$pid" ]]; then
      printf '%s\n' "$pid"
      return 0
    fi
    sleep 0.5
  done
  return 1
}

discover_listen_port() {
  local pid="$1"
  if ! command -v lsof >/dev/null 2>&1; then
    return 1
  fi
  lsof -Pan -p "$pid" -iTCP -sTCP:LISTEN 2>/dev/null \
    | awk '/127\.0\.0\.1/ { split($9, a, ":"); print a[2]; exit }'
}

curl_health_ok() {
  local port="$1"
  local body
  body="$(curl -fsS --max-time 3 "http://127.0.0.1:${port}/api/health" 2>/dev/null || true)"
  [[ "$body" == *'"ok":true'* || "$body" == *'"ok": true'* ]]
}

osascript_window_visible() {
  local out
  out="$(osascript <<'APPLESCRIPT' 2>/dev/null || true)
tell application "System Events"
  if exists process "anyCode" then
    tell process "anyCode"
      if (count of windows) > 0 then return "true"
    end tell
  end if
  if exists process "anycode-desktop" then
    tell process "anycode-desktop"
      if (count of windows) > 0 then return "true"
    end tell
  end if
end tell
return "false"
APPLESCRIPT
)"
  [[ "$out" == "true" ]]
}

quit_desktop_app() {
  osascript -e 'tell application "anyCode" to quit' 2>/dev/null || true
  local deadline=$((SECONDS + 15))
  while pgrep -x anycode-desktop >/dev/null 2>&1 && ((SECONDS < deadline)); do
    sleep 0.5
  done
  while pgrep -x anyCode >/dev/null 2>&1 && ((SECONDS < deadline)); do
    sleep 0.5
  done
  if pgrep -x anycode-desktop >/dev/null 2>&1 || pgrep -x anyCode >/dev/null 2>&1; then
    pkill -x anycode-desktop 2>/dev/null || true
    pkill -x anyCode 2>/dev/null || true
  fi
}

APP_BUNDLE=""
if APP_BUNDLE="$(find_desktop_app)"; then
  :
elif DMG="$(find_desktop_dmg)"; then
  echo "note: found DMG ($DMG) but no .app bundle — mount/build .app for cold-start smoke"
  APP_BUNDLE=""
else
  APP_BUNDLE=""
fi

if [[ -z "$APP_BUNDLE" ]]; then
  if [[ "$require_bundle" == "1" ]]; then
    echo "error: ANYCODE_SMOKE_REQUIRE_BUNDLE=1 but no anyCode.app under target/release/bundle or /Applications" >&2
    exit 1
  fi
  echo "skip: no built anyCode.app — run ./scripts/build-desktop-release.sh (or sync-desktop-dev.sh) for cold-start smoke"
  echo "desktop release smoke: ok (prerequisites only)"
  exit 0
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "skip: cold-start .app smoke is macOS-only (found $APP_BUNDLE)"
  echo "desktop release smoke: ok"
  exit 0
fi

echo "== smoke: cold-start desktop app =="
echo "    bundle: $APP_BUNDLE"

# Ensure a clean slate before launch.
quit_desktop_app

open -n "$APP_BUNDLE" --args --smoke-launch 2>/dev/null || open -n "$APP_BUNDLE"

PID=""
if ! PID="$(wait_for_pid)"; then
  echo "error: anycode-desktop process did not appear within 45s" >&2
  quit_desktop_app
  exit 1
fi
echo "    pid: $PID"

deadline=$((SECONDS + 60))
health_ok=0
window_ok=0
while ((SECONDS < deadline)); do
  if [[ "$window_ok" -eq 0 ]] && osascript_window_visible; then
    window_ok=1
    echo "    window: visible"
  fi
  if [[ "$health_ok" -eq 0 ]]; then
    port="$(discover_listen_port "$PID" || true)"
    if [[ -n "${port:-}" ]] && curl_health_ok "$port"; then
      health_ok=1
      echo "    health: ok (http://127.0.0.1:${port}/api/health)"
    fi
  fi
  if [[ "$health_ok" -eq 1 || "$window_ok" -eq 1 ]]; then
    break
  fi
  sleep 0.5
done

quit_desktop_app

if [[ "$health_ok" -eq 0 && "$window_ok" -eq 0 ]]; then
  echo "error: desktop cold-start failed — no /api/health and no visible window" >&2
  exit 1
fi

if [[ "$health_ok" -eq 0 ]]; then
  echo "    health: skip (ephemeral port not discovered; window check passed)"
fi
if [[ "$window_ok" -eq 0 ]]; then
  echo "    window: skip (health check passed)"
fi

echo "desktop release smoke: ok"
