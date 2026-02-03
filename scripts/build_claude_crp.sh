#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="${ROOT_DIR}/external-harnesses/claude-crp"

cd "${WORKSPACE}"
pnpm install --frozen-lockfile
pnpm build
