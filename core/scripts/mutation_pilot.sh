#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

out_dir="${CTX_MUTATION_OUT_DIR:-/tmp/ctx-mutation-pilot.$(date +%Y%m%d-%H%M%S)}"
mkdir -p "${out_dir}"

report_json="${out_dir}/report.json"
report_txt="${out_dir}/report.txt"

mutations=(
  "crates/ctx-http/src/fault_injection.rs|Err(anyhow::anyhow!(\"fault injection: {point}\"))|Ok(())|http_fault_injection_no_error|cargo test -p ctx-http --features fault_injection --test fault_matrix"
  "crates/ctx-store/src/fault_injection.rs|Err(anyhow::anyhow!(\"fault injection: {point}\"))|Ok(())|store_fault_injection_no_error|cargo test -p ctx-store --features fault_injection"
  "crates/ctx-http/src/fault_injection.rs|guard.insert(point, times);|guard.remove(point);|http_set_failpoint_removed|cargo test -p ctx-http --features fault_injection --test hot_endpoints_no_db"
)

echo "mutation pilot output: ${out_dir}"

python3 - "$report_json" <<'PY'
import json,sys
path=sys.argv[1]
with open(path,"w",encoding="utf-8") as f:
    json.dump({"mutants":[]},f,indent=2)
PY

append_report() {
  python3 - "$report_json" "$1" "$2" "$3" "$4" "$5" <<'PY'
import json,sys
path,file,name,status,command,notes=sys.argv[1:7]
with open(path,"r",encoding="utf-8") as f:
    payload=json.load(f)
payload["mutants"].append({
    "file": file,
    "name": name,
    "status": status,
    "command": command,
    "notes": notes,
})
with open(path,"w",encoding="utf-8") as f:
    json.dump(payload,f,indent=2)
PY
}

run_mutation() {
  local file="$1"
  local needle="$2"
  local repl="$3"
  local name="$4"
  local command="$5"

  local backup="${out_dir}/${name}.bak"
  cp "${file}" "${backup}"
  restore() {
    cp "${backup}" "${file}"
  }

  if ! grep -Fq "${needle}" "${file}"; then
    append_report "${file}" "${name}" "skipped" "${command}" "needle not found"
    return 0
  fi

  python3 - "$file" "$needle" "$repl" <<'PY'
import sys
path,needle,repl=sys.argv[1:4]
with open(path,"r",encoding="utf-8") as f:
    data=f.read()
if needle not in data:
    raise SystemExit("needle missing")
with open(path,"w",encoding="utf-8") as f:
    f.write(data.replace(needle,repl,1))
PY

  set +e
  bash -lc "${command}" >/tmp/"${name}".mutation.log 2>&1
  rc=$?
  set -e

  restore

  if [[ "${rc}" -ne 0 ]]; then
    append_report "${file}" "${name}" "killed" "${command}" "tests failed under mutation"
  else
    append_report "${file}" "${name}" "survived" "${command}" "tests passed under mutation"
  fi
}

for spec in "${mutations[@]}"; do
  IFS='|' read -r file needle repl name command <<<"${spec}"
  run_mutation "${file}" "${needle}" "${repl}" "${name}" "${command}"
done

python3 - "$report_json" "$report_txt" <<'PY'
import json,sys
report_path,txt_path=sys.argv[1:3]
with open(report_path,"r",encoding="utf-8") as f:
    payload=json.load(f)
mutants=payload.get("mutants",[])
killed=sum(1 for m in mutants if m["status"]=="killed")
survived=sum(1 for m in mutants if m["status"]=="survived")
skipped=sum(1 for m in mutants if m["status"]=="skipped")
with open(txt_path,"w",encoding="utf-8") as f:
    f.write(f"mutants={len(mutants)} killed={killed} survived={survived} skipped={skipped}\n")
    for m in mutants:
        f.write(f"- {m['name']}: {m['status']} ({m['file']})\n")
print(f"mutation pilot summary: mutants={len(mutants)} killed={killed} survived={survived} skipped={skipped}")
print(f"report json: {report_path}")
print(f"report txt:  {txt_path}")
PY
