#!/usr/bin/env python3
"""Author the input half of the CLI-surface parity cases.

Each case documents one externally observable CLI behavior (invocation, stdin, files,
env) and writes it to tests/parity/cases/<name>.json with an empty `expected`. Run
tests/parity/capture.py afterwards to fill `expected` from the ORIGINAL binary.

These cases target the I/O / exit-code / logging surface (CLI-01..CLI-10 in PLAN.md).
Option/transform behavior is covered separately by cases generated from the upstream
`test/spec/*.yaml` corpus.
"""

from __future__ import annotations

import json
from pathlib import Path

CASES_DIR = Path(__file__).resolve().parent / "cases"

# A tiny representative bibliography reused across cases.
BIB = "@article{a,\n  TITLE = {Hello World},\n  author = {Smith, John},\n  year = {2020}\n}\n"


def case(name: str, *, args=None, stdin=None, files=None, env=None,
         file_mode: bool = False, match: str = "full") -> dict:
    """Build one case.

    `file_mode=True` sets PRE_COMMIT=1, which forces the original's `hasStdin`
    computation to false. Without it, a piped (always non-TTY) stdin makes the
    parser treat trailing filenames as option values rather than input files, so
    file scenarios would never exercise the file path. PRE_COMMIT=1 is the
    original's own documented switch for "no stdin".

    `match` selects how strictly the checker compares the captured behavior:
      - "full"            : stdout, stderr, exit, and file effects all match exactly.
      - "exit_stdout_files": ignore stderr (used when the original emits a
                            runtime-specific stack trace, which is non-functional
                            per INV-PARITY and non-reproducible in Rust).
      - "exit_only"       : compare only the exit code.
    """
    env = dict(env or {})
    if file_mode:
        env["PRE_COMMIT"] = "1"
    return {
        "name": name,
        "args": args or [],
        "stdin": stdin,
        "files": files or {},
        "env": env,
        "match": match,
        "expected": None,  # filled by capture.py from the original
    }


CASES: list[dict] = [
    # --- CLI-01 version ---
    case("version", args=["--version"]),
    case("version-short", args=["-v"]),
    # --- CLI-02 help (explicit flags) ---
    case("help-long", args=["--help"]),
    case("help-short", args=["-h"]),
    # --- CLI-03/04 stdin -> stdout (default) ---
    case("stdin-default", stdin=BIB),
    case("stdin-omit", args=["--omit=title"], stdin=BIB),
    case("stdin-curly", args=["--curly"], stdin=BIB),
    case("stdin-sort-desc", args=["--sort=-year"], stdin=BIB),
    case("stdin-quiet-stdout", args=["--quiet"], stdin=BIB),
    # --- CLI-06 unknown args ---
    case("unknown-single", args=["--bogus"], stdin=BIB),
    case("unknown-multiple", args=["--bogus", "--nope"], stdin=BIB),
    # --- CLI-04 output to file ---
    case("output-file", args=["-o", "out.bib"], stdin=BIB),
    case("output-long", args=["--output", "out.bib"], stdin=BIB),
    case("output-file-quiet", args=["-o", "out.bib", "--quiet"], stdin=BIB),
    # --- CLI-05/10 modify + backup on an input file ---
    case(
        "modify-backup",
        args=["-m", "--backup", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
    ),
    case(
        "modify-no-backup",
        args=["-m", "--no-backup", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
    ),
    case(
        "modify-explicit-no-backup-default",
        # -m given explicitly -> backup defaults OFF (per option definition)
        args=["-m", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
    ),
    # --- CLI-10 deprecated stdio: file input with no -o/-m/--v2 ---
    case("deprecated-stdio", args=["in.bib"], files={"in.bib": BIB}, file_mode=True),
    # --- CLI-09 v2 gating ---
    case("v2-stdin", args=["--v2"], stdin=BIB),
    case(
        "v2-file-stdout",
        # v2 + file input + no -o/-m -> prints to stdout (no modify)
        args=["--v2", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
    ),
    # --- error combinations ---
    # NOTE: `-m -o out.bib in.bib` in file mode is a genuine parser quirk: the
    # trailing tokens become input paths (not -o's value), so outputPath is
    # undefined and modify loops over both names, crashing on the missing
    # "out.bib". The original dies with a Node stack trace, which is
    # non-functional (INV-PARITY), so we match exit/stdout/files but ignore stderr.
    case(
        "modify-with-output",
        args=["-m", "-o", "out.bib", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
        match="exit_stdout_files",
    ),
    # A missing input file in the non-modify path: same class of runtime crash
    # (uncaught ENOENT), so ignore the stack-trace stderr.
    case(
        "missing-input-file",
        args=["nonexistent.bib"],
        file_mode=True,
        match="exit_stdout_files",
    ),
    case(
        "backup-without-modify-v2",
        args=["--v2", "--backup", "in.bib"],
        files={"in.bib": BIB},
        file_mode=True,
    ),
    case(
        "two-files-no-modify-v2",
        args=["--v2", "a.bib", "b.bib"],
        files={"a.bib": BIB, "b.bib": BIB},
        file_mode=True,
    ),
    case(
        "modify-stdin",
        args=["-m"],
        stdin=BIB,
    ),
    case(
        "backup-stdin",
        args=["--backup"],
        stdin=BIB,
    ),
    # --- CLI-03 legacy '-' stdin marker ---
    case("dash-stdin", args=["-"], stdin=BIB),
    case("dash-with-file", args=["-", "in.bib"], files={"in.bib": BIB}),
]


def main() -> None:
    CASES_DIR.mkdir(parents=True, exist_ok=True)
    for c in CASES:
        path = CASES_DIR / f"{c['name']}.json"
        path.write_text(
            json.dumps(c, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
    print(f"wrote {len(CASES)} CLI-surface case definitions to {CASES_DIR}")


if __name__ == "__main__":
    main()