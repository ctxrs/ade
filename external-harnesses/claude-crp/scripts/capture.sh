#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
PKG_DIR="${ROOT_DIR}/external-harnesses/claude-crp"
OUT_DIR="${PKG_DIR}/fixtures/inputs"
WORK_DIR="${PKG_DIR}/fixtures/tmp/workdir"
MODEL="${CLAUDE_CAPTURE_MODEL:-}"

mkdir -p "${OUT_DIR}" "${WORK_DIR}"

cat > "${WORK_DIR}/readme.txt" <<'TXT'
This is a sample file used for Claude CRP capture.
It has two short lines for deterministic tool output.
TXT

export WORK_DIR
python3 - <<'PY'
import os
path = os.environ.get("WORK_DIR")
if not path:
    raise SystemExit("WORK_DIR missing")
large_path = os.path.join(path, "large.txt")
with open(large_path, "w", encoding="utf-8") as fh:
    for i in range(1, 6001):
        fh.write(f"line {i:04d} - lorem ipsum dolor sit amet\n")
PY

run_capture() {
  local scenario="$1"
  local prompt="$2"
  local extra=()
  if [[ -n "${MODEL}" ]]; then
    extra+=(--model "${MODEL}")
  fi
  node "${PKG_DIR}/scripts/capture.mjs" \
    --scenario "${scenario}" \
    --prompt "${prompt}" \
    --cwd "${WORK_DIR}" \
    --out "${OUT_DIR}/${scenario}.jsonl" \
    --err "${OUT_DIR}/${scenario}.stderr" \
    --timeout-sec 180 \
    "${extra[@]}"
}

run_capture "simple_text" "Say hello in one sentence."
run_capture "multi_paragraph" "Write two short paragraphs about why good logging matters. End with a 3-item bullet list."
run_capture "tool_read" "Read the file ./readme.txt and summarize it in two sentences."
run_capture "tool_write" "Create ./output.txt with the exact text: 'hello from claude crp capture' and confirm the contents."
run_capture "tool_error_read" "Read the file ./missing.txt and report the error message only."
run_capture "bash_list" "Run 'ls -la' in the workspace and explain what you see in one sentence."
run_capture "large_output" "Read the file ./large.txt and report the first and last line only."

node "${PKG_DIR}/scripts/capture.mjs" \
  --scenario "interrupt" \
  --prompt "Explain the history of computing in detail." \
  --cwd "${WORK_DIR}" \
  --out "${OUT_DIR}/interrupt.jsonl" \
  --err "${OUT_DIR}/interrupt.stderr" \
  --timeout-sec 180 \
  --interrupt-after-ms 1500 \
  ${MODEL:+--model "${MODEL}"}

echo "fixtures written to ${OUT_DIR}"
