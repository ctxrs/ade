#!/usr/bin/env bash
set -euo pipefail

dest="${1:-${CONTEXT_DOCS_OUTPUT_DIR:-}}"
if [[ -z "${dest}" ]]; then
  echo "Usage: fetch-react-docs.sh <output-dir>" >&2
  exit 1
fi

mkdir -p "${dest}/learn" "${dest}/reference/react" "${dest}/reference/react-dom"
base="https://raw.githubusercontent.com/reactjs/react.dev/main/src/content"

curl -fsSL "${base}/learn/index.md" -o "${dest}/learn/index.md"
curl -fsSL "${base}/learn/thinking-in-react.md" -o "${dest}/learn/thinking-in-react.md"
curl -fsSL "${base}/reference/react/index.md" -o "${dest}/reference/react/index.md"
curl -fsSL "${base}/reference/react-dom/index.md" -o "${dest}/reference/react-dom/index.md"
