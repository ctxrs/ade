import { useState } from "react";
import type { User } from "@supabase/supabase-js";
import { TextInput } from "../../../components/ui/text-input";
import {
  OrganizationSwitcher,
  PolicySelect,
  RoleSelect,
  StatusPill,
  TEAM_ENTERPRISE_CONTROL_ROW_STYLE,
} from "./TeamEnterpriseSection.controls";
import type { EntitlementsSnapshot, MembershipRole } from "../teamEnterpriseSettingsApi";
import { Card, Row, Toggle } from "../SettingsPage.components";
import type { PlanType } from "../entitlementAnalytics";
import type {
  TeamEnterpriseCloudState,
  TeamEnterprisePolicyDraft,
} from "../teamEnterpriseSettingsApi";
import {
  activeOrgDescription,
  featureStatus,
  formatDateLabel,
  formatPlanLabel,
  formatRoleLabel,
  formatScopeLabel,
  roleCanAdmin,
} from "./TeamEnterpriseSection.helpers";

export function TeamEnterpriseSection({
  supabaseConfigured,
  billingUser,
  entitlementsBusy,
  plan,
  entitlements,
  cloudState,
  cloudBusy,
  cloudError,
  actionBusy,
  actionError,
  actionNotice,
  orgName,
  onOrgNameChange,
  inviteEmail,
  onInviteEmailChange,
  inviteRole,
  onInviteRoleChange,
  seatTarget,
  onSeatTargetChange,
  policyDraft,
  onPolicyDraftChange,
  onRefresh,
  onSelectOrg,
  onCreateOrg,
  onInviteMember,
  onAcceptInvite,
  onUpdateSeats,
  onSavePolicy,
  onStartTeamCheckout,
  onRequestEnterpriseSetup,
}: {
  supabaseConfigured: boolean;
  billingUser: User | null;
  entitlementsBusy: boolean;
  plan: PlanType;
  entitlements: EntitlementsSnapshot | null;
  cloudState: TeamEnterpriseCloudState;
  cloudBusy: boolean;
  cloudError: string | null;
  actionBusy: boolean;
  actionError: string | null;
  actionNotice: string | null;
  orgName: string;
  onOrgNameChange: (value: string) => void;
  inviteEmail: string;
  onInviteEmailChange: (value: string) => void;
  inviteRole: MembershipRole;
  onInviteRoleChange: (value: MembershipRole) => void;
  seatTarget: string;
  onSeatTargetChange: (value: string) => void;
  policyDraft: TeamEnterprisePolicyDraft;
  onPolicyDraftChange: (value: TeamEnterprisePolicyDraft) => void;
  onRefresh: () => void | Promise<void>;
  onSelectOrg: (orgId: string) => void | Promise<void>;
  onCreateOrg: () => void | Promise<void>;
  onInviteMember: () => void | Promise<void>;
  onAcceptInvite: (inviteToken: string) => void | Promise<void>;
  onUpdateSeats: () => void | Promise<void>;
  onSavePolicy: () => void | Promise<void>;
  onStartTeamCheckout: (interval: "month" | "year") => void | Promise<void>;
  onRequestEnterpriseSetup: () => void | Promise<void>;
}) {
  const [inviteToken, setInviteToken] = useState("");
  const currentPlanLabel = entitlementsBusy && !entitlements ? "Loading" : formatPlanLabel(plan);
  const billingScope = entitlements?.billing_subject ?? entitlements?.subject_type ?? null;
  const accountCloudSettings = featureStatus(entitlements?.features?.account_cloud_settings);
  const orgAdminFeature = featureStatus(entitlements?.features?.org_admin);
  const orgPolicyFeature = featureStatus(entitlements?.features?.org_policy);
  const orgRunHistory = featureStatus(entitlements?.features?.org_run_history);
  const enterpriseAudit = featureStatus(entitlements?.features?.org_audit);
  const mobileRelay = featureStatus(entitlements?.features?.mobile_relay);
  const llmTokenRelay = featureStatus(entitlements?.features?.llm_token_relay);
  const activeRole = cloudState.activeOrg?.role ?? entitlements?.membership_role ?? null;
  const canAdmin = Boolean(supabaseConfigured && billingUser && roleCanAdmin(activeRole));
  const canUseOrgActions = canAdmin && !actionBusy && !cloudBusy;
  const canAcceptInvite = Boolean(supabaseConfigured && billingUser && !actionBusy && inviteToken.trim());
  const canRefresh = Boolean(supabaseConfigured && billingUser && !cloudBusy);
  const canCheckout = Boolean(canUseOrgActions && cloudState.activeOrgId && cloudState.billingSubjectId);
  const pendingInvites = cloudState.invites.filter((invite) => invite.status === "pending");
  const activeSubscription = cloudState.subscriptions[0] ?? null;
  const enterprisePlanStatus =
    plan === "enterprise" ? { label: "Entitled", tone: "ok" as const } : { label: "Not enabled", tone: "warn" as const };

  const updatePolicy = (patch: Partial<TeamEnterprisePolicyDraft>) => {
    onPolicyDraftChange({ ...policyDraft, ...patch });
  };

  return (
    <>
      {!supabaseConfigured ? (
        <div className="settings-banner settings-banner-error">
          Billing/auth is not configured in this web build. Account, organization, and enterprise controls are unavailable.
        </div>
      ) : null}
      {cloudBusy ? <div className="settings-banner">Loading organization state…</div> : null}
      {cloudError ? <div className="settings-banner settings-banner-error">{cloudError}</div> : null}
      {actionError ? <div className="settings-banner settings-banner-error">{actionError}</div> : null}
      {actionNotice ? <div className="settings-banner">{actionNotice}</div> : null}

      <Card title="Account & Plans">
        <Row
          title="Current plan"
          description={entitlementsBusy ? "Refreshing the latest entitlements snapshot." : "Server-derived plan state for this install or signed-in account."}
          control={<StatusPill label={currentPlanLabel} tone={plan === "free_local" ? "neutral" : "ok"} />}
        />
        <Row
          title="Signed in account"
          description={billingUser?.email ?? "No signed-in account in this browser session."}
          control={<StatusPill label={billingUser ? "Connected" : "Signed out"} tone={billingUser ? "ok" : "warn"} />}
        />
        <Row
          title="Billing scope"
          description={billingScope ? `Entitlements are scoped to ${formatScopeLabel(billingScope).toLowerCase()}.` : "Billing subject scope is not available yet."}
          control={<StatusPill label={formatScopeLabel(billingScope)} tone={billingScope ? "neutral" : "warn"} />}
        />
        <Row
          title="Account cloud settings"
          description="Personal cloud/account settings are gated by the entitlements snapshot."
          control={<StatusPill label={accountCloudSettings.label} tone={accountCloudSettings.tone} />}
        />
        <Row
          title="Team checkout"
          description={cloudState.billingSubjectId ? "Checkout is bound to the active organization billing subject." : "An organization billing subject is required before checkout."}
          control={
            <div style={TEAM_ENTERPRISE_CONTROL_ROW_STYLE}>
              <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onStartTeamCheckout("month")} disabled={!canCheckout}>
                Monthly
              </button>
              <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onStartTeamCheckout("year")} disabled={!canCheckout}>
                Yearly
              </button>
            </div>
          }
        />
      </Card>

      <Card title="Organization">
        <Row
          title="Active organization"
          description={activeOrgDescription(cloudState)}
          control={<OrganizationSwitcher cloudState={cloudState} disabled={cloudBusy || actionBusy} onSelectOrg={onSelectOrg} />}
        />
        <Row
          title="Create organization"
          description="Creates a self-serve Team organization when the admin API is deployed."
          control={
            <div style={TEAM_ENTERPRISE_CONTROL_ROW_STYLE}>
              <TextInput
                className="settings-control"
                aria-label="Organization name"
                placeholder="Team name"
                value={orgName}
                onChange={(event) => onOrgNameChange(event.target.value)}
                disabled={!supabaseConfigured || !billingUser || actionBusy}
              />
              <button type="button" className="settings-btn" onClick={() => void onCreateOrg()} disabled={!supabaseConfigured || !billingUser || actionBusy}>
                Create
              </button>
            </div>
          }
        />
        <Row
          title="Refresh"
          description="Reloads organizations, billing subject state, invites, subscription state, and entitlements."
          control={
            <button type="button" className="settings-btn settings-btn-secondary" onClick={() => void onRefresh()} disabled={!canRefresh}>
              Refresh
            </button>
          }
        />
        <Row
          title="Admin controls"
          description={canAdmin ? "The active membership can manage this organization." : "Owner or admin role is required for organization changes."}
          control={<StatusPill label={orgAdminFeature.label} tone={orgAdminFeature.tone} />}
        />
      </Card>

      <Card title="Members & Seats">
        <Row
          title="Current role"
          description={cloudState.activeOrg?.name ?? "No active organization selected."}
          control={<StatusPill label={formatRoleLabel(activeRole)} tone={activeRole ? "neutral" : "warn"} />}
        />
        <Row
          title="Invite member"
          description="Invite delivery and acceptance are owned by the Team admin API."
          control={
            <div style={TEAM_ENTERPRISE_CONTROL_ROW_STYLE}>
              <TextInput
                className="settings-control"
                aria-label="Invite email"
                placeholder="teammate@company.com"
                value={inviteEmail}
                onChange={(event) => onInviteEmailChange(event.target.value)}
                disabled={!canUseOrgActions}
              />
              <RoleSelect value={inviteRole} disabled={!canUseOrgActions} onChange={onInviteRoleChange} />
              <button type="button" className="settings-btn" onClick={() => void onInviteMember()} disabled={!canUseOrgActions}>
                Invite
              </button>
            </div>
          }
        />
        <Row
          title="Pending invites"
          description={pendingInvites.length > 0 ? pendingInvites.map((invite) => `${invite.email} (${formatRoleLabel(invite.role)})`).join(", ") : "No pending invites are visible for this organization."}
          control={<StatusPill label={String(pendingInvites.length)} tone={pendingInvites.length > 0 ? "warn" : "neutral"} />}
        />
        <Row
          title="Accept invite"
          description={cloudState.invites.length > 0 ? "Paste the invite token from the invitation email. Visible pending rows are shown above for admins." : "Paste the invite token from the invitation email."}
          control={
            <div style={TEAM_ENTERPRISE_CONTROL_ROW_STYLE}>
              <TextInput
                className="settings-control"
                aria-label="Invite token"
                placeholder="Invite token"
                value={inviteToken}
                onChange={(event) => setInviteToken(event.target.value)}
                disabled={!supabaseConfigured || !billingUser || actionBusy}
              />
              <button
                type="button"
                className="settings-btn settings-btn-secondary"
                onClick={() => void onAcceptInvite(inviteToken.trim())}
                disabled={!canAcceptInvite}
              >
                Accept
              </button>
            </div>
          }
        />
        <Row
          title="Seats"
          description={activeSubscription ? `${formatPlanLabel(activeSubscription.planType)} subscription is ${activeSubscription.status}. Period ends ${formatDateLabel(activeSubscription.currentPeriodEnd)}.` : "No organization subscription row is visible yet."}
          control={
            <div style={TEAM_ENTERPRISE_CONTROL_ROW_STYLE}>
              <TextInput
                className="settings-control"
                aria-label="Seat count"
                inputMode="numeric"
                placeholder="Seats"
                value={seatTarget}
                onChange={(event) => onSeatTargetChange(event.target.value)}
                disabled={!canUseOrgActions}
              />
              <button type="button" className="settings-btn" onClick={() => void onUpdateSeats()} disabled={!canUseOrgActions}>
                Update
              </button>
            </div>
          }
        />
        <Row
          title="Member directory"
          description={cloudState.memberDirectoryAvailable ? "Member directory is available." : "Member directory API is not available in this build; only current role and invite state are shown."}
          control={<StatusPill label={cloudState.memberDirectoryAvailable ? "Available" : "Unavailable"} tone={cloudState.memberDirectoryAvailable ? "ok" : "warn"} />}
        />
      </Card>

      <Card title="Policy">
        <Row
          title="Policy entitlement"
          description="Org policy save requests are sent to the Team admin API."
          control={<StatusPill label={orgPolicyFeature.label} tone={orgPolicyFeature.tone} />}
        />
        <Row
          title="Provider allowlist"
          description="Comma-separated provider ids for org-managed runs."
          control={
            <TextInput
              className="settings-control settings-control-wide"
              aria-label="Provider allowlist"
              value={policyDraft.providers}
              onChange={(event) => updatePolicy({ providers: event.target.value })}
              disabled={!canUseOrgActions}
            />
          }
        />
        <Row
          title="Model allowlist"
          description="Optional comma-separated full model ids; blank means provider defaults."
          control={
            <TextInput
              className="settings-control settings-control-wide"
              aria-label="Model allowlist"
              value={policyDraft.models}
              onChange={(event) => updatePolicy({ models: event.target.value })}
              disabled={!canUseOrgActions}
            />
          }
        />
        <Row
          title="Personal account routes"
          description="Allows personal OAuth or API-key routes only when org policy permits it."
          control={
            <Toggle
              checked={policyDraft.allowPersonalRoutes}
              disabled={!canUseOrgActions}
              ariaLabel="Allow personal account routes"
              onChange={(next) => updatePolicy({ allowPersonalRoutes: next })}
            />
          }
        />
        <Row
          title="Sandbox profile"
          description="Execution requirement for org-managed runs."
          control={
            <PolicySelect
              ariaLabel="Sandbox profile"
              value={policyDraft.sandboxProfile}
              disabled={!canUseOrgActions}
              onChange={(sandboxProfile) => updatePolicy({ sandboxProfile })}
              options={[
                { value: "sandbox_required", label: "Sandbox required" },
                { value: "sandbox_preferred", label: "Sandbox preferred" },
              ]}
            />
          }
        />
        <Row
          title="Network profile"
          description="Outbound network posture for org-managed runs."
          control={
            <PolicySelect
              ariaLabel="Network profile"
              value={policyDraft.networkProfile}
              disabled={!canUseOrgActions}
              onChange={(networkProfile) => updatePolicy({ networkProfile })}
              options={[
                { value: "default", label: "Default" },
                { value: "restricted", label: "Restricted" },
                { value: "offline", label: "Offline" },
              ]}
            />
          }
        />
        <Row
          title="Archive visibility"
          description="Default visibility preset for organization run history."
          control={
            <PolicySelect
              ariaLabel="Archive visibility"
              value={policyDraft.archiveVisibility}
              disabled={!canUseOrgActions}
              onChange={(archiveVisibility) => updatePolicy({ archiveVisibility })}
              options={[
                { value: "local_only", label: "Local only" },
                { value: "org_summary", label: "Org summary" },
                { value: "org_transcript", label: "Org transcript" },
                { value: "org_evidence", label: "Org evidence" },
              ]}
            />
          }
        />
        <Row
          title="Save policy"
          description={cloudState.featureGrants.length > 0 ? `${cloudState.featureGrants.length} org feature grants are visible.` : "No org feature grants are visible yet."}
          control={
            <button type="button" className="settings-btn" onClick={() => void onSavePolicy()} disabled={!canUseOrgActions}>
              Save
            </button>
          }
        />
      </Card>

      <Card title="Enterprise setup">
        <Row
          title="Plan state"
          description={plan === "enterprise" ? "Enterprise entitlements are active for the selected context." : "Enterprise setup is available after enterprise entitlement activation."}
          control={<StatusPill label={enterprisePlanStatus.label} tone={enterprisePlanStatus.tone} />}
        />
        <Row
          title="SSO / SCIM"
          description="WorkOS or equivalent identity-provider enrollment is not active for this organization."
          control={<StatusPill label="Not configured" tone="warn" />}
        />
        <Row
          title="Domain verification"
          description="Verified domains and discovery are not active for this organization."
          control={<StatusPill label="Not configured" tone="warn" />}
        />
        <Row
          title="Audit / export"
          description="Audit/export capability is shown from entitlements; setup remains explicit."
          control={<StatusPill label={enterpriseAudit.label} tone={enterpriseAudit.tone} />}
        />
        <Row
          title="Request setup"
          description="Submits an Enterprise setup request to the Team admin API."
          control={
            <button type="button" className="settings-btn" onClick={() => void onRequestEnterpriseSetup()} disabled={!canUseOrgActions}>
              Request
            </button>
          }
        />
      </Card>

      <Card title="Managed Service Hooks">
        <Row
          title="Run history"
          description="Org-scoped run history is gated by entitlements and archive policy."
          control={<StatusPill label={orgRunHistory.label} tone={orgRunHistory.tone} />}
        />
        <Row
          title="Mobile relay"
          description="Managed mobile relay transport is owned by the mobile arc; this page only shows the entitlement hook."
          control={<StatusPill label={mobileRelay.label} tone={mobileRelay.tone} />}
        />
        <Row
          title="LLM token relay"
          description="Managed token relay routing and budget enforcement are owned by the relay arc; this page only shows the entitlement hook."
          control={<StatusPill label={llmTokenRelay.label} tone={llmTokenRelay.tone} />}
        />
      </Card>
    </>
  );
}
