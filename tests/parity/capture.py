#!/usr/bin/env python3
"""Capture parity fixtures from the ORIGINAL bibtex-tidy binary.

Reads case definitions from tests/parity/cases/*.json (the `input` half), runs the
original binary for each, and fills in the `expected` half. Re-running overwrites
`expected` from the original; never hand-edit `expected` (INV-ORACLE).

The original binary lives in the `upstream/` submodule and is executed with node.
Capture requires node + the submodule; CI never runs this — it replays the committed
fixtures against the Rust port.

Usage:
    python3 tests/parity/capture.py            # capture all cases
    python3 tests/parity/capture.py NAME...    # capture only the named cases
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

PORT_ROOT = Path(__file__).resolve().parents[2]
CASES_DIR = PORT_ROOT / "tests" / "parity" / "cases"
ORIGINAL = PORT_ROOT / "upstream" / "bin" / "bibtex-tidy"
NODE = shutil.which("node") or "node"


def run_original(case: dict) -> dict:
    """Run the original binary for one case and return its normalized observable behavior."""
    with tempfile.TemporaryDirectory(prefix="btparity-") as tmp:
        tmp_path = Path(tmp)
        # Materialize any files the case needs in the temp cwd.
        for rel, content in case.get("files", {}).items():
            target = tmp_path / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(content, encoding="utf-8")

        env = dict(os.environ)
        # Strip any ambient PRE_COMMIT first so it can't leak into cases that don't
        # set it, then apply the case's explicit env (which may deliberately set it
        # to force "no stdin" / file mode).
        env.pop("PRE_COMMIT", None)
        env.update(case.get("env", {}))

        stdin = case.get("stdin")
        proc = subprocess.run(
            [NODE, str(ORIGINAL), *case.get("args", [])],
            cwd=tmp,
            input=stdin if stdin is not None else "",
            capture_output=True,
            text=True,
            env=env,
        )

        # Capture the resulting file tree (relative paths + contents) so file effects
        # (e.g. --output, --modify, backup) are part of the fixture.
        files: dict[str, str] = {}
        for path in sorted(tmp_path.rglob("*")):
            if path.is_file():
                files[str(path.relative_to(tmp_path))] = path.read_text(
                    encoding="utf-8"
                )

        return {
            "stdout": _normalize(proc.stdout, tmp),
            "stderr": _normalize(proc.stderr, tmp),
            "exit": proc.returncode,
            "files": files,
        }


def _normalize(text: str, tmp: str) -> str:
    """Mask the per-run temp directory so fixtures are deterministic across machines."""
    return text.replace(tmp, "<TMP>")


def capture(names: list[str] | None = None) -> int:
    if not ORIGINAL.exists():
        print(f"original binary not found: {ORIGINAL}", file=sys.stderr)
        return 1
    CASES_DIR.mkdir(parents=True, exist_ok=True)

    case_files = sorted(CASES_DIR.glob("*.json"))
    if names:
        case_files = [p for p in case_files if p.stem in names]

    if not case_files:
        print("no case files to capture", file=sys.stderr)
        return 1

    failures = 0
    for path in case_files:
        case = json.loads(path.read_text(encoding="utf-8"))
        case["expected"] = run_original(case)
        path.write_text(
            json.dumps(case, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        print(f"captured {case['name']:<40} exit={case['expected']['exit']}")
    return 0 if failures == 0 else 1


def _diff(label, expected, actual):
    return None if expected == actual else f"    {label}:\n      expected={expected!r}\n      actual  ={actual!r}"


def verify(names: list[str] | None = None) -> int:
    """Self-check: re-run the ORIGINAL and diff against stored `expected`.

    This validates that fixtures are deterministic and that normalization is neither
    over- nor under-masking. It must be 100% green before any porting (S1 exit gate).
    """
    case_files = sorted(CASES_DIR.glob("*.json"))
    if names:
        case_files = [p for p in case_files if p.stem in names]
    if not case_files:
        print("no case files to verify", file=sys.stderr)
        return 1

    failures = 0
    for path in case_files:
        case = json.loads(path.read_text(encoding="utf-8"))
        expected = case.get("expected")
        if not expected:
            print(f"verify {case['name']:<40} SKIP (no expected)")
            continue
        policy = case.get("match", "full")
        actual = run_original(case)
        problems = []
        if actual["exit"] != expected["exit"]:
            problems.append(f"    exit: expected={expected['exit']} actual={actual['exit']}")
        if policy in ("full", "exit_stdout_files"):
            p = _diff("stdout", expected["stdout"], actual["stdout"])
            if p:
                problems.append(p)
            ef = {k: v for k, v in expected["files"].items()}
            af = {k: v for k, v in actual["files"].items()}
            p = _diff("files", sorted(ef.items()), sorted(af.items()))
            if p:
                problems.append(p)
        if policy == "full":
            p = _diff("stderr", expected["stderr"], actual["stderr"])
            if p:
                problems.append(p)
        if problems:
            failures += 1
            print(f"verify {case['name']:<40} FAIL (match={policy})")
            for pr in problems:
                print(pr)
        else:
            print(f"verify {case['name']:<40} ok")
    print(f"\n{failures} failure(s)")
    return 0 if failures == 0 else 1


if __name__ == "__main__":
    args = sys.argv[1:]
    if args and args[0] == "verify":
        sys.exit(verify(args[1:] or None))
    sys.exit(capture(args or None))