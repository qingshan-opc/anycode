#!/usr/bin/env python3
"""Upload account-portal/public/downloads to MinIO bucket `downloads`."""

from __future__ import annotations

import argparse
import mimetypes
import os
import sys
from pathlib import Path

import boto3
from botocore.client import Config
from botocore.exceptions import ClientError

SKIP_NAMES = {".gitkeep", ".DS_Store"}
BINARY_SUFFIXES = {
    ".dmg",
    ".exe",
    ".msi",
    ".appimage",
    ".gz",
    ".zip",
    ".sig",
}


def content_type_for(path: Path) -> str:
    suffix = path.suffix.lower()
    if suffix == ".dmg":
        return "application/x-apple-diskimage"
    if suffix == ".json":
        return "application/json; charset=utf-8"
    if suffix in {".txt", ".sums"}:
        return "text/plain; charset=utf-8"
    if suffix == ".gz":
        return "application/gzip"
    guessed, _ = mimetypes.guess_type(str(path))
    return guessed or "application/octet-stream"


def cache_control_for(path: Path) -> str:
    suffix = path.suffix.lower()
    if suffix in {".json", ".txt"}:
        return "public, max-age=60"
    if suffix in BINARY_SUFFIXES:
        return "public, max-age=31536000, immutable"
    return "public, max-age=3600"


def iter_files(root: Path) -> list[Path]:
    files: list[Path] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        if path.name in SKIP_NAMES:
            continue
        files.append(path)
    return files


def public_read_policy(bucket: str) -> str:
    return (
        '{"Version":"2012-10-17","Statement":[{'
        '"Effect":"Allow","Principal":{"AWS":["*"]},'
        '"Action":["s3:GetObject"],'
        f'"Resource":["arn:aws:s3:::{bucket}/*"]'
        "}]}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--dir",
        type=Path,
        default=Path(__file__).resolve().parents[1]
        / "crates"
        / "account-portal"
        / "public"
        / "downloads",
    )
    parser.add_argument("--endpoint", default=os.environ.get("S3_ENDPOINT", "http://127.0.0.1:19000"))
    parser.add_argument("--bucket", default=os.environ.get("S3_BUCKET", "downloads"))
    parser.add_argument("--region", default=os.environ.get("S3_REGION", "cn-zhangjiakou"))
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    access = os.environ.get("S3_ACCESS_KEY") or os.environ.get("AWS_ACCESS_KEY_ID")
    secret = os.environ.get("S3_SECRET_KEY") or os.environ.get("AWS_SECRET_ACCESS_KEY")
    if not access or not secret:
        print("S3_ACCESS_KEY / S3_SECRET_KEY missing", file=sys.stderr)
        return 2

    root = args.dir.resolve()
    if not root.is_dir():
        print(f"missing downloads dir: {root}", file=sys.stderr)
        return 2

    files = iter_files(root)
    if not files:
        print(f"no files under {root}", file=sys.stderr)
        return 1

    client = boto3.client(
        "s3",
        endpoint_url=args.endpoint,
        aws_access_key_id=access,
        aws_secret_access_key=secret,
        region_name=args.region,
        config=Config(signature_version="s3v4", s3={"addressing_style": "path"}),
    )

    if not args.dry_run:
        try:
            client.head_bucket(Bucket=args.bucket)
        except ClientError:
            client.create_bucket(Bucket=args.bucket)
        client.put_bucket_policy(Bucket=args.bucket, Policy=public_read_policy(args.bucket))

    uploaded = 0
    for path in files:
        key = path.relative_to(root).as_posix()
        extra = {
            "ContentType": content_type_for(path),
            "CacheControl": cache_control_for(path),
        }
        size = path.stat().st_size
        print(
            f"{'dry-run' if args.dry_run else 'put'} s3://{args.bucket}/{key} ({size} bytes)",
            flush=True,
        )
        if args.dry_run:
            continue
        client.upload_file(str(path), args.bucket, key, ExtraArgs=extra)
        uploaded += 1

    print(f"done: {len(files)} files, uploaded={uploaded}, endpoint={args.endpoint}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
