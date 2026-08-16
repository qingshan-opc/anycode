#!/usr/bin/env python3
"""Validate slide HTML: skin tokens + main-visual density (anycode-ppt creative mode)."""
from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "brand-kits" / "lib"))
from brand_kit import find_brand_kit, load_pptx_layouts  # noqa: E402

# Legacy layout classes still count as a main visual.
DENSE_VISUAL = (
    "ladder",
    "layer-stack",
    "layer-stack-4",
    "agent-cycle",
    "evo-grid",
    "duo",
    "trio",
    "metrics",
    "timeline",
    "checklist",
    "diagram-box",
    "quote",
)

# Keep lingqi / brand-slop bans; allow gradients / shadows for creative pages.
FORBIDDEN_STYLE = (
    (r"#1[Bb]3[Aa]5[Cc]", "lingqi navy #1B3A5C forbidden — use VisualBrief tokens"),
    (r"#00[Bb]050", "lingqi green #00B050 forbidden — use VisualBrief accent"),
    (r"\blingqi\b", "lingqi brand/footer forbidden unless user explicitly requested lingqi"),
    (r"https?://[^\"'\s]+echarts", "CDN echarts forbidden — copy skill vendor/echarts.min.js"),
)


def validate_slide_style(path: Path, html: str) -> list[str]:
    issues: list[str] = []
    for pat, msg in FORBIDDEN_STYLE:
        if re.search(pat, html, re.I):
            issues.append(f"{path.name}: {msg}")
    if not (re.search(r"--bg\s*:", html, re.I) and re.search(r"--ink\s*:", html, re.I)):
        issues.append(
            f"{path.name}: missing canvas tokens (--bg / --ink) — apply brief.tokens to :root"
        )
    return issues


def slide_type(html: str) -> str:
    m = re.search(r'data-type=["\']([^"\']+)["\']', html, re.I)
    return (m.group(1).lower() if m else "content").lower()


def count_stats(html: str) -> int:
    return len(re.findall(r'class=["\'][^"\']*\bstat\b|class=["\'][^"\']*\bnumber\b', html, re.I))


def count_list_items(html: str) -> int:
    li = len(re.findall(r"<li\b", html, re.I))
    agenda = len(re.findall(r'class=["\'][^"\']*\bitem\b', html, re.I))
    return li + agenda


def count_panels(html: str) -> int:
    return len(
        re.findall(
            r'class=["\'][^"\']*\b(panel|quote|chip|card|cta-box|contact|side|agenda|item|rung|layer-card|cycle-node|cycle-core|mile|check-item|stat)\b',
            html,
            re.I,
        )
    )


def has_main_visual(html: str) -> bool:
    if re.search(r"<svg\b", html, re.I):
        return True
    if re.search(r"<canvas\b", html, re.I):
        return True
    if re.search(r"<img\b", html, re.I):
        return True
    if re.search(r"echarts|id=[\"']chart[\"']|class=[\"'][^\"']*\bchart\b", html, re.I):
        return True
    return any(re.search(rf'class=["\'][^"\']*\b{pat}\b', html, re.I) for pat in DENSE_VISUAL)


def validate_dense_visual(path: Path, html: str) -> list[str]:
    st = slide_type(html)
    if st in ("cover", "section", "closing"):
        if st == "cover" and not (
            has_main_visual(html) or "tag-row" in html or re.search(r"<h1\b", html, re.I)
        ):
            return [f"{path.name}: cover needs a title block or main visual"]
        if st == "section" and count_list_items(html) < 1 and not has_main_visual(html):
            return [f"{path.name}: section needs agenda items or a main visual"]
        if st == "closing" and not has_main_visual(html):
            return [f"{path.name}: closing needs a main visual or summary blocks"]
        return []
    if not has_main_visual(html):
        return [
            f"{path.name}: content slide missing main visual "
            "(need <svg>, <canvas>, <img>, echarts/#chart, or a dense layout class)"
        ]
    return []


def validate_file(path: Path, layouts: dict, *, dense_mode: bool) -> list[str]:
    html = path.read_text(encoding="utf-8")
    issues: list[str] = []
    if dense_mode:
        issues.extend(validate_dense_visual(path, html))
        issues.extend(validate_slide_style(path, html))
        return issues
    # Brand-kit density mode (non anycode-ppt)
    st = slide_type(html)
    rules = (layouts.get("content_density") or {}).get(st) or {}
    if rules:
        li = count_list_items(html)
        stats = count_stats(html)
        panels = count_panels(html)
        if li < rules.get("min_list_items", 0):
            issues.append(f"{path.name}: type={st} needs ≥{rules['min_list_items']} list items, found {li}")
        if stats < rules.get("min_stat_spans", 0):
            issues.append(
                f"{path.name}: type={st} needs ≥{rules['min_stat_spans']} stat/number blocks, found {stats}"
            )
        if panels < rules.get("min_side_panels", 0):
            issues.append(f"{path.name}: type={st} needs ≥{rules['min_side_panels']} panels/cards, found {panels}")
    return issues


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: validate_slide_html.py slides_dir [brand_kit|anycode-ppt]", file=sys.stderr)
        return 1
    src = Path(sys.argv[1]).resolve()
    kit_arg = sys.argv[2] if len(sys.argv) > 2 else "fde-editorial"
    dense_mode = kit_arg == "anycode-ppt"
    brand_name = "fde-editorial" if dense_mode else kit_arg
    kit = find_brand_kit(brand_name)
    layouts = load_pptx_layouts(kit)
    files = sorted(f for f in src.glob("*.html") if f.name.lower() != "index.html")
    if not files and (src / "slides").is_dir():
        files = sorted(f for f in (src / "slides").glob("*.html") if f.name.lower() != "index.html")
    if dense_mode and len(files) < 2:
        print(f"WARN: need ≥2 slides, found {len(files)}", file=sys.stderr)
        return 1
    all_issues: list[str] = []
    for f in files:
        all_issues.extend(validate_file(f, layouts, dense_mode=dense_mode))
    if all_issues:
        for i in all_issues:
            print(f"WARN: {i}", file=sys.stderr)
        return 1
    mode = "anycode-ppt creative density" if dense_mode else "content density"
    print(f"OK: {len(files)} slides pass {mode}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
