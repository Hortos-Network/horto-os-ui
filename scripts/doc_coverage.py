#!/usr/bin/env python3
"""Measure public-item rustdoc coverage for horto-os-ui DOC_PKGS sources.

Counts `pub fn|struct|enum|trait|type|const|static` whose preceding lines include
`///` (attrs and blanks skipped). Exits non-zero when below DOC_COVERAGE_FAIL_UNDER.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

PUB_RE = re.compile(
    r"^\s*pub(?:\([^)]*\))?\s+(?:async\s+)?(?:unsafe\s+)?"
    r"(?:fn|struct|enum|trait|type|const|static)\b"
)
DOC_RE = re.compile(r"^\s*///")
ATTR_RE = re.compile(r"^\s*#\[")

DEFAULT_ROOTS = (
    "crates/horto-os-ui-shared/src",
    "crates/horto-os-ui-cli/src",
    "crates/horto-os-ui-tui/src",
    "crates/horto-os-ui-status-api/src",
    "crates/horto-os-ui-mcp/src",
)


def has_doc_comment(lines: list[str], item_index: int) -> bool:
    j = item_index - 1
    while j >= 0:
        s = lines[j]
        if DOC_RE.match(s):
            return True
        if ATTR_RE.match(s) or s.strip() == "":
            j -= 1
            continue
        return False
    return False


def scan_file(path: Path) -> tuple[int, int]:
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = text.splitlines()
    total = documented = 0
    for i, line in enumerate(lines):
        if not PUB_RE.match(line):
            continue
        total += 1
        if has_doc_comment(lines, i):
            documented += 1
    return documented, total


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="horto-os-ui repo root",
    )
    parser.add_argument(
        "--fail-under",
        type=float,
        default=float(os.environ.get("DOC_COVERAGE_FAIL_UNDER", "90")),
        help="Minimum documented percentage (default env DOC_COVERAGE_FAIL_UNDER or 90)",
    )
    args = parser.parse_args()
    root: Path = args.root

    documented = total = 0
    for rel in DEFAULT_ROOTS:
        src = root / rel
        if not src.is_dir():
            print(f"error: missing {src}", file=sys.stderr)
            return 2
        for path in sorted(src.rglob("*.rs")):
            d, t = scan_file(path)
            documented += d
            total += t

    pct = (100.0 * documented / total) if total else 100.0
    print(f"rustdoc public-item coverage: {documented}/{total} ({pct:.1f}%)")
    print(f"fail-under: {args.fail_under:.1f}%")
    if pct + 1e-9 < args.fail_under:
        print("FAIL: rustdoc coverage below threshold", file=sys.stderr)
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
