#!/usr/bin/env python3
"""Check relative links and #anchors in the repository's Markdown files.

Anchors are computed like GitHub does (lowercase, punctuation removed, spaces -> '-',
duplicates suffixed with -1, -2, ...). External http(s) links are not fetched.
Usage: scripts/check-docs.py [files...]   (default: README.md and docs/**/*.md)
Exit status 1 if any link is broken.
"""
import pathlib
import re
import sys
import unicodedata

ROOT = pathlib.Path(__file__).resolve().parent.parent
LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")
FENCE = re.compile(r"^\s*(```|~~~)")


def slug(text: str) -> str:
    text = re.sub(r"`([^`]*)`", r"\1", text)  # inline code keeps its text
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)  # links keep their label
    out = []
    for ch in text.lower():
        cat = unicodedata.category(ch)
        if ch in " -_" or cat[0] in ("L", "N", "M"):
            out.append("-" if ch == " " else ch)
    return "".join(out)


def anchors(path: pathlib.Path) -> set:
    seen, result, in_fence = {}, set(), False
    for line in path.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        m = None if in_fence else HEADING.match(line)
        if not m:
            continue
        base = slug(m.group(2))
        n = seen.get(base, 0)
        seen[base] = n + 1
        result.add(base if n == 0 else f"{base}-{n}")
    return result


def links(path: pathlib.Path):
    in_fence = False
    for no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        line = re.sub(r"`[^`]*`", "", line)
        for m in LINK.finditer(line):
            yield no, m.group(1)


def main() -> int:
    files = [pathlib.Path(a).resolve() for a in sys.argv[1:]] or [ROOT / "README.md", *sorted((ROOT / "docs").rglob("*.md"))]
    cache, errors = {}, []
    for f in files:
        for no, target in links(f):
            if re.match(r"^[a-z][a-z0-9+.-]*:", target):  # http:, https:, mailto:
                continue
            file_part, _, frag = target.partition("#")
            dest = (f.parent / file_part).resolve() if file_part else f
            rel = f.relative_to(ROOT)
            if not dest.exists():
                errors.append(f"{rel}:{no}: リンク先がありません: {target}")
                continue
            if frag and dest.suffix == ".md":
                if dest not in cache:
                    cache[dest] = anchors(dest)
                if frag not in cache[dest]:
                    errors.append(f"{rel}:{no}: 見出しが見つかりません: {target}")
    for e in errors:
        print("✘ " + e)
    print(f"{len(files)} ファイルを確認: " + ("リンク切れはありません" if not errors else f"{len(errors)} 件のリンク切れ"))
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
