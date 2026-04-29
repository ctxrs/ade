import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "npm:@supabase/supabase-js@2.49.1";
import {
  CommercialResolverError,
  freeLocalSnapshot,
  resolveAccessContext,
  serializeEntitlementsSnapshot,
} from "../_shared/commercial.ts";
import { corsHeaders } from "../_shared/cors.ts";

function jsonResponse(
  body: unknown,
  status: number,
  origin: string | null,
): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
}

function bearerToken(req: Request): string {
  const authHeader = req.headers.get("authorization") ?? "";
  return authHeader.toLowerCase().startsWith("bearer ")
    ? authHeader.slice(7).trim()
    : "";
}

function activeOrgIdFromRequest(req: Request): string | null {
  const url = new URL(req.url);
  return (
    req.headers.get("x-ctx-active-org-id") ??
      url.searchParams.get("active_org_id") ??
      url.searchParams.get("org_id")
  );
}

function installIdFromRequest(req: Request): string | null {
  return req.headers.get("x-ctx-install-id");
}

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (req.method !== "GET") {
    return jsonResponse({ error: "method_not_allowed" }, 405, origin);
  }

  const supabaseUrl = Deno.env.get("SUPABASE_URL") ?? "";
  const serviceRoleKey = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
  if (!supabaseUrl || !serviceRoleKey) {
    return jsonResponse({ error: "server_misconfigured" }, 500, origin);
  }

  const client = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false },
  });

  const token = bearerToken(req);
  if (!token) {
    return jsonResponse(
      serializeEntitlementsSnapshot(freeLocalSnapshot()),
      200,
      origin,
    );
  }

  const { data: userRes, error: userErr } = await client.auth.getUser(token);
  if (userErr || !userRes?.user?.id) {
    return jsonResponse({ error: "unauthorized" }, 401, origin);
  }

  try {
    const context = await resolveAccessContext({
      supabase: client,
      authUserId: userRes.user.id,
      activeOrgId: activeOrgIdFromRequest(req),
      installId: installIdFromRequest(req),
    });
    return jsonResponse(
      serializeEntitlementsSnapshot(context.entitlements),
      200,
      origin,
    );
  } catch (error: unknown) {
    if (error instanceof CommercialResolverError) {
      return jsonResponse(
        { error: error.code, message: error.message },
        error.status,
        origin,
      );
    }
    const message = error instanceof Error ? error.message : String(error);
    return jsonResponse(
      { error: "commercial_context_failed", message },
      500,
      origin,
    );
  }
});
