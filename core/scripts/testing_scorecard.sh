#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

CTX_TESTING_SCORECARD_SCRIPT_DIR="${SCRIPT_DIR}" \
python3 - "$@" <<'PY'
import argparse
import json
import os
import sys
from collections import defaultdict
from pathlib import Path
from typing import Optional

DEFAULT_CORPUS_DIRS = {
    "providers": "crates/ctx-providers/tests/corpus/acp",
    "mcp": "crates/ctx-mcp/tests/corpus/tools",
    "workspace": "crates/ctx-core/tests/corpus/workspace_payloads",
    "updates": "crates/ctx-http/tests/corpus/release_manifests",
    "desktop_ipc": "apps/web/src/utils/testdata/desktop-ipc",
}

DAEMON_THRESHOLDS = {
    "crates/ctx-core/src/models.rs": 60.0,
    "crates/ctx-http/src/updates.rs": 60.0,
    "crates/ctx-http/src/api/updates.rs": 60.0,
}

WEB_THRESHOLDS = {
    "apps/web/src/state/workspaceActiveSnapshotStoreCore.ts": 55.0,
    "apps/web/src/pages/SessionPage.workbenchViewModel.ts": 50.0,
    "apps/web/src/components/UpdateNoticeBanner.tsx": 80.0,
    "apps/web/src/components/DaemonAvailabilityOverlay.tsx": 75.0,
    "apps/web/src/utils/desktop.ts": 35.0,
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="testing_scorecard.sh",
        description="Summarize testing meta-signals: flakes, coverage, mutation, and fuzz corpora.",
    )
    parser.add_argument("--jsonl", action="append", default=[], help="Add a ci-flake-stats.jsonl input file.")
    parser.add_argument(
        "--corpus-dir",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="Add a corpus directory by stable name.",
    )
    parser.add_argument("--daemon-lcov", help="Path to daemon lcov output.")
    parser.add_argument("--web-summary", help="Path to web coverage-summary.json output.")
    parser.add_argument("--mutation-report", help="Path to mutation report.json output.")
    parser.add_argument("--out-json", help="Write the JSON scorecard to this path.")
    parser.add_argument("--out-pretty", help="Write a human-readable scorecard to this path.")
    parser.add_argument("--pretty", action="store_true", help="Print a human-readable summary.")
    parser.add_argument(
        "--enforce-coverage",
        action="store_true",
        help="Fail when critical coverage thresholds are missing or below target.",
    )
    return parser.parse_args()


def read_json(path: Path):
    with path.open("r", encoding="utf-8") as f:
        return json.load(f)


def resolve_path(raw: str, primary_base: Path, secondary_base: Optional[Path] = None) -> Path:
    path = Path(raw)
    if path.is_absolute():
        return path

    primary = (primary_base / path).resolve()
    if primary.exists():
        return primary

    if secondary_base is not None:
        secondary = (secondary_base / path).resolve()
        if secondary.exists():
            return secondary

    return primary


def normalize_path(raw: str, cwd: Path) -> str:
    path = Path(raw)
    if path.is_absolute():
        try:
            return path.relative_to(cwd).as_posix()
        except ValueError:
            return path.as_posix()
    return path.as_posix()


def find_entry(entries: dict[str, dict], target: str) -> Optional[dict]:
    if target in entries:
        return entries[target]
    matches = [value for key, value in entries.items() if key.endswith(target)]
    if len(matches) == 1:
        return matches[0]
    return None


