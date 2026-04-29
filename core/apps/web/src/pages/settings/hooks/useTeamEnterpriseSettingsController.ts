import type { Dispatch, SetStateAction } from "react";
import { useCallback, useEffect, useState } from "react";
import type { SupabaseClient, User } from "@supabase/supabase-js";
import { errorMessage } from "../../../utils/errorMessage";
import {
  trackCheckoutStarted,
  trackFeatureUsed,
  trackSubscribeCtaClicked,
} from "../../../utils/analytics";
import type { BillingInterval } from "../billingCheckoutFlow";
import type { PlanType } from "../entitlementAnalytics";
import type { SectionId } from "../SettingsPage.types";
import {
  DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT,
  fetchTeamEnterpriseCloudState,
  invokeTeamEnterpriseAdminAction,
  startTeamBillingCheckout,
  type EntitlementsSnapshot,
  type MembershipRole,
  type TeamEnterpriseCloudState,
  type TeamEnterprisePolicyDraft,
} from "../teamEnterpriseSettingsApi";
import { writeStoredTeamEnterpriseActiveOrgId } from "../teamEnterpriseSettingsStorage";

type RefreshEntitlements = (opts?: {
  force?: boolean;
  silent?: boolean;
  activeOrgId?: string | null;
}) => Promise<EntitlementsSnapshot | null>;

export type SettingsTeamEnterpriseController = {
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
  setOrgName: (value: string) => void;
  inviteEmail: string;
  setInviteEmail: (value: string) => void;
  inviteRole: MembershipRole;
  setInviteRole: (value: MembershipRole) => void;
  seatTarget: string;
  setSeatTarget: (value: string) => void;
  policyDraft: TeamEnterprisePolicyDraft;
  setPolicyDraft: (value: TeamEnterprisePolicyDraft) => void;
  onRefresh: () => Promise<void>;
  onSelectOrg: (orgId: string) => Promise<void>;
  onCreateOrg: () => Promise<void>;
  onInviteMember: () => Promise<void>;
  onAcceptInvite: (inviteToken: string) => Promise<void>;
  onUpdateSeats: () => Promise<void>;
  onSavePolicy: () => Promise<void>;
  onStartTeamCheckout: (interval: BillingInterval) => Promise<void>;
  onRequestEnterpriseSetup: () => Promise<void>;
};

type Params = {
  active: SectionId;
  supabase: SupabaseClient | null;
  billingUser: User | null;
  billingReturnPath: string;
  entitlementsBusy: boolean;
  plan: PlanType;
  entitlements: EntitlementsSnapshot | null;
  requestedActiveOrgId: string | null;
  setRequestedActiveOrgId: Dispatch<SetStateAction<string | null>>;
  refreshEntitlements: RefreshEntitlements;
};

const EMPTY_TEAM_ENTERPRISE_CLOUD_STATE: TeamEnterpriseCloudState = {
  orgs: [],
  activeOrgId: null,
  activeOrg: null,
  billingSubjectId: null,
  invites: [],
  subscriptions: [],
  featureGrants: [],
  adminState: null,
  memberDirectoryAvailable: false,
};

