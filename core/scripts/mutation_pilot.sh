#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

out_dir="${CTX_MUTATION_OUT_DIR:-/tmp/ctx-mutation-critical.$(date +%Y%m%d-%H%M%S)}"
mkdir -p "${out_dir}"

report_json="${out_dir}/report.json"
report_txt="${out_dir}/report.txt"

mutations=(
$'store_fault_injection_no_error\tstore\tcrates/ctx-store/src/fault_injection.rs\tErr(anyhow::anyhow!("fault injection: {point}"))\tOk(())\tcargo test -p ctx-store --features fault_injection'
$'http_fault_injection_no_error\thttp\tcrates/ctx-http/src/fault_injection.rs\tErr(anyhow::anyhow!("fault injection: {point}"))\tOk(())\tcargo test -p ctx-http --features fault_injection --test fault_matrix'
$'store_snapshot_keeps_assistant_partial\tstore\tcrates/ctx-store/src/store/mod.rs\tturn.assistant_partial = None;\tturn.assistant_partial = turn.assistant_partial.clone();\tcargo test -p ctx-store session_head_snapshot_excludes_assistant_partials'
$'store_snapshot_keeps_assistant_chunk_events\tstore\tcrates/ctx-store/src/store/mod.rs\tSessionEventType::AssistantChunk | SessionEventType::ThoughtChunk\tSessionEventType::Notice | SessionEventType::ThoughtChunk\tcargo test -p ctx-store session_head_snapshot_excludes_assistant_partials'
$'updater_non_linux_in_place_allowed\tupdater-http\tcrates/ctx-http/src/api/updates.rs\tif !platform_key.starts_with("linux-") {\tif false {\tcargo test -p ctx-http --lib in_place_update_capability_is_false_for_non_linux_platforms'
$'updater_unsupported_platform_allows_update\tupdater-http\tcrates/ctx-http/src/api/updates.rs\tif !platform_supported {\tif false {\tcargo test -p ctx-http --lib in_place_update_capability_is_false_when_platform_is_not_supported'
$'projection_session_gap_no_flush\tprojection-web\tapps/web/src/state/workspaceActiveSnapshotStoreCore.ts\tthis.flushSubscriptions("session_gap");\tthis.publish();\tpnpm -C apps/web exec vitest run src/state/workspaceActiveSnapshotStore.test.ts --silent --reporter=dot'
$'updater_restart_gate_inverted\tupdater-web\tapps/web/src/components/UpdateNoticeBanner.tsx\tif (resp.needs_restart) {\tif (!resp.needs_restart) {\tpnpm -C apps/web exec vitest run src/components/UpdateNoticeBanner.test.tsx --silent --reporter=dot'
)

echo "critical mutation output: ${out_dir}"

python3 - "$report_json" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, "w", encoding="utf-8") as f:
    json.dump({"mutants": []}, f, indent=2)
PY

append_report() {
  python3 - "$report_json" "$1" "$2" "$3" "$4" "$5" "$6" "$7" <<'PY'
import json
import sys

path, name, category, file_path, status, command, log_path, notes = sys.argv[1:9]
with open(path, "r", encoding="utf-8") as f:
    payload = json.load(f)
payload["mutants"].append(
    {
        "name": name,
        "category": category,
        "file": file_path,
        "status": status,
        "command": command,
        "log_path": log_path,
        "notes": notes,
    }
)
with open(path, "w", encoding="utf-8") as f:
    json.dump(payload, f, indent=2)
PY
}