def collect_flake_summary(paths: list[Path]) -> Optional[dict]:
    if not paths:
        return None

    rows = []
    for path in paths:
        with path.open("r", encoding="utf-8") as f:
            for lineno, raw in enumerate(f, start=1):
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    row = json.loads(raw)
                except json.JSONDecodeError as exc:
                    raise SystemExit(f"error: {path}:{lineno}: invalid json: {exc}")
                row["_path"] = path.as_posix()
                rows.append(row)

    if not rows:
        return None

    by_job = defaultdict(
        lambda: {
            "total": 0,
            "failed": 0,
            "flake": 0,
            "flake_failed": 0,
            "attempts_sum": 0,
            "attempts_max": 0,
        }
    )

    for row in rows:
        job = row.get("job", "unknown")
        status = row.get("status", "unknown")
        attempts = int(row.get("attempts", 0))
        stats = by_job[job]
        stats["total"] += 1
        stats["attempts_sum"] += attempts
        stats["attempts_max"] = max(stats["attempts_max"], attempts)
        if status == "failed":
            stats["failed"] += 1
        elif status == "flake":
            stats["flake"] += 1
        elif status == "flake_failed":
            stats["flake_failed"] += 1

    jobs = {}
    for job in sorted(by_job.keys()):
        stats = by_job[job]
        total = stats["total"]
        jobs[job] = {
            "total_runs": total,
            "failed_runs": stats["failed"],
            "flake_runs": stats["flake"],
            "flake_failed_runs": stats["flake_failed"],
            "pass_without_flake_runs": max(
                total - stats["failed"] - stats["flake"] - stats["flake_failed"], 0
            ),
            "flake_rate": round((stats["flake"] + stats["flake_failed"]) / total, 4)
            if total
            else 0.0,
            "failure_rate": round(stats["failed"] / total, 4) if total else 0.0,
            "avg_attempts": round(stats["attempts_sum"] / total, 3) if total else 0.0,
            "max_attempts": stats["attempts_max"],
        }

    return {
        "files": [path.as_posix() for path in paths],
        "rows": len(rows),
        "jobs": jobs,
    }


def collect_corpus_summary(corpus_dirs: dict[str, Path]) -> dict:
    entries = {}
    total_files = 0
    for name, path in sorted(corpus_dirs.items()):
        count = 0
        if path.is_dir():
            count = sum(1 for entry in path.rglob("*") if entry.is_file())
        entries[name] = {"path": path.as_posix(), "files": count}
        total_files += count
    return {"dirs": entries, "total_files": total_files}


