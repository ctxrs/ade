import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";
import { errorMessage } from "../../../utils/errorMessage";
import {
  trackCheckoutStarted,
  trackEntitlementActivated,
  trackFeatureUsed,
  trackPlanViewed,
  trackSubscribeCtaClicked,
} from "../../../utils/analytics";
import {
  ENTITLEMENTS_CACHE_KEY,
  ENTITLEMENTS_CACHE_TTL_MS,
  readCachedValue,
  shouldUseCachedValue,
  writeCachedValue,
} from "../../../utils/entitlementsCache";
import { runBillingCheckoutFlow } from "../billingCheckoutFlow";
import { type PlanType, shouldTrackEntitlementActivated } from "../entitlementAnalytics";
import type { SectionId } from "../SettingsPage.types";
import {
  fetchAccountSession,
  fetchEntitlementsSnapshot,
  invokePersonalBillingCheckout,
  isCtxControlPlaneConfigured,
  openBillingPortal,
  requestManagedTunnelGrant,
  signOutAccount,
  startAccountAuth,
  syncBillingCheckout,
  type CtxBillingUser,
  type EntitlementsSnapshot,
} from "../teamEnterpriseSettingsApi";
import {
  readStoredTeamEnterpriseActiveOrgId,
  teamEnterpriseEntitlementsCacheKey,
} from "../teamEnterpriseSettingsStorage";
import { useMobileAccessController } from "./useMobileAccessController";
import {
  type SettingsTeamEnterpriseController,
  useTeamEnterpriseSettingsController,
} from "./useTeamEnterpriseSettingsController";

type SettingsBillingController = {
  checkoutStatus: string | null;
  billingUser: CtxBillingUser | null;
  billingEmail: string;
  setBillingEmail: (value: string) => void;
  billingPassword: string;
  setBillingPassword: (value: string) => void;
  billingBusy: boolean;
  billingError: string | null;
  entitlementsBusy: boolean;
  plan: PlanType;
  proEnabled: boolean;
  onSignIn: () => Promise<void>;
  onSignUp: () => Promise<void>;
  onSignOut: () => Promise<void>;
  onStartCheckout: (interval: "month" | "year") => Promise<void>;
  onOpenPortal: () => Promise<void>;
};

type SettingsMobileAccessController = {
  billingUser: CtxBillingUser | null;
  entitlementsBusy: boolean;
  proEnabled: boolean;
  mobileStatus: MobileAccessStatus | null;
  mobileStatusBusy: boolean;
  mobileStatusError: string | null;
  mobileEnableBusy: boolean;
  mobileEnableError: string | null;
  mobileQr: EnableMobileAccessResponse | null;
  onEnable: () => Promise<void>;
  onDisable: () => Promise<void>;
};

type SettingsAccountController = {
  controlPlaneConfigured: boolean;
  billing: SettingsBillingController;
  mobileAccess: SettingsMobileAccessController;
  teamEnterprise: SettingsTeamEnterpriseController;
};

type Params = {
  active: SectionId;
  billingReturnPath: string;
  checkoutStatus: string | null;
  checkoutSessionId: string | null;
  clearCheckoutStatus: () => void;
};

