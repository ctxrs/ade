#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORE_DIR="$ROOT/core"
TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ctx-mobile-tunnel-local.XXXXXX")"
PIDS=()
POSTGRES_CONTAINER=""

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
  if [[ -n "$POSTGRES_CONTAINER" ]]; then
    docker rm -f "$POSTGRES_CONTAINER" >/dev/null 2>&1 || true
  fi
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
require_command docker
require_command git
require_command node

CONTROL_PORT="$(pick_port)"
ROUTER_PORT="$(pick_port)"
RELAY_PORT="$(pick_port)"
PG_PORT="$(pick_port)"
MASTER_SECRET="local-mobile-tunnel-secret-${RANDOM}-${RANDOM}"
POSTGRES_CONTAINER="ctx-mobile-tunnel-pg-${RANDOM}-${RANDOM}"
DATABASE_URL="postgresql://ctx_mobile_tunnel:ctx_mobile_tunnel_local@127.0.0.1:${PG_PORT}/postgres?sslmode=disable"
MANAGED_TUNNEL_GRANT="ctmt_local_mobile_tunnel_grant"
MANAGED_TUNNEL_GRANT_DIGEST="$(CTX_MANAGED_TUNNEL_GRANT="$MANAGED_TUNNEL_GRANT" node - <<'NODE'
const { createHash } = require("node:crypto");
console.log(createHash("sha256").update(process.env.CTX_MANAGED_TUNNEL_GRANT, "utf8").digest("base64url"));
NODE
)"
MANAGED_TUNNEL_BINDING_DIGEST="$(CTX_MANAGED_TUNNEL_GRANT="$MANAGED_TUNNEL_GRANT" node - <<'NODE'
const { createHash } = require("node:crypto");
console.log(createHash("sha256").update(process.env.CTX_MANAGED_TUNNEL_GRANT, "utf8").digest("hex"));
NODE
)"
MANAGED_TUNNEL_DAEMON_ID="ctmt-daemon-${MANAGED_TUNNEL_BINDING_DIGEST}"
MANAGED_TUNNEL_DEVICE_ID="ctmt-device-${MANAGED_TUNNEL_BINDING_DIGEST}"

docker run --rm -d \
  --name "$POSTGRES_CONTAINER" \
  -e POSTGRES_PASSWORD=postgres \
  -p "127.0.0.1:${PG_PORT}:5432" \
  postgres:16-alpine >/dev/null

until docker exec "$POSTGRES_CONTAINER" pg_isready -U postgres >/dev/null 2>&1; do
  sleep 0.5
done

