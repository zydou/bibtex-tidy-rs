#!/usr/bin/env python3
"""Generate parity cases from the upstream `test/spec/*.spec.yaml` suite.

Each spec document describes an invocation (`options` in camelCase) plus an `input`
BibTeX document. We turn the options into CLI flags, use `input` as stdin, and let
`capture.py` fill in `expected` from the ORIGINAL binary (INV-ORACLE: fixtures are
captured, never hand-written).

Specs that exercise transforms not yet ported are skipped and reported, so the
generated corpus always covers only implemented behavior.

Usage:
    python3 tests/parity/define_spec_cases.py            # write cases
    python3 tests/parity/define_spec_cases.py --dry-run  # only report
"""

from __future__ import annotations

import glob
import json
import re
import sys
from pathlib import Path

import yaml

PORT_ROOT = Path(__file__).resolve().parents[2]
SPEC_DIR = PORT_ROOT / "upstream" / "test" / "spec"
CASES_DIR = PORT_ROOT / "tests" / "parity" / "cases"

# Transforms not yet ported (S3 in progress). Specs using these are skipped.
UNPORTED_OPTIONS: set[str] = set()


# Spec option keys whose CLI flag is not the naive kebab-case of the key.
CLI_FLAG_OVERRIDES = {
    "removeDuplicateFields": "remove-dupe-fields",
}


def kebab(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "-", name).lower()


def option_to_args(key: str, value) -> list[str]:
    flag = "--" + CLI_FLAG_OVERRIDES.get(key, kebab(key))
    if isinstance(value, bool):
        return [flag] if value else ["--no-" + flag[2:]]
    if isinstance(value, list):
        return [f"{flag}={','.join(str(v) for v in value)}"]
    return [f"{flag}={value}"]


def split_docs(text: str):
    """Split the `---`/`...`-delimited documents the spec suite uses."""
    block: list[str] = []
    for line in text.splitlines():
        if line.strip() in ("---", "..."):
            yield "\n".join(block)
            block = []
        else:
            block.append(line)
    if any(s.strip() for s in block):
        yield "\n".join(block)


def collect():
    specs = []
    skipped = {}
    for path in sorted(SPEC_DIR.glob("*.spec.yaml")):
        stem = path.name[: -len(".spec.yaml")]
        for i, raw in enumerate(split_docs(path.read_text(encoding="utf-8"))):
            doc = yaml.safe_load(raw)
            if not isinstance(doc, dict) or "input" not in doc:
                continue
            options = doc.get("options") or {}
            used_unported = sorted(set(options) & UNPORTED_OPTIONS)
            if used_unported:
                skipped.setdefault(tuple(used_unported), []).append(f"{stem}#{i}")
                continue
            args: list[str] = []
            for key, value in options.items():
                args.extend(option_to_args(key, value))
            title = doc.get("title", f"{stem}#{i}")
            slug = re.sub(r"[^a-z0-9]+", "-", f"{stem}-{title}".lower()).strip("-")
            specs.append(
                {
                    "name": f"spec-{slug}"[:100],
                    "args": args,
                    "stdin": doc["input"],
                    "files": {},
                    "env": {},
                    "match": "full",
                }
            )
    return specs, skipped


def main() -> int:
    dry = "--dry-run" in sys.argv
    specs, skipped = collect()
    for reason, names in sorted(skipped.items()):
        print(f"skip (unported {', '.join(reason)}): {len(names)} cases")
    print(f"generated {len(specs)} spec cases")
    if dry:
        return 0
    CASES_DIR.mkdir(parents=True, exist_ok=True)
    for case in specs:
        out = CASES_DIR / f"{case['name']}.json"
        out.write_text(
            json.dumps(case, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
