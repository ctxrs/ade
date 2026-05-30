import { createNeonTelemetryDatabase, type TelemetryDatabaseFactory } from "./database";
import { capturePostHogEvent, type PostHogEnv } from "./posthog";
import {
  buildTelemetryIngestPlan,
  selectPostHogCapturesForInsertedRows,
  TelemetryIngestError,
  type TelemetryPostHogCapture,
} from "./telemetry-ingest";

export type Env = PostHogEnv & {
  TELEMETRY_DATABASE_URL?: string;
  INSTALL_ID_HASH_SALT?: string;
};

type WorkerDeps = {
  createDatabaseClient: TelemetryDatabaseFactory;
  capturePostHogEvent: (
    input: TelemetryPostHogCapture,
    env: PostHogEnv,
  ) => Promise<void>;
};

const JSON_CONTENT_TYPE = "application/json; charset=utf-8";
const TELEMETRY_PATH = "/functions/v1/telemetry";

const defaultDeps: WorkerDeps = {
  createDatabaseClient: createNeonTelemetryDatabase,
  capturePostHogEvent,
};

export function createTelemetryWorker(deps: Partial<WorkerDeps> = {}) {
  const resolvedDeps: WorkerDeps = { ...defaultDeps, ...deps };
  return {
    async fetch(request: Request, env: Env): Promise<Response> {
      try {
        return await handleFetch(request, env, resolvedDeps);
      } catch (error) {
        console.error("telemetry_worker_unhandled_error", error);
        return jsonResponse({ error: "internal_error" }, 500, request.headers.get("origin"));
      }
    },
  };
}

export default createTelemetryWorker();

async function handleFetch(
  request: Request,
  env: Env,
  deps: WorkerDeps,
): Promise<Response> {
  const origin = request.headers.get("origin");
  const url = new URL(request.url);
  if (url.pathname !== TELEMETRY_PATH) {
    return jsonResponse({ error: "not_found" }, 404, origin);
  }
  if (request.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (request.method !== "POST") {
    return jsonResponse({ error: "method_not_allowed" }, 405, origin, {
      allow: "POST, OPTIONS",
    });
  }

  let payload: unknown;
  try {
    payload = await request.json();
  } catch {
    return jsonResponse({ error: "invalid_json" }, 400, origin);
  }

  const databaseUrl = readRequiredEnv(env, "TELEMETRY_DATABASE_URL");
  const idSalt = readRequiredEnv(env, "INSTALL_ID_HASH_SALT");
  if (!databaseUrl || !idSalt) {
    return jsonResponse({ error: "telemetry_env_not_configured" }, 500, origin);
  }

  let ingestPlan;
  try {
    ingestPlan = await buildTelemetryIngestPlan(payload, { idSalt });
  } catch (err) {
    if (err instanceof TelemetryIngestError) {
      return jsonResponse({ error: err.code }, err.status, origin);
    }
    return jsonResponse(
      { error: "telemetry_normalization_failed" },
      500,
      origin,
    );
  }

  let insertedEventIds: Set<string>;
  try {
    const database = await deps.createDatabaseClient(databaseUrl);
    insertedEventIds = await database.insertTelemetryRows(ingestPlan.rows);
  } catch (error) {
    console.error("telemetry_neon_insert_failed", error);
    return jsonResponse({ error: "telemetry_insert_failed" }, 502, origin);
  }

  const insertedRows = ingestPlan.rows.filter((row) =>
    insertedEventIds.has(row.event_id)
  );
  const capturesToMirror = selectPostHogCapturesForInsertedRows(
    ingestPlan.posthogCaptures,
    insertedRows,
  );
  const mirrorResults = await Promise.allSettled(
    capturesToMirror.map((capture) => deps.capturePostHogEvent(capture, env)),
  );
  for (const result of mirrorResults) {
    if (result.status === "rejected") {
      console.error("telemetry_posthog_mirror_failed", result.reason);
    }
  }

  return new Response(null, {
    status: 204,
    headers: corsHeaders(origin),
  });
}

function corsHeaders(origin: string | null): Record<string, string> {
  return {
    "access-control-allow-origin": origin ?? "*",
    "access-control-allow-headers": "authorization, content-type, x-client-info, apikey",
    "access-control-allow-methods": "POST, OPTIONS",
  };
}

function jsonResponse(
  body: Record<string, unknown>,
  status: number,
  origin: string | null,
  extraHeaders?: Record<string, string>,
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "content-type": JSON_CONTENT_TYPE,
      ...corsHeaders(origin),
      ...extraHeaders,
    },
  });
}

function readRequiredEnv(env: Env, name: keyof Env): string | null {
  const raw = env[name];
  if (!raw) return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
}
