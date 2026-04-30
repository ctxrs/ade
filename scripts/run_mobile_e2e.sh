#!/usr/bin/env bash
set -euo pipefail

if ! command -v supabase >/dev/null 2>&1; then
  echo "error: supabase CLI not found" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq not found" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl not found" >&2
  exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUPABASE_DIR="$ROOT_DIR/supabase"
CORE_DIR="$ROOT_DIR/core"

CLI_TOKEN="${CTX_SUPABASE_ACCESS_TOKEN:-}"
if [ -z "$CLI_TOKEN" ]; then
  echo "error: CTX_SUPABASE_ACCESS_TOKEN is required for supabase CLI auth" >&2
  exit 1
fi

supabase login --token "$CLI_TOKEN" --workdir "$SUPABASE_DIR" >/dev/null

PROJECT_REF="${SUPABASE_PROJECT_REF:-}"
if [ -z "$PROJECT_REF" ]; then
  PROJECT_REF="$(
    supabase projects list --output json --workdir "$SUPABASE_DIR" \
      | jq -r '.[0].ref'
  )"
fi
if [ -z "$PROJECT_REF" ] || [ "$PROJECT_REF" = "null" ]; then
  echo "error: failed to resolve SUPABASE_PROJECT_REF" >&2
  exit 1
fi

KEYS_JSON="$(
  supabase projects api-keys --project-ref "$PROJECT_REF" --output json --workdir "$SUPABASE_DIR"
)"
ANON_KEY="$(echo "$KEYS_JSON" | jq -r '.[] | select(.id=="anon") | .api_key')"
SERVICE_ROLE_KEY="$(echo "$KEYS_JSON" | jq -r '.[] | select(.id=="service_role") | .api_key')"
if [ -z "$ANON_KEY" ] || [ "$ANON_KEY" = "null" ]; then
  echo "error: anon key not found for project $PROJECT_REF" >&2
  exit 1
fi
if [ -z "$SERVICE_ROLE_KEY" ] || [ "$SERVICE_ROLE_KEY" = "null" ]; then
  echo "error: service_role key not found for project $PROJECT_REF" >&2
  exit 1
fi

SUPABASE_URL="https://${PROJECT_REF}.supabase.co"
EMAIL="mobile-e2e-$(date +%s)-${RANDOM}@example.com"
PASSWORD="$(openssl rand -hex 12)"

CREATE_USER_PAYLOAD="$(jq -nc --arg email "$EMAIL" --arg password "$PASSWORD" '{
  email: $email,
  password: $password,
  email_confirm: true
}')"
CREATE_USER_RESP="$(
  curl -sS -X POST "$SUPABASE_URL/auth/v1/admin/users" \
    -H "apikey: $SERVICE_ROLE_KEY" \
    -H "Authorization: Bearer $SERVICE_ROLE_KEY" \
    -H "Content-Type: application/json" \
    -d "$CREATE_USER_PAYLOAD"
)"
USER_ID="$(echo "$CREATE_USER_RESP" | jq -r '.id // .user.id')"
if [ -z "$USER_ID" ] || [ "$USER_ID" = "null" ]; then
  echo "error: failed to create user for mobile e2e" >&2
  exit 1
fi

PERIOD_END="$(
  python3 - <<'PY'
from datetime import datetime, timedelta, timezone

print((datetime.now(timezone.utc) + timedelta(days=30)).strftime("%Y-%m-%dT%H:%M:%SZ"))
PY
)"
SUBSCRIPTION_PAYLOAD="$(jq -nc --arg plan "pro" --arg status "active" --arg end "$PERIOD_END" '{
  plan_type: $plan,
  status: $status,
  current_period_end: $end
}')"
ACCOUNT_RESP="$(
  curl -sS "$SUPABASE_URL/rest/v1/external_identity_link?provider=eq.supabase_auth&external_subject=eq.$USER_ID&select=account_id" \
    -H "apikey: $SERVICE_ROLE_KEY" \
    -H "Authorization: Bearer $SERVICE_ROLE_KEY"
)"
ACCOUNT_ID="$(echo "$ACCOUNT_RESP" | jq -r '.[0].account_id // empty')"
if [ -z "$ACCOUNT_ID" ]; then
  echo "error: failed to resolve ctx account for mobile e2e user" >&2
  exit 1
fi

BILLING_SUBJECT_RESP="$(
  curl -sS "$SUPABASE_URL/rest/v1/billing_subject?subject_type=eq.account&account_id=eq.$ACCOUNT_ID&select=id" \
    -H "apikey: $SERVICE_ROLE_KEY" \
    -H "Authorization: Bearer $SERVICE_ROLE_KEY"
)"
BILLING_SUBJECT_ID="$(echo "$BILLING_SUBJECT_RESP" | jq -r '.[0].id // empty')"
if [ -z "$BILLING_SUBJECT_ID" ]; then
  echo "error: failed to resolve billing subject for mobile e2e user" >&2
  exit 1
fi

COMMERCE_SUBSCRIPTION_PAYLOAD="$(echo "$SUBSCRIPTION_PAYLOAD" | jq --arg billing_subject_id "$BILLING_SUBJECT_ID" '. + {
  billing_subject_id: $billing_subject_id,
  provider: "mobile_e2e"
}')"
curl -sS -X POST "$SUPABASE_URL/rest/v1/commerce_subscription?on_conflict=billing_subject_id,provider" \
  -H "apikey: $SERVICE_ROLE_KEY" \
  -H "Authorization: Bearer $SERVICE_ROLE_KEY" \
  -H "Content-Type: application/json" \
  -H "Prefer: resolution=merge-duplicates,return=minimal" \
  -d "$COMMERCE_SUBSCRIPTION_PAYLOAD" >/dev/null

TOKEN_PAYLOAD="$(jq -nc --arg email "$EMAIL" --arg password "$PASSWORD" '{
  email: $email,
  password: $password
}')"
TOKEN_RESP="$(
  curl -sS -X POST "$SUPABASE_URL/auth/v1/token?grant_type=password" \
    -H "apikey: $ANON_KEY" \
    -H "Authorization: Bearer $ANON_KEY" \
    -H "Content-Type: application/json" \
    -d "$TOKEN_PAYLOAD"
)"
USER_TOKEN="$(echo "$TOKEN_RESP" | jq -r '.access_token')"
if [ -z "$USER_TOKEN" ] || [ "$USER_TOKEN" = "null" ]; then
  echo "error: failed to mint user session token" >&2
  exit 1
fi

CONTROL_PLANE_URL="${CTX_TUNNEL_CONTROL_PLANE_URL:-https://tunnel.ctx.rs}"

echo "Running mobile e2e..."
CTX_TUNNEL_CONTROL_PLANE_URL="$CONTROL_PLANE_URL" \
CTX_SUPABASE_ACCESS_TOKEN="$USER_TOKEN" \
  node "$CORE_DIR/scripts/run_with_ctx_cache_env.cjs" \
    --mode workspace \
    --cwd "$CORE_DIR" \
    -- \
    cargo run -p ctx-http --bin mobile_e2e
