#!/usr/bin/env python3
"""Scan account-portal/public/downloads and write releases.json + latest.json + SHA256SUMS.txt."""

from __future__ import annotations

import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

BASE_URL = "https://anycode.work/downloads"

# anyCode_0.40.0_aarch64.dmg | anyCode_0.40.0_x86_64.dmg | anyCode_0.40.0_x64.msi|exe
# | anyCode_0.40.0_x86_64.AppImage
VERSIONED_RE = re.compile(
    r"^anyCode_(?P<version>\d+\.\d+\.\d+)_(?P<arch>aarch64|x86_64|x64)\.(?P<ext>dmg|msi|exe|AppImage)$"
)
LATEST_RE = re.compile(
    r"^anyCode_latest_(?P<arch>aarch64|x86_64|x64)\.(?P<ext>dmg|msi|exe|AppImage)$"
)

ARCH_TO_PLATFORM = {
    ("aarch64", "dmg"): "macos-aarch64",
    ("x86_64", "dmg"): "macos-x86_64",
    ("x64", "msi"): "windows-x64",
    ("x64", "exe"): "windows-x64",
    ("x86_64", "AppImage"): "linux-x86_64",
}

# Retention: keep only the newest N versions per platform (flat installers and
# their updater counterparts under update/). latest_* pointers are untouched.
KEEP_VERSIONS = 3

# Updater artifacts live under update/ and follow these names:
#   anyCode_<ver>_<arch>.app.tar.gz(+.sig)   -> darwin-<arch>
#   anyCode_<ver>_x64-setup.exe.sig          -> windows-x86_64 (exe itself stays flat)
#   anyCode_<ver>_x86_64.AppImage.sig        -> linux-x86_64   (AppImage stays flat)
UPDATE_TARBALL_RE = re.compile(
    r"^anyCode_(?P<version>\d+\.\d+\.\d+)_(?P<arch>aarch64|x86_64)\.app\.tar\.gz$"
)


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def platform_for(arch: str, ext: str) -> str | None:
    return ARCH_TO_PLATFORM.get((arch, ext))


def version_key(v: str) -> tuple[int, ...]:
    return tuple(int(x) for x in v.split("."))


def prune_old_versions(download_dir: Path) -> None:
    """Keep only the newest KEEP_VERSIONS versions per platform; delete older
    flat installers and their updater counterparts under update/."""
    by_platform: dict[str, set[str]] = {}
    for path in download_dir.iterdir():
        if not path.is_file():
            continue
        m = VERSIONED_RE.match(path.name)
        if not m:
            continue
        platform = platform_for(m.group("arch"), m.group("ext"))
        if platform:
            by_platform.setdefault(platform, set()).add(m.group("version"))

    stale: dict[str, set[str]] = {}
    for platform, versions in by_platform.items():
        ordered = sorted(versions, key=version_key, reverse=True)
        old = set(ordered[KEEP_VERSIONS:])
        if old:
            stale[platform] = old
    if not stale:
        return

    for path in download_dir.iterdir():
        if not path.is_file():
            continue
        m = VERSIONED_RE.match(path.name)
        if not m:
            continue
        platform = platform_for(m.group("arch"), m.group("ext"))
        if platform and m.group("version") in stale.get(platform, set()):
            print(f"prune: {path.name}")
            path.unlink()

    update_dir = download_dir / "update"
    if update_dir.is_dir():
        for path in update_dir.iterdir():
            if not path.is_file():
                continue
            name = path.name.removesuffix(".sig")
            version: str | None = None
            platform: str | None = None
            m = UPDATE_TARBALL_RE.match(name)
            if m:
                version, platform = m.group("version"), f"macos-{m.group('arch')}"
            elif re.match(r"^anyCode_\d+\.\d+\.\d+_x64-setup\.exe$", name):
                version = name.split("_")[1]
                platform = "windows-x64"
            elif re.match(r"^anyCode_\d+\.\d+\.\d+_x86_64\.AppImage$", name):
                version = name.split("_")[1]
                platform = "linux-x86_64"
            if platform and version and version in stale.get(platform, set()):
                print(f"prune: update/{path.name}")
                path.unlink()


