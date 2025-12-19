import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "https://esm.sh/@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";
import { sha256Hex } from "../_shared/hash.ts";

type TelemetryEvent = {
  name: string;
  occurred_at: string;
  provider_id?: string | null;
  model_id?: string | null;
  env_target?: string | null;
  duration_ms?: number | null;
  status?: string | null;
  success?: boolean | null;
};

type TelemetryBatch = {
  install_id: string;
  app_version?: string | null;
  os?: string | null;
  arch?: string | null;
  events: TelemetryEvent[];
};

const MAX_EVENTS = 200;

function truncate(value: string | null | undefined, max: number): string | null {
  if (!value) return null;
  const s = String(value);
  if (s.length <= max) return s;
  return s.slice(0, max);
}

function parseTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const parsed = Date.parse(value);
  if (Number.isNaN(parsed)) return null;
  return new Date(parsed).toISOString();
}

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ error: "method_not_allowed" }), {
      status: 405,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  let payload: TelemetryBatch | null = null;
  try {
    payload = await req.json();
  } catch {
    return new Response(JSON.stringify({ error: "invalid_json" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  if (!payload?.install_id || !Array.isArray(payload.events)) {
    return new Response(JSON.stringify({ error: "bad_request" }), {
      status: 400,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const supabaseUrl = Deno.env.get("SUPABASE_URL") ?? "";
  const serviceRoleKey = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
  const idSalt = Deno.env.get("INSTALL_ID_HASH_SALT") ?? "local-dev";

  const installIdHash = await sha256Hex(`${idSalt}:${payload.install_id}`);
  const appVersion = truncate(payload.app_version ?? null, 64);
  const os = truncate(payload.os ?? null, 32);
  const arch = truncate(payload.arch ?? null, 32);

  const events = payload.events.slice(0, MAX_EVENTS).map((ev) => ({
    install_id_hash: installIdHash,
    occurred_at: parseTimestamp(ev.occurred_at) ?? new Date().toISOString(),
    event_name: truncate(ev.name, 64),
    app_version: appVersion,
    os,
    arch,
    provider_id: truncate(ev.provider_id ?? null, 64),
    model_id: truncate(ev.model_id ?? null, 128),
    env_target: truncate(ev.env_target ?? null, 32),
    duration_ms: typeof ev.duration_ms === "number" ? Math.max(0, Math.floor(ev.duration_ms)) : null,
    status: truncate(ev.status ?? null, 32),
    success: typeof ev.success === "boolean" ? ev.success : null,
  }));

  try {
    const client = createClient(supabaseUrl, serviceRoleKey, {
      auth: { persistSession: false },
    });
    await client.from("telemetry_event").insert(events);
  } catch {
    // ignore
  }

  return new Response(null, {
    status: 204,
    headers: {
      ...corsHeaders(origin),
    },
  });
});