export function useTeamEnterpriseSettingsController({
  active,
  supabase,
  billingUser,
  billingReturnPath,
  entitlementsBusy,
  plan,
  entitlements,
  requestedActiveOrgId,
  setRequestedActiveOrgId,
  refreshEntitlements,
}: Params): SettingsTeamEnterpriseController {
  const [cloudState, setCloudState] = useState<TeamEnterpriseCloudState>(EMPTY_TEAM_ENTERPRISE_CLOUD_STATE);
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudError, setCloudError] = useState<string | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [orgName, setOrgName] = useState("");
  const [inviteEmail, setInviteEmail] = useState("");
  const [inviteRole, setInviteRole] = useState<MembershipRole>("member");
  const [seatTarget, setSeatTarget] = useState("");
  const [policyDraft, setPolicyDraft] = useState<TeamEnterprisePolicyDraft>(
    DEFAULT_TEAM_ENTERPRISE_POLICY_DRAFT,
  );

  const refreshCloudState = useCallback(
    async (opts?: { requestedActiveOrgId?: string | null; silent?: boolean }) => {
      if (!supabase || !billingUser) {
        setCloudState(EMPTY_TEAM_ENTERPRISE_CLOUD_STATE);
        return EMPTY_TEAM_ENTERPRISE_CLOUD_STATE;
      }
      const nextRequestedActiveOrgId =
        opts && "requestedActiveOrgId" in opts ? (opts.requestedActiveOrgId ?? null) : requestedActiveOrgId;
      if (!opts?.silent) {
        setCloudBusy(true);
        setCloudError(null);
      }
      try {
        const next = await fetchTeamEnterpriseCloudState({
          client: supabase,
          requestedActiveOrgId: nextRequestedActiveOrgId,
        });
        setCloudState(next);
        if (next.adminState?.seatTarget) {
          setSeatTarget(String(next.adminState.seatTarget));
        }
        if (next.adminState?.policy) {
          setPolicyDraft(next.adminState.policy);
        }
        if (next.activeOrgId !== nextRequestedActiveOrgId) {
          setRequestedActiveOrgId(next.activeOrgId);
          writeStoredTeamEnterpriseActiveOrgId(next.activeOrgId);
        }
        return next;
      } catch (error: unknown) {
        if (!opts?.silent) {
          setCloudError(errorMessage(error));
        }
        return null;
      } finally {
        if (!opts?.silent) {
          setCloudBusy(false);
        }
      }
    },
    [billingUser, requestedActiveOrgId, setRequestedActiveOrgId, supabase],
  );

  useEffect(() => {
    if (active !== "team_enterprise") return;
    void refreshCloudState();
  }, [active, refreshCloudState]);

  const refreshAll = useCallback(
    async (activeOrgId?: string | null) => {
      const nextRequestedActiveOrgId = activeOrgId ?? requestedActiveOrgId;
      await refreshEntitlements({ force: true, silent: true, activeOrgId: nextRequestedActiveOrgId });
      await refreshCloudState({ requestedActiveOrgId: nextRequestedActiveOrgId, silent: true });
    },
    [refreshCloudState, refreshEntitlements, requestedActiveOrgId],
  );

  const selectOrg = useCallback(
    async (orgId: string) => {
      if (!orgId) return;
      setRequestedActiveOrgId(orgId);
      writeStoredTeamEnterpriseActiveOrgId(orgId);
      setActionError(null);
      setActionNotice(null);
      setCloudBusy(true);
      try {
        await refreshEntitlements({ force: true, activeOrgId: orgId, silent: true });
        await refreshCloudState({ requestedActiveOrgId: orgId, silent: true });
      } finally {
        setCloudBusy(false);
      }
    },
    [refreshCloudState, refreshEntitlements, setRequestedActiveOrgId],
  );

  const runAction = useCallback(
    async (run: () => Promise<void>, successMessage: string) => {
      if (!supabase || !billingUser) {
        setActionError("Sign in required to manage Team and Enterprise settings.");
        return;
      }
      setActionBusy(true);
      setActionError(null);
      setActionNotice(null);
      try {
        await run();
        setActionNotice(successMessage);
        await refreshAll();
      } catch (error: unknown) {
        setActionError(errorMessage(error));
      } finally {
        setActionBusy(false);
      }
    },
    [billingUser, refreshAll, supabase],
  );

  const createOrg = useCallback(async () => {
    const name = orgName.trim();
    if (!name) {
      setActionError("Organization name is required.");
      return;
    }
    await runAction(async () => {
      if (!supabase) throw new Error("Supabase is not configured.");
      await invokeTeamEnterpriseAdminAction(supabase, { action: "create_organization", name });
      setOrgName("");
      trackFeatureUsed("org_created");
    }, "Organization create request submitted.");
  }, [orgName, runAction, supabase]);

  const inviteMember = useCallback(async () => {
    const activeOrgId = cloudState.activeOrgId;
    const email = inviteEmail.trim();
    if (!activeOrgId) {
      setActionError("Select an organization before inviting members.");
      return;
    }
    if (!email) {
      setActionError("Invite email is required.");
      return;
    }
    await runAction(async () => {
      if (!supabase) throw new Error("Supabase is not configured.");
      await invokeTeamEnterpriseAdminAction(supabase, {
        action: "invite_member",
        organization_id: activeOrgId,
        email,
        role: inviteRole,
      });
      setInviteEmail("");
      trackFeatureUsed("org_invite_sent");
    }, "Invite request submitted.");
  }, [cloudState.activeOrgId, inviteEmail, inviteRole, runAction, supabase]);

  const acceptInvite = useCallback(
    async (inviteToken: string) => {
      const token = inviteToken.trim();
      if (!token) return;
      await runAction(async () => {
        if (!supabase) throw new Error("Supabase is not configured.");
        await invokeTeamEnterpriseAdminAction(supabase, { action: "accept_invite", token });
        trackFeatureUsed("org_invite_accepted");
      }, "Invite accept request submitted.");
    },
    [runAction, supabase],
  );

  const updateSeats = useCallback(async () => {
    const activeOrgId = cloudState.activeOrgId;
    const seats = Number.parseInt(seatTarget, 10);
    if (!activeOrgId) {
      setActionError("Select an organization before updating seats.");
      return;
    }
    if (!Number.isFinite(seats) || seats < 1) {
      setActionError("Seat count must be a positive whole number.");
      return;
    }
    await runAction(async () => {
      if (!supabase) throw new Error("Supabase is not configured.");
      await invokeTeamEnterpriseAdminAction(supabase, {
        action: "update_seats",
        organization_id: activeOrgId,
        seats,
      });
      trackFeatureUsed("org_seats_updated");
    }, "Seat update request submitted.");
  }, [cloudState.activeOrgId, runAction, seatTarget, supabase]);

  const savePolicy = useCallback(async () => {
    const activeOrgId = cloudState.activeOrgId;
    if (!activeOrgId) {
      setActionError("Select an organization before saving policy.");
      return;
    }
    await runAction(async () => {
      if (!supabase) throw new Error("Supabase is not configured.");
      await invokeTeamEnterpriseAdminAction(supabase, {
        action: "update_policy",
        organization_id: activeOrgId,
        policy: policyDraft,
      });
      trackFeatureUsed("org_policy_updated");
    }, "Policy update request submitted.");
  }, [cloudState.activeOrgId, policyDraft, runAction, supabase]);

  const startCheckout = useCallback(
    async (interval: BillingInterval) => {
      if (!supabase || !billingUser) {
        setActionError("Sign in required to start Team checkout.");
        return;
      }
      const activeOrgId = cloudState.activeOrgId;
      const billingSubjectId = cloudState.billingSubjectId;
      if (!activeOrgId || !billingSubjectId) {
        setActionError("Select an organization with a billing subject before starting Team checkout.");
        return;
      }
      setActionBusy(true);
      setActionError(null);
      setActionNotice(null);
      try {
        trackSubscribeCtaClicked(interval);
        const url = await startTeamBillingCheckout({
          client: supabase,
          organizationId: activeOrgId,
          billingSubjectId,
          interval,
          returnPath: billingReturnPath,
          seatCount: Number.parseInt(seatTarget, 10) || null,
        });
        trackCheckoutStarted(interval);
        window.location.href = url;
      } catch (error: unknown) {
        setActionError(errorMessage(error));
        setActionBusy(false);
      }
    },
    [billingReturnPath, billingUser, cloudState.activeOrgId, cloudState.billingSubjectId, seatTarget, supabase],
  );

  const requestEnterpriseSetup = useCallback(async () => {
    const activeOrgId = cloudState.activeOrgId;
    if (!activeOrgId) {
      setActionError("Select an organization before requesting Enterprise setup.");
      return;
    }
    await runAction(async () => {
      if (!supabase) throw new Error("Supabase is not configured.");
      await invokeTeamEnterpriseAdminAction(supabase, {
        action: "request_enterprise_setup",
        organization_id: activeOrgId,
      });
      trackFeatureUsed("enterprise_setup_requested");
    }, "Enterprise setup request submitted.");
  }, [cloudState.activeOrgId, runAction, supabase]);

  return {
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
    setOrgName,
    inviteEmail,
    setInviteEmail,
    inviteRole,
    setInviteRole,
    seatTarget,
    setSeatTarget,
    policyDraft,
    setPolicyDraft,
    onRefresh: refreshAll,
    onSelectOrg: selectOrg,
    onCreateOrg: createOrg,
    onInviteMember: inviteMember,
    onAcceptInvite: acceptInvite,
    onUpdateSeats: updateSeats,
    onSavePolicy: savePolicy,
    onStartTeamCheckout: startCheckout,
    onRequestEnterpriseSetup: requestEnterpriseSetup,
  };
}
