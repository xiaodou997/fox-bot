#!/usr/bin/env python3
"""Offline Markdown checks against the actual checkout; no connector snapshots.

Checks local inline/reference link file targets (not anchors), fenced JSON, and
acceptance case declarations/references. Does not fetch external URLs or lint all
Markdown syntax. Uses git's ignore rules to exclude build/private state files.
"""
from __future__ import annotations

import collections
import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
CASE = re.compile(r"\b(?:DOC|CORE|AI|TX|GR|OC|AD|MC|WI|NW|RL)-\d{2}\b")
FENCE = re.compile(r"^```([^\n]*)\n(.*?)^```\s*$", re.M | re.S)
INLINE = re.compile(r"!?\[[^\]\n]*\]\(([^)\n]+)\)")
REFERENCE = re.compile(r"^\s*\[[^\]\n]+\]:\s*(\S+)", re.M)


def markdown_paths() -> list[Path]:
    result = subprocess.run(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "*.md"],
        cwd=ROOT, check=True, stdout=subprocess.PIPE,
    )
    return sorted({ROOT / item.decode("utf-8") for item in result.stdout.split(b"\0") if item})


def inspect(paths: list[Path]) -> dict:
    errors: list[str] = []
    links = 0
    json_blocks = 0
    contents = {}
    for path in paths:
        if not path.is_file():
            errors.append(f"missing Markdown file: {path.relative_to(ROOT)}")
            continue
        contents[path] = path.read_text(encoding="utf-8")
    checklist = ROOT / "docs/acceptance/ACCEPTANCE_CHECKLIST.md"
    declarations = re.findall(r"^\|\s*([A-Z]+-\d{2})\s*\|", contents.get(checklist, ""), re.M)
    declared = set(declarations)
    duplicates = [key for key, count in collections.Counter(declarations).items() if count != 1]
    if not declared or duplicates:
        errors.append(f"invalid acceptance declarations: duplicates={duplicates}")
    for path, text in contents.items():
        label = str(path.relative_to(ROOT))
        for language, payload in FENCE.findall(text):
            if language.strip().lower() == "json":
                json_blocks += 1
                try:
                    json.loads(payload)
                except ValueError as error:
                    errors.append(f"{label}: invalid fenced JSON: {error}")
        prose = FENCE.sub("", text)
        for raw in INLINE.findall(prose) + REFERENCE.findall(prose):
            target = raw.strip().split()[0].strip("<>")
            url = urlsplit(target)
            if url.scheme or url.netloc or not url.path:
                continue
            links += 1
            resolved = (path.parent / unquote(url.path)).resolve()
            try:
                resolved.relative_to(ROOT)
            except ValueError:
                errors.append(f"{label}: link escapes checkout: {target}")
                continue
            if not resolved.exists():
                errors.append(f"{label}: missing local link target: {target}")
        for case in sorted(set(CASE.findall(prose)) - declared):
            errors.append(f"{label}: undefined case {case}")
    return {"markdown_files": len(contents), "local_links_checked": links,
            "fenced_json_checked": json_blocks, "unique_acceptance_cases": len(declared),
            "errors": errors, "scope": "checkout local file targets, fenced JSON and case references; no web requests or anchor validation"}


def main() -> int:
    try:
        result = inspect(markdown_paths())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"documentation check failed: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 1 if result["errors"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
