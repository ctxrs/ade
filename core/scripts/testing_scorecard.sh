#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: testing_scorecard.sh [--jsonl <path>]... [--pretty]

Summarize ci_retry flake telemetry jsonl files.

Options:
  --jsonl <path>   Add a ci-flake-stats.jsonl input file (repeatable).
  --pretty         Print a human-readable table summary.
  -h, --help       Show this help.

If no --jsonl arguments are provided, defaults to:
  ./ci-flake-stats.jsonl
EOF
}

pretty=0
declare -a files=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --jsonl)
      if [[ $# -lt 2 ]]; then
        echo "error: --jsonl requires a path" >&2
        exit 2
      fi
      files+=("$2")
      shift 2
      ;;
    --pretty)
      pretty=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${#files[@]}" -eq 0 ]]; then
  files=("ci-flake-stats.jsonl")
fi

for f in "${files[@]}"; do
  if [[ ! -f "$f" ]]; then
    echo "error: missing input file: $f" >&2
    exit 1
  fi
done

python3 - "$pretty" "${files[@]}" <<'PY'
import json
import sys
from collections import defaultdict

pretty = sys.argv[1] == "1"
paths = sys.argv[2:]

rows = []
for path in paths:
    with open(path, "r", encoding="utf-8") as f:
        for lineno, raw in enumerate(f, start=1):
            raw = raw.strip()
            if not raw:
                continue
            try:
                row = json.loads(raw)
            except json.JSONDecodeError as e:
                raise SystemExit(f"error: {path}:{lineno}: invalid json: {e}")
            row["_path"] = path
            rows.append(row)

if not rows:
    raise SystemExit("error: no telemetry rows found in input files")

by_job = defaultdict(lambda: {
    "total": 0,
    "failed": 0,
    "flake": 0,
    "flake_failed": 0,
    "attempts_sum": 0,
    "attempts_max": 0,
})

for row in rows:
    job = row.get("job", "unknown")
    status = row.get("status", "unknown")
    attempts = int(row.get("attempts", 0))
    s = by_job[job]
    s["total"] += 1
    s["attempts_sum"] += attempts
    s["attempts_max"] = max(s["attempts_max"], attempts)
    if status == "failed":
        s["failed"] += 1
    elif status == "flake":
        s["flake"] += 1
    elif status == "flake_failed":
        s["flake_failed"] += 1

summary = {
    "files": paths,
    "rows": len(rows),
    "jobs": {},
}

for job in sorted(by_job.keys()):
    s = by_job[job]
    total = s["total"]
    summary["jobs"][job] = {
        "total_runs": total,
        "failed_runs": s["failed"],
        "flake_runs": s["flake"],
        "flake_failed_runs": s["flake_failed"],
        "pass_without_flake_runs": max(total - s["failed"] - s["flake"] - s["flake_failed"], 0),
        "flake_rate": round((s["flake"] + s["flake_failed"]) / total, 4) if total else 0.0,
        "failure_rate": round(s["failed"] / total, 4) if total else 0.0,
        "avg_attempts": round(s["attempts_sum"] / total, 3) if total else 0.0,
        "max_attempts": s["attempts_max"],
    }

if pretty:
    print(f"rows={summary['rows']} files={len(summary['files'])}")
    print(
        "job\t"
        "runs\t"
        "pass_clean\t"
        "failed\t"
        "flake\t"
        "flake_failed\t"
        "flake_rate\t"
        "failure_rate\t"
        "avg_attempts\t"
        "max_attempts"
    )
    for job, s in summary["jobs"].items():
        print(
            f"{job}\t"
            f"{s['total_runs']}\t"
            f"{s['pass_without_flake_runs']}\t"
            f"{s['failed_runs']}\t"
            f"{s['flake_runs']}\t"
            f"{s['flake_failed_runs']}\t"
            f"{s['flake_rate']}\t"
            f"{s['failure_rate']}\t"
            f"{s['avg_attempts']}\t"
            f"{s['max_attempts']}"
        )
else:
    print(json.dumps(summary, indent=2, sort_keys=True))
PY