def write_tauri_update_manifest(download_dir: Path) -> None:
    """Emit update/latest.json in the Tauri updater schema:
    {version, notes, pub_date, platforms: {<target>: {signature, url}}}."""
    update_dir = download_dir / "update"
    if not update_dir.is_dir():
        return

    # target -> (version_tuple, payload, mtime)
    best: dict[str, tuple[tuple[int, ...], dict, float]] = {}

    def consider(target: str, version: str, payload: dict, mtime: float) -> None:
        key = version_key(version)
        prev = best.get(target)
        if prev is None or key > prev[0]:
            best[target] = (key, payload, mtime)

    def sig_text(artifact: Path) -> str | None:
        sig = artifact.with_name(artifact.name + ".sig")
        if not sig.is_file():
            return None
        return sig.read_text(encoding="utf-8").strip()

    for path in sorted(update_dir.iterdir()):
        if not path.is_file():
            continue
        name = path.name
        if name.endswith(".sig") or name == "latest.json":
            continue
        m = UPDATE_TARBALL_RE.match(name)
        if m:
            signature = sig_text(path)
            if not signature:
                print(f"warn: {name} has no .sig — skipped in update manifest")
                continue
            consider(
                f"darwin-{m.group('arch')}",
                m.group("version"),
                {
                    "signature": signature,
                    "url": f"{BASE_URL}/update/{name}",
                },
                path.stat().st_mtime,
            )

    # Windows / Linux: signature-only entries under update/, artifact stays flat.
    for sig in sorted(update_dir.glob("*.sig")):
        base = sig.name[: -len(".sig")]
        m = re.match(r"^anyCode_(?P<version>\d+\.\d+\.\d+)_x64-setup\.exe$", base)
        if m:
            flat = download_dir / f"anyCode_{m.group('version')}_x64.exe"
            if flat.is_file():
                consider(
                    "windows-x86_64",
                    m.group("version"),
                    {
                        "signature": sig.read_text(encoding="utf-8").strip(),
                        "url": f"{BASE_URL}/{flat.name}",
                    },
                    sig.stat().st_mtime,
                )
            continue
        m = re.match(r"^anyCode_(?P<version>\d+\.\d+\.\d+)_x86_64\.AppImage$", base)
        if m:
            flat = download_dir / f"anyCode_{m.group('version')}_x86_64.AppImage"
            if flat.is_file():
                consider(
                    "linux-x86_64",
                    m.group("version"),
                    {
                        "signature": sig.read_text(encoding="utf-8").strip(),
                        "url": f"{BASE_URL}/{flat.name}",
                    },
                    sig.stat().st_mtime,
                )

    if not best:
        return

    platforms = {target: payload for target, (_, payload, _) in best.items()}
    newest_mtime = max(mtime for _, _, mtime in best.values())
    versions = [key for key, _, _ in best.values()]
    version_labels = [".".join(str(n) for n in v) for v in versions]
    if len(set(version_labels)) > 1:
        print(
            "warn: updater platforms have mixed versions "
            f"{sorted(set(version_labels))}; top-level version is the newest"
        )
    manifest = {
        "version": max(version_labels, key=version_key),
        "notes": "",
        "pub_date": datetime.fromtimestamp(newest_mtime, timezone.utc).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
        "platforms": platforms,
    }
    out = update_dir / "latest.json"
    out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {out} ({', '.join(sorted(platforms))})")


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: regen-downloads-manifest.py <downloads_dir>", file=sys.stderr)
        return 2
    download_dir = Path(sys.argv[1])
    download_dir.mkdir(parents=True, exist_ok=True)

    prune_old_versions(download_dir)

    artifacts: list[dict] = []
    checksum_lines: list[str] = []

    for path in sorted(download_dir.iterdir()):
        if not path.is_file():
            continue
        name = path.name
        if name in {"latest.json", "releases.json", "SHA256SUMS.txt", ".gitkeep"}:
            continue
        if name.startswith("."):
            continue

        m = VERSIONED_RE.match(name)
        if not m:
            # keep latest_* in checksums but not as historical artifacts
            if LATEST_RE.match(name):
                digest = sha256_file(path)
                checksum_lines.append(f"{digest}  {name}")
            continue

        arch = m.group("arch")
        ext = m.group("ext")
        version = m.group("version")
        platform = platform_for(arch, ext)
        if not platform:
            continue

        digest = sha256_file(path)
        checksum_lines.append(f"{digest}  {name}")
        latest_name = f"anyCode_latest_{arch}.{ext}"
        artifacts.append(
            {
                "platform": platform,
                "version": version,
                "arch": arch,
                "filename": name,
                "url": f"{BASE_URL}/{name}",
                "latest_url": f"{BASE_URL}/{latest_name}",
                "sha256": digest,
                "ext": ext,
            }
        )

    # Prefer .msi over .exe for the same version on windows when both exist
    by_key: dict[tuple[str, str], dict] = {}
    for art in artifacts:
        key = (art["platform"], art["version"])
        prev = by_key.get(key)
        if prev is None:
            by_key[key] = art
            continue
        if prev["ext"] == "exe" and art["ext"] == "msi":
            by_key[key] = art

    artifacts = sorted(
        by_key.values(),
        key=lambda a: (a["platform"], version_key(a["version"])),
        reverse=True,
    )

    latest_by_platform: dict[str, str] = {}
    for art in artifacts:
        plat = art["platform"]
        if plat not in latest_by_platform:
            latest_by_platform[plat] = art["version"]
        art["latest"] = art["version"] == latest_by_platform.get(plat)

    platforms: dict[str, dict] = {}
    for plat, ver in latest_by_platform.items():
        art = next(a for a in artifacts if a["platform"] == plat and a["version"] == ver)
        platforms[plat] = {
            "version": art["version"],
            "arch": art["arch"],
            "filename": art["filename"],
            "url": art["url"],
            "latest_url": art["latest_url"],
            "sha256": art["sha256"],
            "ext": art["ext"],
        }

    releases = {
        "generated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "latest_by_platform": latest_by_platform,
        "platforms": platforms,
        "artifacts": artifacts,
    }
    (download_dir / "releases.json").write_text(
        json.dumps(releases, indent=2) + "\n", encoding="utf-8"
    )

    # Backward-compatible latest.json (prefer macos-aarch64, else first platform)
    primary = platforms.get("macos-aarch64") or next(iter(platforms.values()), None)
    if primary:
        latest_payload = {
            "version": primary["version"],
            "arch": primary["arch"],
            "filename": primary["filename"],
            "url": primary["url"],
            "latest_url": primary["latest_url"],
            "sha256": primary["sha256"],
            "platforms": platforms,
        }
        (download_dir / "latest.json").write_text(
            json.dumps(latest_payload, indent=2) + "\n", encoding="utf-8"
        )

    checksum_lines = sorted(set(checksum_lines))
    (download_dir / "SHA256SUMS.txt").write_text(
        "\n".join(checksum_lines) + ("\n" if checksum_lines else ""),
        encoding="utf-8",
    )

    write_tauri_update_manifest(download_dir)

    print(f"wrote {download_dir / 'releases.json'} ({len(artifacts)} artifacts)")
    print(f"platforms: {', '.join(sorted(platforms)) or '(none)'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
