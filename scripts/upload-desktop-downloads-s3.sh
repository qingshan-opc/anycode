#!/usr/bin/env bash
# Upload staged desktop artifacts to cluster MinIO (bucket `downloads`).
# Credentials: env, ~/.anycode/release.env, or FDE deploy/k8s/818cloud/.env
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DOWNLOAD_DIR="${ANYCODE_DOWNLOAD_DIR:-$ROOT/crates/account-portal/public/downloads}"
FDE_ENV="${FDE_S3_ENV:-$HOME/workspace/research/digital-fde-platform/deploy/k8s/818cloud/.env}"
LOCAL_S3_PORT="${S3_LOCAL_PORT:-19000}"

if [[ -f "$HOME/.anycode/release.env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.anycode/release.env"
fi
if [[ -z "${S3_ACCESS_KEY:-}" && -f "$FDE_ENV" ]]; then
  # shellcheck disable=SC1090
  set -a
  source "$FDE_ENV"
  set +a
fi
if [[ -z "${S3_ACCESS_KEY:-}" || -z "${S3_SECRET_KEY:-}" ]]; then
  echo "Need S3_ACCESS_KEY and S3_SECRET_KEY (release.env or FDE 818cloud/.env)" >&2
  exit 1
fi

export S3_BUCKET="${S3_BUCKET:-downloads}"
export S3_REGION="${S3_REGION:-cn-zhangjiakou}"
export S3_ENDPOINT="${S3_ENDPOINT:-http://127.0.0.1:${LOCAL_S3_PORT}}"

if [[ "${S3_SKIP_PORT_FORWARD:-0}" != "1" ]]; then
  TUNNEL="${FDE_K8S_TUNNEL:-$HOME/workspace/research/digital-fde-platform/scripts/ensure_k8s_tunnel.sh}"
  if [[ -x "$TUNNEL" ]]; then
    "$TUNNEL"
  fi
  export KUBECONFIG="${KUBECONFIG:-$HOME/.kube/config-fde-818cloud}"
  if ! curl -sf -o /dev/null --max-time 2 "${S3_ENDPOINT}/minio/health/live" 2>/dev/null; then
    echo "==> port-forward minio → 127.0.0.1:${LOCAL_S3_PORT}"
    kubectl -n base-service port-forward svc/minio "${LOCAL_S3_PORT}:9000" >/tmp/anycode-minio-pf.log 2>&1 &
    PF_PID=$!
    cleanup() { kill "$PF_PID" 2>/dev/null || true; }
    trap cleanup EXIT
    for _ in $(seq 1 30); do
      if curl -sf -o /dev/null --max-time 1 "${S3_ENDPOINT}/minio/health/live" 2>/dev/null \
        || curl -sf -o /dev/null --max-time 1 "${S3_ENDPOINT}/minio/health/ready" 2>/dev/null; then
        break
      fi
      sleep 0.4
    done
  fi
fi

PYTHON="${ANYCODE_S3_PYTHON:-}"
if [[ -z "$PYTHON" ]]; then
  for candidate in \
    "$HOME/workspace/research/digital-fde-platform/.venv/bin/python" \
    "$ROOT/.venv-s3/bin/python" \
    python3; do
    if [[ -x "$candidate" ]] && "$candidate" -c "import boto3" >/dev/null 2>&1; then
      PYTHON="$candidate"
      break
    fi
  done
fi
if [[ -z "${PYTHON:-}" ]]; then
  echo "Need Python with boto3 (FDE .venv or python3 -m venv .venv-s3 && pip install boto3)" >&2
  exit 1
fi

echo "==> uploading ${DOWNLOAD_DIR} → ${S3_ENDPOINT} bucket ${S3_BUCKET}"
"$PYTHON" "$ROOT/scripts/upload_desktop_downloads_s3.py" --dir "$DOWNLOAD_DIR" "$@"
