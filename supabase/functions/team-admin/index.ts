import { serve } from "https://deno.land/std@0.224.0/http/server.ts";
import {
  createClient,
  type SupabaseClient,
} from "npm:@supabase/supabase-js@2.49.1";
import {
  acceptOrganizationInvite,
  CommercialOrgError,
  createOrganizationForAccount,
  inviteOrganizationMember,
  listOrganizationsForAccount,
  resolveTeamCheckoutContext,
  SupabaseCommercialOrgStore,
  updateOrganizationMember,
} from "../_shared/commercial_orgs.ts";
import {
  CommercialResolverError,
  resolveAccessContext,
  serializeEntitlementsSnapshot,
} from "../_shared/commercial.ts";
import { corsHeaders } from "../_shared/cors.ts";

type TeamAdminRequest = {
  action?: string;
  name?: unknown;
  slug?: unknown;
  organization_id?: unknown;
  org_id?: unknown;
  invite_token?: unknown;
  token?: unknown;
  email?: unknown;
  role?: unknown;
  status?: unknown;
  membership_id?: unknown;
  seats?: unknown;
  policy?: unknown;
};

type TeamPolicyDraft = {
  providers: string;
  models: string;
  allowPersonalRoutes: boolean;
  sandboxProfile: "sandbox_required" | "sandbox_preferred";
  networkProfile: "default" | "restricted" | "offline";
  archiveVisibility:
    | "local_only"
    | "org_summary"
    | "org_transcript"
    | "org_evidence";
};

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

function requiredEnv(name: string): string {
  const value = Deno.env.get(name)?.trim() ?? "";
  if (!value) throw new Error(`Missing ${name}`);
  return value;
}

async function parseJson(req: Request): Promise<TeamAdminRequest> {
  const value = await req.json().catch(() => null);
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as TeamAdminRequest
    : {};
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
  if (req.method !== "GET" && req.method !== "POST") {
    return jsonResponse({ error: "method_not_allowed" }, 405, origin);
  }

  const token = bearerToken(req);
  if (!token) {
    return jsonResponse({ error: "unauthorized" }, 401, origin);
  }

  const supabase = createClient(
    requiredEnv("SUPABASE_URL"),
    requiredEnv("SUPABASE_SERVICE_ROLE_KEY"),
    { auth: { persistSession: false } },
  );
  const { data: userRes, error: userErr } = await supabase.auth.getUser(token);
  if (userErr || !userRes?.user?.id) {
    return jsonResponse({ error: "unauthorized" }, 401, origin);
  }

  const userId = userRes.user.id;
  const store = new SupabaseCommercialOrgStore(supabase);

  try {
    if (req.method === "GET") {
      const activeOrgId = activeOrgIdFromRequest(req);
      const payload = await listOrganizationsForAccount(store, userId);
      const activeOrgSummary = activeOrgId
        ? payload.organizations.find((candidate) =>
          candidate.organization.id === activeOrgId
        ) ?? null
        : payload.organizations[0] ?? null;
      const resolvedActiveOrgId = activeOrgSummary?.organization.id ?? null;
      const activeMembershipCanAdmin = activeOrgSummary &&
        activeOrgSummary.membership.status === "active" &&
        (activeOrgSummary.membership.role === "owner" ||
          activeOrgSummary.membership.role === "admin");
      const activeInvites = activeMembershipCanAdmin
        ? await store.listPendingInvitesByOrganization(
          activeOrgSummary.organization.id,
        )
        : [];
      const activeAdminState = activeMembershipCanAdmin
        ? await loadOrganizationAdminState(
          supabase,
          activeOrgSummary.organization.id,
        )
        : null;
      const accessContext = resolvedActiveOrgId
        ? await resolveAccessContext({
          supabase,
          authUserId: userId,
          activeOrgId: resolvedActiveOrgId,
          installId: installIdFromRequest(req),
        })
        : null;
      return jsonResponse(
        {
          ...payload,
          active_org_id: resolvedActiveOrgId,
          active_invites: activeInvites.map(serializeInvite),
          active_admin_state: serializeAdminState(activeAdminState),
          access_context: accessContext
            ? {
              ...accessContext,
              entitlements: serializeEntitlementsSnapshot(
                accessContext.entitlements,
              ),
            }
            : null,
        },
        200,
        origin,
      );
    }

    const payload = await parseJson(req);
    switch (payload.action) {
      case "create_organization":
        return jsonResponse(
          {
            organization: await createOrganizationForAccount(
              store,
              userId,
              { name: payload.name, slug: payload.slug },
            ),
          },
          200,
          origin,
        );
      case "invite_member":
        return jsonResponse(
          {
            invite: serializeInviteCreation(
              await inviteOrganizationMember(
                store,
                userId,
                {
                  organizationId: payload.organization_id ?? payload.org_id,
                  email: payload.email,
                  role: payload.role,
                },
              ),
            ),
          },
          200,
          origin,
        );
      case "accept_invite":
        return jsonResponse(
          serializeInviteAcceptance(
            await acceptOrganizationInvite(
              store,
              userId,
              { token: payload.token ?? payload.invite_token },
            ),
          ),
          200,
          origin,
        );
      case "update_member":
      case "update_member_role":
        return jsonResponse(
          {
            member: await updateOrganizationMember(
              store,
              userId,
              {
                organizationId: payload.organization_id ?? payload.org_id,
                membershipId: payload.membership_id,
                role: payload.role,
                status: payload.status,
              },
            ),
          },
          200,
          origin,
        );
      case "update_seats": {
        const seatCount = normalizeSeatCount(payload.seats);
        const context = await resolveTeamCheckoutContext(
          store,
          userId,
          {
            organizationId: payload.organization_id ?? payload.org_id,
            interval: "month",
            seatCount,
          },
        );
        await upsertOrganizationSeatTarget(
          supabase,
          context.organization.id,
          context.seatCount,
        );
        return jsonResponse(
          { ok: true, seat_target: context.seatCount },
          200,
          origin,
        );
      }
      case "update_policy": {
        const policy = normalizePolicyDraft(payload.policy);
        const context = await resolveTeamCheckoutContext(
          store,
          userId,
          {
            organizationId: payload.organization_id ?? payload.org_id,
            interval: "month",
          },
        );
        await upsertOrganizationPolicy(
          supabase,
          context.organization.id,
          policy,
        );
        return jsonResponse({ ok: true, policy }, 200, origin);
      }
      case "request_enterprise_setup": {
        const context = await resolveTeamCheckoutContext(
          store,
          userId,
          {
            organizationId: payload.organization_id ?? payload.org_id,
            interval: "month",
          },
        );
        const requestedAt = new Date().toISOString();
        await upsertEnterpriseSetupRequest(
          supabase,
          context.organization.id,
          context.account.id,
          requestedAt,
        );
        return jsonResponse(
          { ok: true, enterprise_setup_requested_at: requestedAt },
          200,
          origin,
        );
      }
      default:
        return jsonResponse({ error: "unknown_action" }, 400, origin);
    }
  } catch (error: unknown) {
    if (error instanceof CommercialOrgError) {
      return jsonResponse(
        { error: error.code, message: error.message },
        error.status,
        origin,
      );
    }
    if (error instanceof CommercialResolverError) {
      return jsonResponse(
        { error: error.code, message: error.message },
        error.status,
        origin,
      );
    }
    const message = error instanceof Error ? error.message : String(error);
    return jsonResponse({ error: "team_admin_failed", message }, 500, origin);
  }
});

