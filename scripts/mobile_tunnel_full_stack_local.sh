#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORE_DIR="$ROOT/core"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ctx-mobile-tunnel-local.XXXXXX")"
PIDS=()

dump_logs() {
  local log
  echo "diagnostics: temp dir ${TMP_ROOT}" >&2
  for log in "$TMP_ROOT"/*.log; do
    if [[ -f "$log" ]]; then
      echo "== ${log##*/} ==" >&2
      tail -n 80 "$log" >&2 || true
    fi
  done
}

cleanup() {
  local pid
  for pid in "${PIDS[@]:-}"; do
    if kill -0 "$pid" >/dev/null 2>&1; then
      kill "$pid" >/dev/null 2>&1 || true
    fi
  done
  rm -rf "$TMP_ROOT"
}
on_exit() {
  local status=$?
  if (( status != 0 )); then
    dump_logs
  fi
  cleanup
  exit "$status"
}
trap on_exit EXIT

require_command() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "error: ${name} is required" >&2
    exit 1
  fi
}

pick_port() {
  node - <<'NODE'
const net = require("node:net");
const server = net.createServer();
server.listen(0, "127.0.0.1", () => {
  const address = server.address();
  console.log(address.port);
  server.close();
});
NODE
}

wait_for_http() {
  local name="$1"
  local url="$2"
  local deadline=$((SECONDS + ${CTX_MOBILE_TUNNEL_WAIT_SECS:-120}))
  until curl -fsS --max-time 2 "$url" >/dev/null 2>&1; do
    if (( SECONDS > deadline )); then
      echo "error: timed out waiting for ${name} at ${url}" >&2
      exit 1
    fi
    sleep 0.5
  done
  echo "ok: ${name} ${url}"
}

require_command cargo
require_command curl
require_command git
require_command node

AUTH_PORT="$(pick_port)"
CONTROL_PORT="$(pick_port)"
ROUTER_PORT="$(pick_port)"
RELAY_PORT="$(pick_port)"
DB_PATH="$TMP_ROOT/control-plane.sqlite"
MASTER_SECRET="local-mobile-tunnel-secret-${RANDOM}-${RANDOM}"

cat >"$TMP_ROOT/mock_auth.js" <<'NODE'
const http = require("node:http");

const expected = process.env.EXPECTED_TOKEN || "local-token";
const port = Number(process.env.PORT);

const json = (res, status, body) => {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};

const server = http.createServer((req, res) => {
  if (req.url === "/health") {
    json(res, 200, { ok: true });
    return;
  }
  if (req.headers.authorization !== `Bearer ${expected}`) {
    res.writeHead(401);
    res.end();
    return;
  }
  if (req.url.startsWith("/auth/v1/user")) {
    json(res, 200, { id: "local-user" });
    return;
  }
  if (req.url.startsWith("/functions/v1/entitlements")) {
    json(res, 200, {
      plan_type: "pro",
      features: { remote_mobile_access: "enabled" },
    });
    return;
  }
  res.writeHead(404);
  res.end();
});

server.listen(port, "127.0.0.1");
NODE

PORT="$AUTH_PORT" EXPECTED_TOKEN="local-token" node "$TMP_ROOT/mock_auth.js" &
PIDS+=("$!")
wait_for_http "mock auth" "http://127.0.0.1:${AUTH_PORT}/health"

(
  cd "$CORE_DIR"
  CONTROL_PLANE_DATABASE_URL="sqlite://${DB_PATH}?mode=rwc" \
  SUPABASE_URL="http://127.0.0.1:${AUTH_PORT}" \
  SUPABASE_ANON_KEY="local-anon" \
  CONTROL_PLANE_ENTITLEMENTS_URL="http://127.0.0.1:${AUTH_PORT}/functions/v1/entitlements" \
  MOBILE_TUNNEL_PUBLIC_BASE_URL="http://127.0.0.1:${ROUTER_PORT}" \
  MOBILE_TUNNEL_RELAY_BASE_URLS="http://127.0.0.1:${RELAY_PORT}" \
  CTX_TUNNEL_MASTER_SECRET="$MASTER_SECRET" \
  cargo run -p ctx-tunnel-control-plane -- --listen "127.0.0.1:${CONTROL_PORT}"
) >"$TMP_ROOT/control-plane.log" 2>&1 &
PIDS+=("$!")

wait_for_http "control-plane" "http://127.0.0.1:${CONTROL_PORT}/health"

(
  cd "$CORE_DIR"
  CONTROL_PLANE_DATABASE_URL="sqlite://${DB_PATH}?mode=rwc" \
  cargo run -p ctx-tunnel-router -- --listen "127.0.0.1:${ROUTER_PORT}"
) >"$TMP_ROOT/router.log" 2>&1 &
PIDS+=("$!")

(
  cd "$CORE_DIR"
  CTX_TUNNEL_MASTER_SECRET="$MASTER_SECRET" \
  cargo run -p ctx-tunnel-relay -- --listen "127.0.0.1:${RELAY_PORT}"
) >"$TMP_ROOT/relay.log" 2>&1 &
PIDS+=("$!")

wait_for_http "router" "http://127.0.0.1:${ROUTER_PORT}/health"
wait_for_http "relay" "http://127.0.0.1:${RELAY_PORT}/health"

(
  cd "$CORE_DIR"
  CTX_TUNNEL_CONTROL_PLANE_URL="http://127.0.0.1:${CONTROL_PORT}" \
  CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK=1 \
  CTX_SUPABASE_ACCESS_TOKEN="local-token" \
  cargo run -p ctx-http --bin mobile_e2e
)
