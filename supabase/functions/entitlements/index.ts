import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import { createClient } from "https://esm.sh/@supabase/supabase-js@2.49.1";
import { corsHeaders } from "../_shared/cors.ts";

type EntitlementsSnapshot = {
  plan_type: "free_local" | "pro" | "team" | "enterprise";
  features: Record<string, "enabled" | "disabled">;
  expires_at?: string | null;
  grace_expires_at?: string | null;
};

function emptySnapshot(): EntitlementsSnapshot {
  return {
    plan_type: "free_local",
    features: {
      remote_mobile_access: "disabled",
      push_notifications: "disabled",
    },
    expires_at: null,
    grace_expires_at: null,
  };
}

function planHasManagedFeatures(planType: string): boolean {
  return planType === "pro" || planType === "team" || planType === "enterprise";
}

serve(async (req) => {
  const origin = req.headers.get("origin");
  if (req.method === "OPTIONS") {
    return new Response("ok", { headers: corsHeaders(origin) });
  }
  if (req.method !== "GET") {
    return new Response(JSON.stringify({ error: "method_not_allowed" }), {
      status: 405,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const supabaseUrl = Deno.env.get("SUPABASE_URL") ?? "";
  const serviceRoleKey = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "";
  if (!supabaseUrl || !serviceRoleKey) {
    return new Response(JSON.stringify({ error: "server_misconfigured" }), {
      status: 500,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const client = createClient(supabaseUrl, serviceRoleKey, {
    auth: { persistSession: false },
  });

  const authHeader = req.headers.get("authorization") ?? "";
  const token = authHeader.toLowerCase().startsWith("bearer ") ? authHeader.slice(7).trim() : "";
  if (!token) {
    return new Response(JSON.stringify(emptySnapshot()), {
      status: 200,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const { data: userRes, error: userErr } = await client.auth.getUser(token);
  if (userErr || !userRes?.user?.id) {
    return new Response(JSON.stringify(emptySnapshot()), {
      status: 200,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const userId = userRes.user.id;
  const { data, error } = await client
    .from("billing_subscription")
    .select("plan_type,status,current_period_end")
    .eq("user_id", userId)
    .maybeSingle();

  if (error || !data) {
    return new Response(JSON.stringify(emptySnapshot()), {
      status: 200,
      headers: { "content-type": "application/json", ...corsHeaders(origin) },
    });
  }

  const planType = String(data.plan_type ?? "free_local") as EntitlementsSnapshot["plan_type"];
  const managed = planHasManagedFeatures(planType) && (data.status === "active" || data.status === "trialing");
  const expiresAt = data.current_period_end ? new Date(String(data.current_period_end)).toISOString() : null;
  const graceExpiresAt = expiresAt ? new Date(Date.parse(expiresAt) + 7 * 24 * 60 * 60 * 1000).toISOString() : null;

  const snapshot: EntitlementsSnapshot = {
    plan_type: planType,
    features: {
      remote_mobile_access: managed ? "enabled" : "disabled",
      push_notifications: managed ? "enabled" : "disabled",
    },
    expires_at: expiresAt,
    grace_expires_at: graceExpiresAt,
  };

  return new Response(JSON.stringify(snapshot), {
    status: 200,
    headers: { "content-type": "application/json", ...corsHeaders(origin) },
  });
});