export function useSettingsAccountController({
  active,
  billingReturnPath,
  checkoutStatus,
  checkoutSessionId,
  clearCheckoutStatus,
}: Params): SettingsAccountController {
  const controlPlaneConfigured = useMemo(() => isCtxControlPlaneConfigured(), []);
  const [billingUser, setBillingUser] = useState<CtxBillingUser | null>(null);
  const [billingEmail, setBillingEmail] = useState("");
  const [billingPassword, setBillingPassword] = useState("");
  const [billingBusy, setBillingBusy] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);
  const [entitlementsBusy, setEntitlementsBusy] = useState(false);
  const billingViewTrackedRef = useRef(false);
  const [teamRequestedActiveOrgId, setTeamRequestedActiveOrgId] = useState<string | null>(() =>
    readStoredTeamEnterpriseActiveOrgId(),
  );
  const [entitlements, setEntitlements] = useState<EntitlementsSnapshot | null>(() => {
    try {
      const cached = readCachedValue<EntitlementsSnapshot>(
        window.localStorage,
        teamEnterpriseEntitlementsCacheKey(readStoredTeamEnterpriseActiveOrgId()),
      );
      return cached?.value ?? null;
    } catch {
      return null;
    }
  });
  const priorPlanRef = useRef<PlanType | null>(entitlements?.plan_type ?? null);

  useEffect(() => {
    if (!controlPlaneConfigured) return;

    let cancelled = false;
    fetchAccountSession()
      .then((user) => {
        if (cancelled) return;
        setBillingUser(user);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [controlPlaneConfigured]);

  const refreshEntitlements = useCallback(async (opts?: { force?: boolean; silent?: boolean; activeOrgId?: string | null }) => {
    if (!controlPlaneConfigured) return null;
    const activeOrgId = opts && "activeOrgId" in opts ? (opts.activeOrgId ?? null) : teamRequestedActiveOrgId;
    const cacheKey = teamEnterpriseEntitlementsCacheKey(activeOrgId);
    const cached = readCachedValue<EntitlementsSnapshot>(window.localStorage, cacheKey);
    if (!opts?.force && cached && shouldUseCachedValue(cached, ENTITLEMENTS_CACHE_TTL_MS)) {
      setEntitlements(cached.value);
      if (!opts?.silent) {
        setEntitlementsBusy(false);
      }
      return cached.value;
    }
    if (!opts?.silent) {
      setEntitlementsBusy(true);
      setBillingError(null);
    }
    try {
      const next = await fetchEntitlementsSnapshot({ activeOrgId });
      setEntitlements(next);
      if (next) {
        writeCachedValue(window.localStorage, cacheKey, next);
      }
      return next;
    } catch (error: unknown) {
      if (!opts?.silent) {
        setBillingError(errorMessage(error));
      }
      return null;
    } finally {
      if (!opts?.silent) {
        setEntitlementsBusy(false);
      }
    }
  }, [controlPlaneConfigured, teamRequestedActiveOrgId]);

  useEffect(() => {
    if (!controlPlaneConfigured) return;
    void refreshEntitlements();
  }, [billingUser, controlPlaneConfigured, refreshEntitlements]);

  useEffect(() => {
    if (active !== "billing") {
      billingViewTrackedRef.current = false;
      return;
    }
    if (billingViewTrackedRef.current) return;
    billingViewTrackedRef.current = true;
    trackPlanViewed("settings_billing");
    trackFeatureUsed("billing_settings_viewed");
  }, [active]);

  useEffect(() => {
    const nextPlan = entitlements?.plan_type ?? null;
    const priorPlan = priorPlanRef.current;
    if (nextPlan === null) {
      return;
    }
    if (priorPlan === null) {
      priorPlanRef.current = nextPlan;
      return;
    }
    if (shouldTrackEntitlementActivated(priorPlan, nextPlan)) {
      trackEntitlementActivated(nextPlan);
    }
    priorPlanRef.current = nextPlan;
  }, [entitlements?.plan_type]);

  const isPaidPlan = useCallback((snapshot: EntitlementsSnapshot | null) => {
    return Boolean(snapshot?.plan_type && snapshot.plan_type !== "free_local");
  }, []);

  useEffect(() => {
    if (!controlPlaneConfigured || checkoutStatus !== "success") return;
    let cancelled = false;
    let attempt = 0;
    const maxAttempts = 6;
    let syncStarted = false;

    const syncCheckout = async () => {
      if (syncStarted) return;
      syncStarted = true;
      try {
        await syncBillingCheckout(checkoutSessionId);
      } catch (error: unknown) {
        setBillingError(errorMessage(error));
      }
    };

    const poll = async () => {
      if (cancelled || attempt >= maxAttempts) return;
      attempt += 1;
      await syncCheckout();
      window.localStorage.removeItem(ENTITLEMENTS_CACHE_KEY);
      const next = await refreshEntitlements({ force: true, silent: true });
      if (next && isPaidPlan(next)) {
        clearCheckoutStatus();
        return;
      }
      if (!cancelled && attempt < maxAttempts) {
        window.setTimeout(poll, 2000);
      }
    };

    void poll();
    return () => {
      cancelled = true;
    };
  }, [
    checkoutSessionId,
    checkoutStatus,
    clearCheckoutStatus,
    isPaidPlan,
    refreshEntitlements,
    controlPlaneConfigured,
  ]);

  const getAuthToken = useCallback(async (): Promise<string> => {
    return requestManagedTunnelGrant();
  }, []);

  const {
    mobileStatus,
    mobileStatusBusy,
    mobileStatusError,
    mobileEnableBusy,
    mobileEnableError,
    mobileQr,
    refreshMobileAccess,
    handleEnableMobile,
    handleDisableMobile,
  } = useMobileAccessController({ getAuthToken: controlPlaneConfigured ? getAuthToken : null });

  useEffect(() => {
    if (active !== "mobile_access") return;
    void refreshMobileAccess();
  }, [active, refreshMobileAccess]);

  const doSignIn = useCallback(async () => {
    if (!controlPlaneConfigured) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      await startAccountAuth(billingEmail.trim(), billingPassword, "sign_in");
      setBillingUser(await fetchAccountSession());
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [billingEmail, billingPassword, controlPlaneConfigured]);

  const doSignUp = useCallback(async () => {
    if (!controlPlaneConfigured) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      await startAccountAuth(billingEmail.trim(), billingPassword, "sign_up");
      setBillingUser(await fetchAccountSession());
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [billingEmail, billingPassword, controlPlaneConfigured]);

  const doSignOut = useCallback(async () => {
    if (!controlPlaneConfigured) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      await signOutAccount();
      setBillingUser(null);
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [controlPlaneConfigured]);

  const startCheckout = useCallback(
    async (interval: "month" | "year") => {
      if (!controlPlaneConfigured) return;
      setBillingBusy(true);
      setBillingError(null);
      try {
        const url = await runBillingCheckoutFlow({
          interval,
          returnPath: billingReturnPath,
          invokeCheckout: ({ interval: nextInterval, returnPath }) =>
            invokePersonalBillingCheckout({ interval: nextInterval, returnPath }),
          trackSubscribeCtaClicked,
          trackCheckoutStarted,
        });
        window.location.href = url;
      } catch (error: unknown) {
        setBillingError(errorMessage(error));
        setBillingBusy(false);
      }
    },
    [billingReturnPath, controlPlaneConfigured],
  );

  const openPortal = useCallback(async () => {
    if (!controlPlaneConfigured) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      const url = await openBillingPortal(billingReturnPath);
      window.location.href = url;
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
      setBillingBusy(false);
    }
  }, [billingReturnPath, controlPlaneConfigured]);

  const plan = entitlements?.plan_type ?? "free_local";
  const proEnabled =
    entitlements?.features?.remote_mobile_access === "enabled" ||
    entitlements?.features?.mobile_relay === "enabled";
  const teamEnterprise = useTeamEnterpriseSettingsController({
    active,
    billingUser,
    billingReturnPath,
    entitlementsBusy,
    plan,
    entitlements,
    requestedActiveOrgId: teamRequestedActiveOrgId,
    setRequestedActiveOrgId: setTeamRequestedActiveOrgId,
    refreshEntitlements,
  });

  return {
    controlPlaneConfigured,
    billing: {
      checkoutStatus,
      billingUser,
      billingEmail,
      setBillingEmail,
      billingPassword,
      setBillingPassword,
      billingBusy,
      billingError,
      entitlementsBusy,
      plan,
      proEnabled,
      onSignIn: doSignIn,
      onSignUp: doSignUp,
      onSignOut: doSignOut,
      onStartCheckout: startCheckout,
      onOpenPortal: openPortal,
    },
    mobileAccess: {
      billingUser,
      entitlementsBusy,
      proEnabled,
      mobileStatus,
      mobileStatusBusy,
      mobileStatusError,
      mobileEnableBusy,
      mobileEnableError,
      mobileQr,
      onEnable: handleEnableMobile,
      onDisable: handleDisableMobile,
    },
    teamEnterprise,
  };
}

export type {
  SettingsAccountController,
  SettingsBillingController,
  SettingsMobileAccessController,
  SettingsTeamEnterpriseController,
};