for migration in "$ROOT"/neon/migrations/*.sql; do
  docker exec -i "$POSTGRES_CONTAINER" psql -U postgres -d postgres < "$migration" >/dev/null
done
docker exec "$POSTGRES_CONTAINER" psql -U postgres -d postgres \
  -c "alter role ctx_mobile_tunnel with password 'ctx_mobile_tunnel_local';" >/dev/null
docker exec -i "$POSTGRES_CONTAINER" psql -U postgres -d postgres >/dev/null <<SQL
insert into ctx.ctx_users (id, primary_email, display_name)
values ('11111111-1111-1111-1111-111111111111', 'contact-265514505e6c@fixture.example.test', 'Local Mobile')
on conflict (id) do nothing;

insert into ctx.ctx_accounts (id, ctx_user_id, account_handle)
values ('22222222-2222-2222-2222-222222222222', '11111111-1111-1111-1111-111111111111', 'local-mobile')
on conflict (id) do nothing;

insert into ctx.billing_subjects (id, subject_kind, ctx_account_id)
values ('33333333-3333-3333-3333-333333333333', 'personal', '22222222-2222-2222-2222-222222222222')
on conflict (id) do nothing;

insert into ctx.billing_entitlements
  (id, billing_subject_id, entitlement_key, entitlement_value, source, active, valid_from)
values
  ('44444444-4444-4444-4444-444444444441', '33333333-3333-3333-3333-333333333333', 'remote_mobile_access', '{"state":"enabled"}'::jsonb, 'manual', true, now()),
  ('44444444-4444-4444-4444-444444444442', '33333333-3333-3333-3333-333333333333', 'mobile_relay', '{"state":"enabled"}'::jsonb, 'manual', true, now())
on conflict (id) do nothing;

insert into ctx.mobile_tunnel_grants
  (grant_id, grant_jti, audience, ctx_user_id, ctx_account_id, ctx_org_id, billing_subject_id, daemon_id, device_id, entitlement_version, scopes, grant_digest, issued_at, expires_at)
values
  ('55555555-5555-5555-5555-555555555555', 'local-mobile-grant', 'ctx-mobile-tunnel', '11111111-1111-1111-1111-111111111111', '22222222-2222-2222-2222-222222222222', null, '33333333-3333-3333-3333-333333333333', '${MANAGED_TUNNEL_DAEMON_ID}', '${MANAGED_TUNNEL_DEVICE_ID}', 'local', ARRAY['mobile_tunnel:enable']::text[], '${MANAGED_TUNNEL_GRANT_DIGEST}', now() - interval '1 minute', now() + interval '1 hour')
on conflict (grant_jti) do update
set grant_digest = excluded.grant_digest,
    expires_at = excluded.expires_at,
    revoked_at = null;
SQL
echo "ok: postgres 127.0.0.1:${PG_PORT}"

(
  cd "$CORE_DIR"
  MOBILE_TUNNEL_DATABASE_URL="$DATABASE_URL" \
  CTX_TUNNEL_RELAY_ID="relay-1" \
  CTX_TUNNEL_RELAY_REGION="us" \
  CTX_TUNNEL_RELAY_PUBLIC_BASE_URL="http://127.0.0.1:${RELAY_PORT}" \
  CTX_TUNNEL_RELAY_INTERNAL_BASE_URL="http://127.0.0.1:${RELAY_PORT}" \
  CTX_TUNNEL_RELAY_MAX_ACTIVE_TUNNELS=100 \
  CTX_TUNNEL_MASTER_SECRET="$MASTER_SECRET" \
  node "$CORE_DIR/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$CORE_DIR" -- \
    cargo run -p ctx-tunnel-relay -- --listen "127.0.0.1:${RELAY_PORT}"
) >"$TMP_ROOT/relay.log" 2>&1 &
PIDS+=("$!")

wait_for_http "relay" "http://127.0.0.1:${RELAY_PORT}/health"

(
  cd "$CORE_DIR"
  MOBILE_TUNNEL_DATABASE_URL="$DATABASE_URL" \
  MOBILE_TUNNEL_PUBLIC_BASE_URL="http://127.0.0.1:${ROUTER_PORT}" \
  MOBILE_TUNNEL_RELAY_REGION="us" \
  CTX_TUNNEL_MASTER_SECRET="$MASTER_SECRET" \
  node "$CORE_DIR/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$CORE_DIR" -- \
    cargo run -p ctx-tunnel-control-plane -- --listen "127.0.0.1:${CONTROL_PORT}"
) >"$TMP_ROOT/control-plane.log" 2>&1 &
PIDS+=("$!")

wait_for_http "control-plane" "http://127.0.0.1:${CONTROL_PORT}/health"

(
  cd "$CORE_DIR"
  MOBILE_TUNNEL_DATABASE_URL="$DATABASE_URL" \
  node "$CORE_DIR/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$CORE_DIR" -- \
    cargo run -p ctx-tunnel-router -- --listen "127.0.0.1:${ROUTER_PORT}"
) >"$TMP_ROOT/router.log" 2>&1 &
PIDS+=("$!")

wait_for_http "router" "http://127.0.0.1:${ROUTER_PORT}/health"

(
  cd "$CORE_DIR"
  CTX_TUNNEL_CONTROL_PLANE_URL="http://127.0.0.1:${CONTROL_PORT}" \
  CTX_MOBILE_TUNNEL_ALLOW_INSECURE_LOOPBACK=1 \
  CTX_MANAGED_TUNNEL_GRANT="$MANAGED_TUNNEL_GRANT" \
  node "$CORE_DIR/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "$CORE_DIR" -- \
    cargo run -p ctx-http --bin mobile_e2e
)