run_mutation() {
  local name="$1"
  local category="$2"
  local file="$3"
  local needle="$4"
  local repl="$5"
  local command="$6"

  local backup="${out_dir}/${name}.bak"
  local log_path="${out_dir}/${name}.log"

  cp "${file}" "${backup}"

  restore() {
    cp "${backup}" "${file}"
  }

  if ! grep -Fq "${needle}" "${file}"; then
    append_report "${name}" "${category}" "${file}" "skipped" "${command}" "${log_path}" "needle not found"
    restore
    return 0
  fi

  python3 - "$file" "$needle" "$repl" <<'PY'
import sys

path, needle, repl = sys.argv[1:4]
with open(path, "r", encoding="utf-8") as f:
    data = f.read()
if needle not in data:
    raise SystemExit("needle missing")
with open(path, "w", encoding="utf-8") as f:
    f.write(data.replace(needle, repl, 1))
PY

  set +e
  CTX_MUTATION_COMMAND="${command}" \
  CTX_MUTATION_LOG_PATH="${log_path}" \
  CTX_MUTATION_TIMEOUT_SEC="${CTX_MUTATION_TIMEOUT_SEC:-180}" \
    python3 - <<'PY'
import os
import subprocess
import sys

command = os.environ["CTX_MUTATION_COMMAND"]
log_path = os.environ["CTX_MUTATION_LOG_PATH"]
timeout_sec = int(os.environ.get("CTX_MUTATION_TIMEOUT_SEC", "180"))

with open(log_path, "w", encoding="utf-8") as log:
    try:
        completed = subprocess.run(
            ["bash", "-lc", command],
            stdout=log,
            stderr=subprocess.STDOUT,
            timeout=timeout_sec,
        )
    except subprocess.TimeoutExpired:
        log.write(f"\nmutation command timed out after {timeout_sec}s\n")
        sys.exit(124)
sys.exit(completed.returncode)
PY
  rc=$?
  set -e

  restore

  if [[ "${rc}" -eq 124 ]]; then
    append_report "${name}" "${category}" "${file}" "timed_out" "${command}" "${log_path}" "tests timed out under mutation"
  elif [[ "${rc}" -ne 0 ]]; then
    append_report "${name}" "${category}" "${file}" "killed" "${command}" "${log_path}" "tests failed under mutation"
  else
    append_report "${name}" "${category}" "${file}" "survived" "${command}" "${log_path}" "tests passed under mutation"
  fi
}

for spec in "${mutations[@]}"; do
  IFS=$'\t' read -r name category file needle repl command <<<"${spec}"
  run_mutation "${name}" "${category}" "${file}" "${needle}" "${repl}" "${command}"
done

python3 - "$report_json" "$report_txt" <<'PY'
import json
import sys
from collections import defaultdict

report_path, txt_path = sys.argv[1:3]
with open(report_path, "r", encoding="utf-8") as f:
    payload = json.load(f)

mutants = payload.get("mutants", [])
eligible = [m for m in mutants if m["status"] != "skipped"]
killed = sum(1 for m in eligible if m["status"] == "killed")
survived = sum(1 for m in eligible if m["status"] == "survived")
timed_out = sum(1 for m in eligible if m["status"] == "timed_out")
skipped = sum(1 for m in mutants if m["status"] == "skipped")
score_pct = round((killed / len(eligible) * 100.0), 2) if eligible else 0.0

categories = defaultdict(
    lambda: {"eligible": 0, "killed": 0, "survived": 0, "timed_out": 0, "skipped": 0}
)
for mutant in mutants:
    bucket = categories[mutant["category"]]
    if mutant["status"] == "skipped":
        bucket["skipped"] += 1
        continue
    bucket["eligible"] += 1
    if mutant["status"] == "killed":
        bucket["killed"] += 1
    elif mutant["status"] == "survived":
        bucket["survived"] += 1
    elif mutant["status"] == "timed_out":
        bucket["timed_out"] += 1

summary = {
    "total_mutants": len(mutants),
    "eligible_mutants": len(eligible),
    "killed_mutants": killed,
    "survived_mutants": survived,
    "timed_out_mutants": timed_out,
    "skipped_mutants": skipped,
    "score_pct": score_pct,
    "categories": categories,
}
payload["summary"] = summary

with open(report_path, "w", encoding="utf-8") as f:
    json.dump(payload, f, indent=2)

with open(txt_path, "w", encoding="utf-8") as f:
    f.write(
        "eligible={eligible} killed={killed} survived={survived} timed_out={timed_out} skipped={skipped} score_pct={score_pct:.2f}\n".format(
            eligible=len(eligible),
            killed=killed,
            survived=survived,
            timed_out=timed_out,
            skipped=skipped,
            score_pct=score_pct,
        )
    )
    for category in sorted(categories.keys()):
        bucket = categories[category]
        f.write(
            "- {category}: eligible={eligible} killed={killed} survived={survived} timed_out={timed_out} skipped={skipped}\n".format(
                category=category,
                eligible=bucket["eligible"],
                killed=bucket["killed"],
                survived=bucket["survived"],
                timed_out=bucket["timed_out"],
                skipped=bucket["skipped"],
            )
        )
    for mutant in mutants:
        f.write(
            "- {name}: {status} [{category}] ({file})\n".format(
                name=mutant["name"],
                status=mutant["status"],
                category=mutant["category"],
                file=mutant["file"],
            )
        )

print(
    "critical mutation summary: eligible={eligible} killed={killed} survived={survived} timed_out={timed_out} skipped={skipped} score_pct={score_pct:.2f}".format(
        eligible=len(eligible),
        killed=killed,
        survived=survived,
        timed_out=timed_out,
        skipped=skipped,
        score_pct=score_pct,
    )
)
print(f"report json: {report_path}")
print(f"report txt:  {txt_path}")

if survived or timed_out:
    raise SystemExit(1)
PY