async function loadOrganizationAdminState(
  supabase: SupabaseClient,
  organizationId: string,
): Promise<Record<string, unknown> | null> {
  const { data, error } = await supabase
    .from("organization_admin_state")
    .select(
      "organization_id, seat_target, policy_json, enterprise_setup_requested_at, enterprise_setup_requested_by_account_id, created_at, updated_at",
    )
    .eq("organization_id", organizationId)
    .maybeSingle();
  if (error) {
    throw new CommercialOrgError(
      "organization_admin_state_load_failed",
      500,
      error.message,
    );
  }
  return asRecord(data);
}

async function upsertOrganizationSeatTarget(
  supabase: SupabaseClient,
  organizationId: string,
  seatTarget: number,
): Promise<void> {
  const { error } = await supabase
    .from("organization_admin_state")
    .upsert(
      {
        organization_id: organizationId,
        seat_target: seatTarget,
      },
      { onConflict: "organization_id" },
    );
  if (error) {
    throw new CommercialOrgError(
      "organization_admin_state_update_failed",
      500,
      error.message,
    );
  }
}

async function upsertOrganizationPolicy(
  supabase: SupabaseClient,
  organizationId: string,
  policy: TeamPolicyDraft,
): Promise<void> {
  const { error } = await supabase
    .from("organization_admin_state")
    .upsert(
      {
        organization_id: organizationId,
        policy_json: policy,
      },
      { onConflict: "organization_id" },
    );
  if (error) {
    throw new CommercialOrgError(
      "organization_policy_update_failed",
      500,
      error.message,
    );
  }
}

async function upsertEnterpriseSetupRequest(
  supabase: SupabaseClient,
  organizationId: string,
  accountId: string,
  requestedAt: string,
): Promise<void> {
  const { error } = await supabase
    .from("organization_admin_state")
    .upsert(
      {
        organization_id: organizationId,
        enterprise_setup_requested_at: requestedAt,
        enterprise_setup_requested_by_account_id: accountId,
      },
      { onConflict: "organization_id" },
    );
  if (error) {
    throw new CommercialOrgError(
      "enterprise_setup_request_failed",
      500,
      error.message,
    );
  }
}

