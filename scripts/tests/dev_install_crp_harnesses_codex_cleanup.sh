#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

tmp="$(mktemp -d /tmp/ctx-dev-install-codex-cleanup.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

fake_root="$tmp/repo"
mkdir -p \
  "$fake_root/scripts" \
  "$fake_root/external-harnesses/claude-crp/bin" \
  "$fake_root/external-harnesses/claude-crp/dist" \
  "$fake_root/data/providers/agent-servers"

cp "$ROOT/scripts/dev_install_crp_harnesses.sh" "$fake_root/scripts/dev_install_crp_harnesses.sh"
chmod +x "$fake_root/scripts/dev_install_crp_harnesses.sh"
printf '#!/usr/bin/env bash\nexit 0\n' > "$fake_root/external-harnesses/claude-crp/bin/claude-crp"
chmod +x "$fake_root/external-harnesses/claude-crp/bin/claude-crp"
printf 'module.exports = {};\n' > "$fake_root/external-harnesses/claude-crp/dist/runtime.js"

cat > "$fake_root/data/providers/agent-servers/agent_servers.json" <<'JSON'
{
  "providers": {
    "codex-crp": {
      "command": "/tmp/stale-codex-crp",
      "args": [],
      "dependencies": []
    }
  },
  "managed_installs": {
    "existing": {
      "package": "keep-me"
    }
  }
}
JSON

CTX_DATA_DIR="$fake_root/data" \
CTX_INSTALL_LOCAL_CODEX_CRP=0 \
"$fake_root/scripts/dev_install_crp_harnesses.sh"

python3 - "$fake_root/data/providers/agent-servers/agent_servers.json" <<'PY'
import json
import sys
from pathlib import Path

cfg = json.loads(Path(sys.argv[1]).read_text())
providers = cfg.get("providers") or {}
if "codex-crp" in providers:
    print("error: stale codex override was not removed", file=sys.stderr)
    raise SystemExit(1)
claude = providers.get("claude-crp")
if not isinstance(claude, dict) or not str(claude.get("command", "")).endswith("/external-harnesses/claude-crp/bin/claude-crp"):
    print(f"error: claude-crp entry missing or unexpected: {claude!r}", file=sys.stderr)
    raise SystemExit(1)
managed = cfg.get("managed_installs") or {}
if managed.get("existing", {}).get("package") != "keep-me":
    print(f"error: managed installs were not preserved: {managed!r}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: default dev_install_crp_harnesses removes stale codex override"