def parse_lcov(path: Path, cwd: Path) -> dict[str, dict]:
    entries: dict[str, dict] = {}
    current_path = None
    total = 0
    covered = 0

    def flush() -> None:
        nonlocal current_path, total, covered
        if current_path is None:
            return
        pct = round((covered / total * 100.0), 2) if total else 0.0
        entries[current_path] = {
            "path": current_path,
            "lines_total": total,
            "lines_covered": covered,
            "lines_pct": pct,
        }
        current_path = None
        total = 0
        covered = 0

    with path.open("r", encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if line.startswith("SF:"):
                flush()
                current_path = normalize_path(line[3:], cwd)
            elif line.startswith("DA:"):
                _, payload = line.split(":", 1)
                _line_no, hits = payload.split(",", 1)
                total += 1
                if int(hits) > 0:
                    covered += 1
            elif line == "end_of_record":
                flush()
    flush()
    return entries


def collect_daemon_coverage(path: Path, cwd: Path) -> dict:
    parsed = parse_lcov(path, cwd)
    files = {}
    all_pass = True
    failures = []

    for target, threshold in DAEMON_THRESHOLDS.items():
        entry = find_entry(parsed, target)
        if entry is None:
            files[target] = {
                "path": target,
                "present": False,
                "lines_pct": 0.0,
                "lines_total": 0,
                "lines_covered": 0,
                "threshold_pct": threshold,
                "pass": False,
            }
            failures.append(f"missing daemon coverage entry for {target}")
            all_pass = False
            continue

        result = {
            **entry,
            "present": True,
            "threshold_pct": threshold,
            "pass": entry["lines_pct"] >= threshold,
        }
        if not result["pass"]:
            failures.append(
                f"daemon coverage {target} below threshold: {entry['lines_pct']:.2f} < {threshold:.2f}"
            )
            all_pass = False
        files[target] = result

    return {"source": path.as_posix(), "pass": all_pass, "files": files, "failures": failures}


def collect_web_coverage(path: Path, cwd: Path) -> dict:
    payload = read_json(path)
    entries = {}
    for raw_path, summary in payload.items():
        if raw_path == "total":
            continue
        entries[normalize_path(raw_path, cwd)] = {
            "path": normalize_path(raw_path, cwd),
            "lines_pct": float(summary["lines"]["pct"]),
            "branches_pct": float(summary["branches"]["pct"]),
            "functions_pct": float(summary["functions"]["pct"]),
            "statements_pct": float(summary["statements"]["pct"]),
        }

    files = {}
    all_pass = True
    failures = []

    for target, threshold in WEB_THRESHOLDS.items():
        entry = find_entry(entries, target)
        if entry is None:
            files[target] = {
                "path": target,
                "present": False,
                "lines_pct": 0.0,
                "branches_pct": 0.0,
                "functions_pct": 0.0,
                "statements_pct": 0.0,
                "threshold_pct": threshold,
                "pass": False,
            }
            failures.append(f"missing web coverage entry for {target}")
            all_pass = False
            continue

        result = {
            **entry,
            "present": True,
            "threshold_pct": threshold,
            "pass": entry["lines_pct"] >= threshold,
        }
        if not result["pass"]:
            failures.append(
                f"web coverage {target} below threshold: {entry['lines_pct']:.2f} < {threshold:.2f}"
            )
            all_pass = False
        files[target] = result

    return {"source": path.as_posix(), "pass": all_pass, "files": files, "failures": failures}


def collect_mutation_summary(path: Optional[Path]) -> Optional[dict]:
    if path is None:
        return None
    payload = read_json(path)
    mutants = payload.get("mutants", [])
    summary = payload.get("summary")
    if summary is None:
        eligible = [m for m in mutants if m.get("status") != "skipped"]
        killed = sum(1 for m in eligible if m.get("status") == "killed")
        survived = sum(1 for m in eligible if m.get("status") == "survived")
        timed_out = sum(1 for m in eligible if m.get("status") == "timed_out")
        skipped = sum(1 for m in mutants if m.get("status") == "skipped")
        categories = defaultdict(
            lambda: {"eligible": 0, "killed": 0, "survived": 0, "timed_out": 0, "skipped": 0}
        )
        for mutant in mutants:
            bucket = categories[mutant.get("category", "unknown")]
            if mutant.get("status") == "skipped":
                bucket["skipped"] += 1
                continue
            bucket["eligible"] += 1
            if mutant.get("status") == "killed":
                bucket["killed"] += 1
            elif mutant.get("status") == "survived":
                bucket["survived"] += 1
            elif mutant.get("status") == "timed_out":
                bucket["timed_out"] += 1
        summary = {
            "total_mutants": len(mutants),
            "eligible_mutants": len(eligible),
            "killed_mutants": killed,
            "survived_mutants": survived,
            "timed_out_mutants": timed_out,
            "skipped_mutants": skipped,
            "score_pct": round((killed / len(eligible) * 100.0), 2) if eligible else 0.0,
            "categories": categories,
        }
    return {"source": path.as_posix(), **summary}


def build_pretty(scorecard: dict) -> str:
    lines = []

    missing_inputs = scorecard.get("missing_inputs") or []
    if missing_inputs:
        lines.append("missing_inputs")
        for path in missing_inputs:
            lines.append(f"  {path}")

    coverage = scorecard.get("coverage")
    if coverage:
        lines.append("coverage")
        for label in ("daemon", "web"):
            section = coverage.get(label)
            if section is None:
                continue
            lines.append(f"  {label}: {'pass' if section['pass'] else 'fail'}")
            for path, item in section["files"].items():
                lines.append(
                    "    {path}: lines={lines_pct:.2f}% threshold={threshold_pct:.2f}% {status}".format(
                        path=path,
                        lines_pct=item["lines_pct"],
                        threshold_pct=item["threshold_pct"],
                        status="pass" if item["pass"] else "fail",
                    )
                )

    mutation = scorecard.get("mutation")
    if mutation:
        lines.append(
            "mutation\n  eligible={eligible_mutants} killed={killed_mutants} survived={survived_mutants} timed_out={timed_out_mutants} skipped={skipped_mutants} score={score_pct:.2f}%".format(
                **mutation
            )
        )
        categories = mutation.get("categories", {})
        for category in sorted(categories.keys()):
            bucket = categories[category]
            lines.append(
                "    {category}: eligible={eligible} killed={killed} survived={survived} skipped={skipped}".format(
                    category=category,
                    eligible=bucket["eligible"],
                    killed=bucket["killed"],
                    survived=bucket["survived"],
                    skipped=bucket["skipped"],
                )
            )
            if bucket.get("timed_out", 0):
                lines.append(
                    "      timed_out={timed_out}".format(
                        timed_out=bucket["timed_out"],
                    )
                )

    corpus = scorecard.get("fuzz_corpus")
    if corpus:
        lines.append(f"corpus\n  total_files={corpus['total_files']}")
        for name, entry in corpus["dirs"].items():
            lines.append(f"    {name}: {entry['files']} ({entry['path']})")

    flake = scorecard.get("flake")
    if flake:
        lines.append(f"flake\n  rows={flake['rows']} files={len(flake['files'])}")
        for job, entry in flake["jobs"].items():
            lines.append(
                "    {job}: flake_rate={flake_rate} failure_rate={failure_rate} avg_attempts={avg_attempts}".format(
                    job=job,
                    flake_rate=entry["flake_rate"],
                    failure_rate=entry["failure_rate"],
                    avg_attempts=entry["avg_attempts"],
                )
            )

    if not lines:
        lines.append("no inputs")

    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()
    cwd = Path.cwd()
    script_dir_env = os.environ.get("CTX_TESTING_SCORECARD_SCRIPT_DIR")
    script_dir = Path(script_dir_env).resolve() if script_dir_env else cwd
    core_root = script_dir.parent if script_dir.name == "scripts" else script_dir

    flake_paths = [resolve_path(item, cwd, core_root) for item in args.jsonl]
    if not flake_paths:
        default_flake = cwd / "ci-flake-stats.jsonl"
        if default_flake.is_file():
            flake_paths = [default_flake]

    corpus_dirs = {}
    if args.corpus_dir:
        for entry in args.corpus_dir:
            if "=" not in entry:
                raise SystemExit(f"error: --corpus-dir must be NAME=PATH, got: {entry}")
            name, raw_path = entry.split("=", 1)
            corpus_dirs[name] = resolve_path(raw_path, cwd, core_root)
    else:
        corpus_dirs = {name: (core_root / rel).resolve() for name, rel in DEFAULT_CORPUS_DIRS.items()}

    daemon_lcov = resolve_path(args.daemon_lcov, cwd, core_root) if args.daemon_lcov else None
    web_summary = resolve_path(args.web_summary, cwd, core_root) if args.web_summary else None
    mutation_report = resolve_path(args.mutation_report, cwd, core_root) if args.mutation_report else None
    if mutation_report is None:
        env_value = os.environ.get("CTX_MUTATION_REPORT")
        if env_value:
            mutation_report = resolve_path(env_value, cwd, core_root)

    missing_inputs = []

    resolved_flake_paths = []
    for path in flake_paths:
        if path.is_file():
            resolved_flake_paths.append(path)
        else:
            missing_inputs.append(path.as_posix())

    def resolve_optional_input(path: Optional[Path]) -> Optional[Path]:
        if path is None:
            return None
        if path.is_file():
            return path
        missing_inputs.append(path.as_posix())
        return None

    flake_paths = resolved_flake_paths
    daemon_lcov = resolve_optional_input(daemon_lcov)
    web_summary = resolve_optional_input(web_summary)
    mutation_report = resolve_optional_input(mutation_report)

    scorecard = {
        "coverage": {},
        "mutation": collect_mutation_summary(mutation_report),
        "fuzz_corpus": collect_corpus_summary(corpus_dirs),
        "flake": collect_flake_summary(flake_paths),
        "missing_inputs": missing_inputs,
        "thresholds": {
            "daemon_lines_pct": DAEMON_THRESHOLDS,
            "web_lines_pct": WEB_THRESHOLDS,
        },
    }

    coverage_failures = []
    if daemon_lcov is not None:
        daemon = collect_daemon_coverage(daemon_lcov, core_root)
        scorecard["coverage"]["daemon"] = daemon
        coverage_failures.extend(daemon["failures"])
    if web_summary is not None:
        web = collect_web_coverage(web_summary, core_root)
        scorecard["coverage"]["web"] = web
        coverage_failures.extend(web["failures"])
    if not scorecard["coverage"]:
        scorecard["coverage"] = None

    pretty_output = build_pretty(scorecard)
    json_output = json.dumps(scorecard, indent=2, sort_keys=True) + "\n"

    if args.out_json:
        out_json = Path(args.out_json)
        out_json.parent.mkdir(parents=True, exist_ok=True)
        out_json.write_text(json_output, encoding="utf-8")
    if args.out_pretty:
        out_pretty = Path(args.out_pretty)
        out_pretty.parent.mkdir(parents=True, exist_ok=True)
        out_pretty.write_text(pretty_output, encoding="utf-8")

    if args.pretty:
        sys.stdout.write(pretty_output)
    else:
        sys.stdout.write(json_output)

    if args.enforce_coverage:
        if args.daemon_lcov and daemon_lcov is None:
            coverage_failures.append(f"missing daemon coverage input: {args.daemon_lcov}")
        if args.web_summary and web_summary is None:
            coverage_failures.append(f"missing web coverage input: {args.web_summary}")
    if args.enforce_coverage and coverage_failures:
        for failure in coverage_failures:
            print(f"error: {failure}", file=sys.stderr)
        return 1

    return 0


raise SystemExit(main())
PY
