#!/usr/bin/env python3
"""Compare explicitly extracted command response bytes; never execute clients.

Example (client comparison against the SAME Director and fixture):
  python3 scripts/compare_command_responses.py \
    --command '.defaults client=fd' --context director-main --fixture-id cat-001 \
    --comparison-level clients --reference-director-version bareos-25.1.2 \
    --candidate-director-version bareos-25.1.2 \
    --bconsole-version bareos-25.1.2 --oconsole-version onesimus-0.1.0 \
    --bconsole-response /tmp/bconsole.raw --oconsole-response /tmp/oconsole.raw \
    --bconsole-status completed --oconsole-status completed \
    --bconsole-exit-code 0 --oconsole-exit-code 0 --report /tmp/parity.json

For the separate Original/Rust Director comparison, use --comparison-level
 directors and record the two distinct Director versions. Record API mode,
 locale, timezone, terminal parameters and extraction boundaries in the fixture
 identified by --fixture-id; --context identifies the command context/API mode.

Inputs must already exclude explicitly documented prompt/banner/echo boundaries.
No trimming, newline conversion, decoding, sorting or JSON reserialization takes
place. Empty completed responses are valid; empty files from unrun/failed/timed
out commands never pass. Both completed statuses AND explicit zero exit codes
are required. Missing/unreadable files never pass. Exit 0 means byte parity for
this recorded case only; exit 1 means failed/incomplete evidence, exit 2 CLI error.
Report writing failures also exit nonzero. This tool proves response-byte parity,
not matching side effects, ACL correctness or full command implementation.
"""

import argparse
import difflib
import hashlib
import json
from pathlib import Path


def compare(args):
    """Return a JSON-serializable report without altering either input file."""
    report = {
        "schema_version": 1,
        "command": args.command,
        "context": args.context,
        "fixture_id": args.fixture_id,
        "comparison_level": args.comparison_level,
        "versions": {
            "bconsole": args.bconsole_version,
            "oconsole": args.oconsole_version,
            "reference_director": args.reference_director_version,
            "candidate_director": args.candidate_director_version,
        },
        "passed": False,
        "reasons": [],
        "responses": {},
    }
    payloads = {}
    for side in ("bconsole", "oconsole"):
        path = Path(getattr(args, side + "_response"))
        status = getattr(args, side + "_status")
        exit_code = getattr(args, side + "_exit_code")
        item = {"path": str(path), "status": status, "exit_code": exit_code,
                "byte_count": None, "sha256": None}
        report["responses"][side] = item
        if status != "completed":
            report["reasons"].append(f"{side}: status is {status}")
        if exit_code != 0:
            report["reasons"].append(f"{side}: explicit zero exit code required")
        try:
            data = path.read_bytes()
        except OSError as exc:
            item["read_error"] = str(exc)
            report["reasons"].append(f"{side}: response file unavailable")
        else:
            payloads[side] = data
            item["byte_count"] = len(data)
            item["sha256"] = hashlib.sha256(data).hexdigest()

    if args.comparison_level == "clients" and args.reference_director_version != args.candidate_director_version:
        report["reasons"].append("client comparison requires the same Director version")
    if len(payloads) == 2:
        left, right = payloads["bconsole"], payloads["oconsole"]
        equal = left == right
        report["bytes_equal"] = equal
        if not equal:
            report["reasons"].append("response bytes differ")
            report["first_difference_offset"] = next(
                (i for i, (a, b) in enumerate(zip(left, right)) if a != b),
                min(len(left), len(right)),
            )
            # Decode only the diagnostic representation, never compared inputs.
            # repr() makes CR/LF, missing newline and arbitrary encodings visible.
            diff = difflib.unified_diff(
                [repr(line) + "\n" for line in left.splitlines(keepends=True)],
                [repr(line) + "\n" for line in right.splitlines(keepends=True)],
                fromfile="bconsole response bytes", tofile="oconsole response bytes",
            )
            report["diff"] = "".join(diff)
    else:
        report["bytes_equal"] = None
    report["passed"] = not report["reasons"]
    return report


def parser():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    for key in ("command", "context", "fixture-id", "bconsole-version", "oconsole-version",
                "reference-director-version", "candidate-director-version", "report"):
        p.add_argument("--" + key, required=True)
    p.add_argument("--comparison-level", required=True, choices=("clients", "directors"))
    for side in ("bconsole", "oconsole"):
        p.add_argument(f"--{side}-response", required=True)
        p.add_argument(f"--{side}-status", required=True, choices=("completed", "timeout", "unrun", "error"))
        p.add_argument(f"--{side}-exit-code", type=int)
    p.add_argument("--diff-file", help="Optional escaped-byte unified diff; empty for equal/unavailable bytes")
    return p


def main(argv=None):
    p = parser()
    args = p.parse_args(argv)
    for name in ("command", "context", "fixture_id", "bconsole_version", "oconsole_version",
                 "reference_director_version", "candidate_director_version"):
        if not getattr(args, name).strip():
            p.error(f"{name.replace('_', '-')} must not be empty")
    inputs = {Path(args.bconsole_response).resolve(), Path(args.oconsole_response).resolve()}
    outputs = [Path(args.report).resolve()]
    if args.diff_file:
        outputs.append(Path(args.diff_file).resolve())
    def same_file(a, b):
        if a == b:
            return True
        try:
            return a.samefile(b)
        except OSError:
            return False

    if any(same_file(output, source) for output in outputs for source in inputs) or (
        len(outputs) == 2 and same_file(outputs[0], outputs[1])
    ):
        p.error("report/diff outputs must be distinct from each other and input files")
    report = compare(args)
    try:
        if args.diff_file:
            Path(args.diff_file).write_text(report.get("diff", ""), encoding="utf-8")
        Path(args.report).write_text(json.dumps(report, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")
    except OSError as exc:
        p.exit(1, f"Could not write comparison evidence: {exc}\n")
    print("PASS" if report["passed"] else "FAIL")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
