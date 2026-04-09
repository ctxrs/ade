#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MATRIX_JSON="$ROOT/core/crates/ctx-provider-accounts/src/provider_matrix.json"

python_cmd="python3"
if ! command -v "$python_cmd" >/dev/null 2>&1; then
  if command -v python >/dev/null 2>&1; then
    python_cmd="python"
  else
    echo "error: python3 (or python) is required for this regression test" >&2
    exit 2
  fi
fi

"$python_cmd" - "$MATRIX_JSON" <<'PY'
import json
import re
import sys
from pathlib import Path

matrix_path = Path(sys.argv[1])
matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
providers = matrix.get("providers") or []

entry = None
for provider in providers:
    if str(provider.get("id") or "").strip() == "codex":
        entry = provider
        break

errors: list[str] = []
if not isinstance(entry, dict):
    errors.append("missing codex provider entry")
else:
    managed = entry.get("managed_install") or {}
    if str(managed.get("kind") or "").strip() != "archive":
        errors.append("codex managed_install.kind must be archive")

    managed_version = str(managed.get("version") or "").strip()
    if not managed_version:
        errors.append("codex managed_install.version missing")

    releases = [r for r in (entry.get("releases") or []) if isinstance(r, dict)]
    supported = [r for r in releases if str(r.get("status") or "supported").strip() == "supported"]
    release = supported[0] if supported else (releases[0] if releases else None)
    if not isinstance(release, dict):
        errors.append("codex release metadata missing")
        release = {}

    release_version = str(release.get("version") or "").strip()
    if not release_version:
        errors.append("codex release.version missing")
    elif managed_version and release_version != managed_version:
        errors.append(f"codex release.version mismatch: {release_version} vs managed {managed_version}")

    upstream_version = str(release.get("upstream_version") or "").strip()

    provenance = release.get("provenance") or {}
    if not isinstance(provenance, dict):
        provenance = {}

    upstream_repo = str(provenance.get("upstream_repo") or "").strip()
    upstream_release_tag = str(provenance.get("upstream_release_tag") or "").strip()
    upstream_commit_sha = str(provenance.get("upstream_commit_sha") or "").strip()
    ctx_repo = str(provenance.get("ctx_repo") or "").strip()
    ctx_release_tag = str(provenance.get("ctx_release_tag") or "").strip()

    repo_pattern = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
    sha_pattern = re.compile(r"^[0-9a-f]{40}$", re.IGNORECASE)
    upstream_tag_pattern = re.compile(r"^rust-v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$", re.IGNORECASE)
    ctx_tag_pattern = re.compile(r"^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")

    if not upstream_repo or not repo_pattern.match(upstream_repo):
        errors.append("codex provenance.upstream_repo missing/invalid")
    elif upstream_repo != "openai/codex":
        errors.append(f"codex provenance.upstream_repo unexpected: {upstream_repo}")

    if not upstream_release_tag or not upstream_tag_pattern.match(upstream_release_tag):
        errors.append("codex provenance.upstream_release_tag missing/invalid")

    if not upstream_commit_sha or not sha_pattern.match(upstream_commit_sha):
        errors.append("codex provenance.upstream_commit_sha missing/invalid")

    if not ctx_repo or not repo_pattern.match(ctx_repo):
        errors.append("codex provenance.ctx_repo missing/invalid")
    elif ctx_repo != "ctxrs/codex-crp":
        errors.append(f"codex provenance.ctx_repo unexpected: {ctx_repo}")

    if not ctx_release_tag or not ctx_tag_pattern.match(ctx_release_tag):
        errors.append("codex provenance.ctx_release_tag missing/invalid")
    elif managed_version and ctx_release_tag != f"v{managed_version}":
        errors.append(
            f"codex provenance.ctx_release_tag mismatch: {ctx_release_tag} vs expected v{managed_version}"
        )

    if upstream_version and upstream_release_tag:
        upstream_from_tag = re.sub(r"^rust-v", "", upstream_release_tag, flags=re.IGNORECASE)
        if upstream_version != upstream_from_tag:
            errors.append(
                f"codex upstream_version mismatch: {upstream_version} vs {upstream_from_tag} from upstream_release_tag"
            )

    targets = managed.get("targets") or {}
    if not isinstance(targets, dict) or not targets:
        errors.append("codex managed_install.targets missing")
    else:
        expected_prefix = f"https://github.com/{ctx_repo}/releases/download/{ctx_release_tag}/" if ctx_repo and ctx_release_tag else ""
        for target_id, target in targets.items():
            if not isinstance(target, dict):
                errors.append(f"codex target entry invalid: {target_id}")
                continue
            url = str(target.get("url") or "").strip()
            if not url:
                errors.append(f"codex target url missing: {target_id}")
                continue
            if expected_prefix and not url.startswith(expected_prefix):
                errors.append(f"codex target url does not match ctx release tag: {target_id} -> {url}")
            if managed_version and f"codex-crp-{managed_version}" not in url:
                errors.append(f"codex target url missing managed version: {target_id} -> {url}")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print("ok: codex release provenance is pinned and internally consistent")
PY
