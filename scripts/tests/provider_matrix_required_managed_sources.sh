#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MATRIX_JSON="$ROOT/core/crates/ctx-http/src/provider_matrix.json"
LOCK_JSON="$ROOT/core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json"

python_cmd="python3"
if ! command -v "$python_cmd" >/dev/null 2>&1; then
  if command -v python >/dev/null 2>&1; then
    python_cmd="python"
  else
    echo "error: python3 (or python) is required for this regression test" >&2
    exit 2
  fi
fi

"$python_cmd" - "$MATRIX_JSON" "$LOCK_JSON" <<'PY'
import json
import os
import re
import sys
from pathlib import Path

matrix_path = Path(sys.argv[1])
lock_path = Path(sys.argv[2])

matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
lock = json.loads(lock_path.read_text(encoding="utf-8"))

providers = {str(p.get("id", "")): p for p in (matrix.get("providers") or [])}
env_required = [
    value.strip()
    for value in str(os.environ.get("ARCHIVE_REQUIRED_PROVIDERS", "")).split(",")
    if value.strip()
]
lock_required = [str(v) for v in (lock.get("required", {}).get("provider_ids") or []) if str(v)]
if env_required:
    required = env_required
elif lock_required:
    required = lock_required
else:
    required = sorted(
        provider_id
        for provider_id, entry in providers.items()
        if isinstance(entry.get("managed_install"), dict)
        and str(entry.get("managed_install", {}).get("kind", "")).strip() == "archive"
    )

errors: list[str] = []

for provider_id in required:
    entry = providers.get(provider_id)
    if not entry:
        errors.append(f"missing provider matrix entry: {provider_id}")
        continue

    managed = entry.get("managed_install")
    if not isinstance(managed, dict):
        errors.append(f"provider missing managed_install: {provider_id}")
        continue

    kind = str(managed.get("kind") or "").strip()
    if not kind:
        errors.append(f"provider managed_install.kind missing: {provider_id}")
        continue

    releases = entry.get("releases") or []
    if not isinstance(releases, list) or not releases:
        errors.append(f"provider releases missing: {provider_id}")
    else:
        if not any(str(r.get("version", "")).strip() for r in releases if isinstance(r, dict)):
            errors.append(f"provider releases have no pinned version values: {provider_id}")

    if kind in {"archive", "python"}:
        if not str(managed.get("version", "")).strip():
            errors.append(f"provider managed_install.version missing: {provider_id}")
    elif kind == "npm":
        if not str(managed.get("package", "")).strip():
            errors.append(f"provider npm package missing: {provider_id}")
        if not str(managed.get("entrypoint", "")).strip():
            errors.append(f"provider npm entrypoint missing: {provider_id}")
    else:
        errors.append(f"provider managed_install.kind unsupported: {provider_id} ({kind})")

    if provider_id == "claude-crp":
        if kind != "archive":
            errors.append("provider claude-crp must remain managed_install.kind=archive")
        targets = managed.get("targets") if isinstance(managed, dict) else {}
        if not isinstance(targets, dict):
            errors.append("provider claude-crp managed_install.targets missing")
            targets = {}
        required_targets = ["darwin-aarch64", "darwin-x86_64", "linux-aarch64", "linux-x86_64"]
        for target_key in required_targets:
            target = targets.get(target_key)
            if not isinstance(target, dict):
                errors.append(f"provider claude-crp missing managed target: {target_key}")
                continue
            if not str(target.get("url", "")).strip():
                errors.append(f"provider claude-crp target {target_key} missing url")
            if str(target.get("archive", "")).strip() != "tar_gz":
                errors.append(f"provider claude-crp target {target_key} archive must be tar_gz")
            if str(target.get("bin_path", "")).strip() != "bin/claude-crp":
                errors.append(f"provider claude-crp target {target_key} bin_path must be bin/claude-crp")
            sha = str(target.get("sha256", "")).strip().lower()
            if not re.match(r"^[0-9a-f]{64}$", sha):
                errors.append(f"provider claude-crp target {target_key} missing/invalid sha256")

    if provider_id == "codex":
        releases = [r for r in releases if isinstance(r, dict)]
        release = releases[0] if releases else {}
        provenance = release.get("provenance") if isinstance(release, dict) else {}
        if not isinstance(provenance, dict):
            provenance = {}

        upstream_repo = str(provenance.get("upstream_repo") or "").strip()
        upstream_release_tag = str(provenance.get("upstream_release_tag") or "").strip()
        upstream_commit_sha = str(provenance.get("upstream_commit_sha") or "").strip()
        ctx_repo = str(provenance.get("ctx_repo") or "").strip()
        ctx_release_tag = str(provenance.get("ctx_release_tag") or "").strip()

        if not re.match(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$", upstream_repo):
            errors.append("provider codex provenance.upstream_repo missing/invalid")
        if not re.match(r"^rust-v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$", upstream_release_tag):
            errors.append("provider codex provenance.upstream_release_tag missing/invalid")
        if not re.match(r"^[0-9a-fA-F]{40}$", upstream_commit_sha):
            errors.append("provider codex provenance.upstream_commit_sha missing/invalid")
        if not re.match(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$", ctx_repo):
            errors.append("provider codex provenance.ctx_repo missing/invalid")
        if not re.match(r"^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$", ctx_release_tag):
            errors.append("provider codex provenance.ctx_release_tag missing/invalid")

        managed_version = str(managed.get("version") or "").strip()
        release_version = str(release.get("version") or "").strip()
        if managed_version and release_version and managed_version != release_version:
            errors.append(
                f"provider codex release.version mismatch ({release_version} vs managed {managed_version})"
            )
        if managed_version and ctx_release_tag and ctx_release_tag != f"v{managed_version}":
            errors.append(
                f"provider codex provenance.ctx_release_tag mismatch ({ctx_release_tag} vs expected v{managed_version})"
            )

        targets = managed.get("targets") if isinstance(managed, dict) else {}
        if not isinstance(targets, dict) or not targets:
            errors.append("provider codex managed_install.targets missing")
        else:
            expected_prefix = (
                f"https://github.com/{ctx_repo}/releases/download/{ctx_release_tag}/"
                if ctx_repo and ctx_release_tag
                else ""
            )
            for target_id, target in targets.items():
                if not isinstance(target, dict):
                    errors.append(f"provider codex target invalid ({target_id})")
                    continue
                url = str(target.get("url") or "").strip()
                if not url:
                    errors.append(f"provider codex target URL missing ({target_id})")
                    continue
                if expected_prefix and not url.startswith(expected_prefix):
                    errors.append(
                        f"provider codex target URL does not match ctx release tag ({target_id})"
                    )
                if managed_version and f"codex-crp-{managed_version}" not in url:
                    errors.append(
                        f"provider codex target URL missing managed version ({target_id})"
                    )

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"ok: required providers have managed source + pinned versions ({len(required)})")
PY
