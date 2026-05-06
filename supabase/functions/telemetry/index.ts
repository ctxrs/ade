import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { capturePostHogEvent } from "../_shared/posthog.ts";
import {
  buildTelemetryIngestPlan,
  TelemetryIngestError,
} from "../_shared/telemetry_ingest.ts";

const jsonResponse = (
  body: Record<string, unknown>,
  status: number,
  origin: string | null,
): Response =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });

const readRequiredEnv = (name: string): string | null => {
  const raw = Deno.env.get(name);
  if (!raw) return null;
  const trimmed = raw.trim();
  return trimmed.length > 0 ? trimmed : null;
};

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (req.method !== "POST") {
    return jsonResponse({ error: "method_not_allowed" }, 405, origin);
  }

  let payload: unknown;
  try {
    payload = await req.json();
  } catch {
    return jsonResponse({ error: "invalid_json" }, 400, origin);
  }

  const supabaseUrl = readRequiredEnv("SUPABASE_URL");
  const serviceRoleKey = readRequiredEnv("SUPABASE_SERVICE_ROLE_KEY");
  const idSalt = readRequiredEnv("INSTALL_ID_HASH_SALT");
  if (!supabaseUrl || !serviceRoleKey || !idSalt) {
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

  const client = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false },
  });

  const eventIds = ingestPlan.rows.map((row) => row.event_id);
  const { data: existingRows, error: existingError } = await client
    .from("telemetry_event")
    .select("event_id")
    .in("event_id", eventIds);
  if (existingError) {
    return jsonResponse(
      { error: "telemetry_idempotency_check_failed" },
      502,
      origin,
    );
  }

  const existingEventIds = new Set(
    (existingRows ?? [])
      .map((row) => typeof row.event_id === "string" ? row.event_id : null)
      .filter((eventId): eventId is string => eventId !== null),
  );
  const rowsToInsert = ingestPlan.rows.filter((row) =>
    !existingEventIds.has(row.event_id)
  );
  const capturesToMirror = ingestPlan.posthogCaptures.filter((_, index) => {
    const row = ingestPlan.rows[index];
    return row ? !existingEventIds.has(row.event_id) : false;
  });

  if (rowsToInsert.length > 0) {
    const { error } = await client.from("telemetry_event").insert(
      rowsToInsert,
    );
    if (error) {
      return jsonResponse({ error: "telemetry_insert_failed" }, 502, origin);
    }
  }

  const mirrorResults = await Promise.allSettled(
    capturesToMirror.map((capture) => capturePostHogEvent(capture)),
  );
  for (const result of mirrorResults) {
    if (result.status === "rejected") {
      console.error("telemetry_posthog_mirror_failed", result.reason);
    }
  }

  return new Response(null, {
    status: 204,
    headers: {
      ...corsHeaders(origin),
    },
  });
});
