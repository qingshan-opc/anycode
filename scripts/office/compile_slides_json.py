#!/usr/bin/env python3
"""Compile anycode-ppt slides.json → slides/NN-slug.html without LLM writing full HTML.

Usage:
  python compile_slides_json.py path/to/slides.json [out_dir]
"""
from __future__ import annotations

import html
import json
import re
import sys
from pathlib import Path

SKILL_DIR = Path(__file__).resolve().parents[2] / "skills-starter" / "anycode-ppt"
# When invoked from a packaged skill copy, prefer sibling templates/
_LOCAL_SKILL = Path(__file__).resolve().parent.parent
if (_LOCAL_SKILL / "templates").is_dir():
    SKILL_DIR = _LOCAL_SKILL
elif not (SKILL_DIR / "templates").is_dir():
    # Desktop bundled: skills-starter/anycode-ppt next to scripts via repo root
    for parent in Path(__file__).resolve().parents:
        cand = parent / "skills-starter" / "anycode-ppt"
        if (cand / "templates").is_dir():
            SKILL_DIR = cand
            break
        cand2 = parent / "apps" / "anycode-desktop" / "resources" / "skills-starter" / "anycode-ppt"
        if (cand2 / "templates").is_dir():
            SKILL_DIR = cand2
            break

LAYOUT_ALIASES = {
    "cover": "cover",
    "ladder": "ladder-flow",
    "metrics": "metrics-kpi",
    "section": "section",
    "closing": "closing",
    "timeline": "timeline",
    "checklist": "checklist",
    "quote": "quote-insight",
    "trio": "trio-cards",
    "duo": "duo-compare",
}


def resolve_template(layout: str) -> Path:
    name = LAYOUT_ALIASES.get(layout, layout)
    name = name.removesuffix(".html")
    path = SKILL_DIR / "templates" / f"{name}.html"
    if not path.is_file():
        # fallback cover
        path = SKILL_DIR / "templates" / "cover.html"
    return path


def apply_tokens(doc: str, tokens: dict) -> str:
    if not tokens:
        return doc

    def repl_root(m: re.Match[str]) -> str:
        block = m.group(0)
        for key, val in tokens.items():
            if not isinstance(val, str) or not val.strip():
                continue
            css_key = key if key.startswith("--") else f"--{key}"
            # map common short names
            if key in ("bg", "ink", "accent", "serif", "sans", "mono"):
                css_key = f"--{key}"
            pat = re.compile(rf"({re.escape(css_key)}\s*:\s*)([^;}}]+)")
            if pat.search(block):
                block = pat.sub(rf"\g<1>{val}", block, count=1)
            else:
                # inject before closing of first :root
                block = block[:-1] + f"{css_key}:{val};" + "}"
        return block

    return re.sub(r":root\s*\{[^}]*\}", repl_root, doc, count=1)


def fill_text_fields(doc: str, slide: dict) -> str:
    title = slide.get("title") or ""
    subtitle = slide.get("subtitle") or ""
    statement = slide.get("statement") or slide.get("caption") or ""
    bullets = slide.get("bullets") or slide.get("items") or []
    if not isinstance(bullets, list):
        bullets = []

    # <title>
    if title:
        doc = re.sub(
            r"<title>[^<]*</title>",
            f"<title>{html.escape(title)}</title>",
            doc,
            count=1,
        )

    # First h1
    if title:
        h1_inner = html.escape(title)
        if subtitle:
            h1_inner += f'<br><span class="thin">{html.escape(subtitle)}</span>'
        doc = re.sub(r"<h1[^>]*>.*?</h1>", f"<h1>{h1_inner}</h1>", doc, count=1, flags=re.S)

    # .statement / .cap / first <p>
    if statement:
        esc = html.escape(statement)
        # allow **bold** → <strong>
        esc = re.sub(r"\*\*(.+?)\*\*", r"<strong>\1</strong>", esc)
        if re.search(r'class="statement"', doc):
            doc = re.sub(
                r'(<p class="statement">).*?(</p>)',
                rf"\1{esc}\2",
                doc,
                count=1,
                flags=re.S,
            )
        elif re.search(r'class="cap"', doc):
            doc = re.sub(
                r'(<p class="cap">).*?(</p>)',
                rf"\1{esc}\2",
                doc,
                count=1,
                flags=re.S,
            )

    # od-build-minimal .word
    word = slide.get("word")
    if word and re.search(r'class="word"', doc):
        doc = re.sub(
            r'(<div class="word">).*?(</div>)',
            rf"\1{html.escape(str(word))}\2",
            doc,
            count=1,
            flags=re.S,
        )

    # bullets → first <ul> or ladder rungs
    if bullets:
        lis = "".join(f"<li>{html.escape(str(b))}</li>" for b in bullets)
        if re.search(r"<ul[\s>]", doc):
            doc = re.sub(r"<ul[^>]*>.*?</ul>", f"<ul>{lis}</ul>", doc, count=1, flags=re.S)
        elif re.search(r'class="ladder"', doc):
            rungs = []
            for i, b in enumerate(bullets, 1):
                hot = " hot" if i == len(bullets) else ""
                rungs.append(
                    f'<div class="rung{hot}"><small>{i:02d}</small><b>{html.escape(str(b))}</b></div>'
                )
            doc = re.sub(
                r'(<div class="ladder">).*?(</div>\s*(?:<div class="brand-bar"|</div>\s*</body>))',
                rf'\1{"".join(rungs)}\2',
                doc,
                count=1,
                flags=re.S,
            )

    return doc


def compile_deck(spec_path: Path, out_dir: Path | None = None) -> int:
    data = json.loads(spec_path.read_text(encoding="utf-8"))
    tokens = data.get("tokens") or {}
    family = data.get("family") or ""
    if family and "family" not in tokens:
        tokens = {**tokens, "family": family}
    slides = data.get("slides") or []
    if not slides:
        print("slides.json has no slides[]", file=sys.stderr)
        return 1

    out_dir = out_dir or spec_path.parent
    out_dir.mkdir(parents=True, exist_ok=True)

    written: list[Path] = []
    for i, slide in enumerate(slides, 1):
        layout = slide.get("layout") or "cover"
        sid = slide.get("id") or f"{i:02d}-slide"
        # normalize filename
        slug = re.sub(r"[^\w\-]+", "-", str(sid)).strip("-") or f"{i:02d}-slide"
        if not re.match(r"^\d{2}", slug):
            slug = f"{i:02d}-{slug}"
        tmpl = resolve_template(str(layout))
        doc = tmpl.read_text(encoding="utf-8")
        doc = apply_tokens(doc, tokens)
        doc = fill_text_fields(doc, slide)
        # mark slide type
        doc = re.sub(
            r'data-type="[^"]*"',
            f'data-type="{html.escape(str(layout))}"',
            doc,
            count=1,
        )
        dest = out_dir / f"{slug}.html"
        dest.write_text(doc, encoding="utf-8")
        written.append(dest)
        print(f"wrote {dest} (from {tmpl.name})")

    print(f"compiled {len(written)} slides → {out_dir}")
    return 0


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: compile_slides_json.py slides.json [out_dir]", file=sys.stderr)
        return 1
    spec = Path(sys.argv[1]).resolve()
    out = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    if not spec.is_file():
        print(f"not found: {spec}", file=sys.stderr)
        return 1
    return compile_deck(spec, out)


if __name__ == "__main__":
    raise SystemExit(main())
