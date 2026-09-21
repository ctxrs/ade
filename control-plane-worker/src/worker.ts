export type Env = {
  CONTROL_PLANE_ALLOWED_ORIGINS?: string;
  CONTROL_PLANE_DATABASE_URL?: string;
  ENVIRONMENT?: string;
};

type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };

const JSON_HEADERS = {
  "content-type": "application/json; charset=utf-8",
};

const FREE_ENTITLEMENTS = {
  plan_type: "free_local",
  subject_type: "install",
  account_id: null,
  org_id: null,
  active_org_id: null,
  membership_role: null,
  billing_subject: "install",
  features: {
    account_cloud_settings: "disabled",
    mobile_relay: "disabled",
    remote_mobile_access: "disabled",
    org_admin: "disabled",
    org_policy: "disabled",
    org_run_history: "disabled",
    org_audit: "disabled",
    llm_token_relay: "disabled",
  },
  expires_at: null,
  grace_expires_at: null,
};

const EMPTY_TEAM_STATE = {
  organizations: [],
  active_org_id: null,
  active_invites: [],
  feature_grants: [],
  active_admin_state: null,
};

const DEFAULT_ALLOWED_ORIGINS = new Set([
  "https://ctx.rs",
  "https://www.ctx.rs",
  "https://app.ctx.rs",
  "tauri://localhost",
  "http://tauri.localhost",
  "https://tauri.localhost",
  "http://localhost:1420",
  "http://localhost:3000",
  "http://localhost:5173",
]);

function allowedOrigins(env: Env): Set<string> {
  const configured = (env.CONTROL_PLANE_ALLOWED_ORIGINS ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
  return configured.length > 0 ? new Set(configured) : DEFAULT_ALLOWED_ORIGINS;
}

function corsHeaders(origin: string | null, env: Env): Record<string, string> {
  const headers = {
    "access-control-allow-headers": "authorization, content-type, x-csrf-token, x-ctx-active-org-id",
    "access-control-allow-methods": "GET, POST, OPTIONS",
  };
  if (!origin) return headers;
  if (!allowedOrigins(env).has(origin)) return headers;
  return {
    ...headers,
    "access-control-allow-origin": origin,
    "access-control-allow-credentials": "true",
  };
}

function jsonResponse(
  body: JsonValue,
  status: number,
  cors: Record<string, string>,
  headers: Record<string, string> = {},
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      ...JSON_HEADERS,
      ...cors,
      ...headers,
    },
  });
}

function featureUnavailable(cors: Record<string, string>, surface: string): Response {
  return jsonResponse({ error: "feature_unavailable", message: `${surface} is unavailable in this deployment.` }, 409, cors);
}

function methodNotAllowed(cors: Record<string, string>): Response {
  return jsonResponse({ error: "method_not_allowed" }, 405, cors, { allow: "GET, POST, OPTIONS" });
}

async function handleRequest(request: Request, env: Env): Promise<Response> {
  const origin = request.headers.get("origin");
  const cors = corsHeaders(origin, env);
  if (request.method === "OPTIONS") {
    if (origin && !cors["access-control-allow-origin"]) {
      return jsonResponse({ error: "origin_not_allowed" }, 403, cors);
    }
    return new Response("ok", { headers: cors });
  }

  const url = new URL(request.url);
  const path = url.pathname.replace(/\/+$/, "") || "/";

  if (path === "/v1/session" && request.method === "GET") {
    return jsonResponse({ user: null }, 200, cors);
  }
  if (path === "/v1/auth/logout" && request.method === "POST") {
    return jsonResponse({ ok: true }, 200, cors);
  }
  if ((path === "/v1/auth/start" || path === "/v1/auth/callback") && request.method === "POST") {
    return featureUnavailable(cors, "account authentication");
  }
  if (path === "/v1/account" && request.method === "GET") {
    return jsonResponse({ user: null, account: null }, 200, cors);
  }
  if (path === "/v1/entitlements" && request.method === "GET") {
    return jsonResponse(FREE_ENTITLEMENTS, 200, cors);
  }
  if (path === "/v1/team/state" && request.method === "GET") {
    return jsonResponse(EMPTY_TEAM_STATE, 200, cors);
  }
  if (path === "/v1/team/admin" && request.method === "POST") {
    return featureUnavailable(cors, "organization administration");
  }
  if (path === "/v1/mobile/tunnel-grant" && request.method === "POST") {
    return featureUnavailable(cors, "remote access grants");
  }
  if (path === "/v1/billing/checkout" && request.method === "POST") {
    return featureUnavailable(cors, "billing operation");
  }
  if (path === "/v1/billing/portal" && request.method === "POST") {
    return featureUnavailable(cors, "billing operation");
  }
  if (path === "/v1/billing/sync" && request.method === "POST") {
    return jsonResponse({ ok: true, synced: false }, 200, cors);
  }
  if ((path === "/v1/webhooks/stripe" || path === "/v1/webhooks/workos") && request.method === "POST") {
    return featureUnavailable(cors, "webhook intake");
  }

  if (["GET", "POST"].includes(request.method)) {
    return jsonResponse({ error: "not_found" }, 404, cors);
  }
  return methodNotAllowed(cors);
}

export function createControlPlaneWorker() {
  return {
    async fetch(request: Request, env: Env): Promise<Response> {
      try {
        return await handleRequest(request, env);
      } catch (error) {
        console.error("control_plane_unhandled_error", error);
        return jsonResponse({ error: "internal_error" }, 500, corsHeaders(request.headers.get("origin"), env));
      }
    },
  };
}

export default createControlPlaneWorker();