function serializeInviteCreation(
  result: Awaited<ReturnType<typeof inviteOrganizationMember>>,
): Record<string, unknown> {
  return {
    ...serializeInvite(result.invite),
    invite_token: result.inviteToken,
  };
}

function serializeInviteAcceptance(
  result: Awaited<ReturnType<typeof acceptOrganizationInvite>>,
): Record<string, unknown> {
  return {
    invite: serializeInvite(result.invite),
    membership: result.membership,
  };
}

function serializeInvite(invite: {
  id: string;
  organizationId: string;
  email: string;
  role: string;
  status: string;
  invitedByAccountId: string | null;
  acceptedByAccountId: string | null;
  membershipId: string | null;
  expiresAt: string;
  respondedAt: string | null;
  createdAt: string | null;
  updatedAt: string | null;
}): Record<string, unknown> {
  return {
    id: invite.id,
    organization_id: invite.organizationId,
    email: invite.email,
    role: invite.role,
    status: invite.status,
    invited_by_account_id: invite.invitedByAccountId,
    accepted_by_account_id: invite.acceptedByAccountId,
    membership_id: invite.membershipId,
    expires_at: invite.expiresAt,
    responded_at: invite.respondedAt,
    created_at: invite.createdAt,
    updated_at: invite.updatedAt,
  };
}

function serializeAdminState(
  state: Record<string, unknown> | null,
): Record<string, unknown> | null {
  if (!state) return null;
  return {
    organization_id: readNonEmptyString(state.organization_id),
    seat_target: readPositiveInteger(state.seat_target),
    policy_json: asRecord(state.policy_json) ?? {},
    enterprise_setup_requested_at: readStringOrNull(
      state.enterprise_setup_requested_at,
    ),
    enterprise_setup_requested_by_account_id: readStringOrNull(
      state.enterprise_setup_requested_by_account_id,
    ),
    created_at: readStringOrNull(state.created_at),
    updated_at: readStringOrNull(state.updated_at),
  };
}

function normalizeSeatCount(value: unknown): number {
  if (
    typeof value !== "number" || !Number.isInteger(value) || value < 1 ||
    value > 999
  ) {
    throw new CommercialOrgError(
      "invalid_seat_count",
      400,
      "Seat count must be an integer between 1 and 999.",
    );
  }
  return value;
}

function normalizePolicyDraft(value: unknown): TeamPolicyDraft {
  const record = asRecord(value);
  if (!record) {
    throw new CommercialOrgError(
      "invalid_policy",
      400,
      "Policy payload must be an object.",
    );
  }
  const allowPersonalRoutes = record.allowPersonalRoutes;
  if (typeof allowPersonalRoutes !== "boolean") {
    throw new CommercialOrgError(
      "invalid_policy",
      400,
      "Policy allowPersonalRoutes must be a boolean.",
    );
  }
  return {
    providers: normalizePolicyText(record.providers, "providers", 500),
    models: normalizePolicyText(record.models, "models", 1000),
    allowPersonalRoutes,
    sandboxProfile: normalizeEnum(
      record.sandboxProfile,
      "sandboxProfile",
      ["sandbox_required", "sandbox_preferred"],
    ),
    networkProfile: normalizeEnum(
      record.networkProfile,
      "networkProfile",
      ["default", "restricted", "offline"],
    ),
    archiveVisibility: normalizeEnum(
      record.archiveVisibility,
      "archiveVisibility",
      ["local_only", "org_summary", "org_transcript", "org_evidence"],
    ),
  };
}

function normalizePolicyText(
  value: unknown,
  fieldName: string,
  maxLength: number,
): string {
  if (typeof value !== "string") {
    throw new CommercialOrgError(
      "invalid_policy",
      400,
      `Policy ${fieldName} must be a string.`,
    );
  }
  const trimmed = value.trim();
  if (trimmed.length > maxLength) {
    throw new CommercialOrgError(
      "invalid_policy",
      400,
      `Policy ${fieldName} is too long.`,
    );
  }
  return trimmed;
}

function normalizeEnum<const T extends string>(
  value: unknown,
  fieldName: string,
  allowed: readonly T[],
): T {
  if (typeof value === "string" && allowed.includes(value as T)) {
    return value as T;
  }
  throw new CommercialOrgError(
    "invalid_policy",
    400,
    `Policy ${fieldName} is invalid.`,
  );
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function readNonEmptyString(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function readStringOrNull(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function readPositiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 1
    ? value
    : null;
}
