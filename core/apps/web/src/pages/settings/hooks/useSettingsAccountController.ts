import type { User } from "@supabase/supabase-js";
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
import { getSupabaseClient } from "../../../utils/supabaseClient";
import { runBillingCheckoutFlow } from "../billingCheckoutFlow";
import { type PlanType, shouldTrackEntitlementActivated } from "../entitlementAnalytics";
import type { SectionId } from "../SettingsPage.types";
import { useMobileAccessController } from "./useMobileAccessController";

type EntitlementsSnapshot = {
  plan_type: PlanType;
  features: Record<string, "enabled" | "disabled">;
  expires_at?: string | null;
  grace_expires_at?: string | null;
};

type SettingsBillingController = {
  checkoutStatus: string | null;
  billingUser: User | null;
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
  billingUser: User | null;
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
  supabaseConfigured: boolean;
  billing: SettingsBillingController;
  mobileAccess: SettingsMobileAccessController;
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
  const supabase = useMemo(() => getSupabaseClient(), []);
  const [billingUser, setBillingUser] = useState<User | null>(null);
  const [billingEmail, setBillingEmail] = useState("");
  const [billingPassword, setBillingPassword] = useState("");
  const [billingBusy, setBillingBusy] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);
  const [entitlementsBusy, setEntitlementsBusy] = useState(false);
  const billingViewTrackedRef = useRef(false);
  const [entitlements, setEntitlements] = useState<EntitlementsSnapshot | null>(() => {
    try {
      const cached = readCachedValue<EntitlementsSnapshot>(window.localStorage, ENTITLEMENTS_CACHE_KEY);
      return cached?.value ?? null;
    } catch {
      return null;
    }
  });
  const priorPlanRef = useRef<PlanType | null>(entitlements?.plan_type ?? null);

  useEffect(() => {
    if (!supabase) return;

    let cancelled = false;
    supabase.auth
      .getUser()
      .then(({ data }) => {
        if (cancelled) return;
        setBillingUser(data.user ?? null);
      })
      .catch(() => {});

    const { data } = supabase.auth.onAuthStateChange((_event, session) => {
      setBillingUser(session?.user ?? null);
    });

    return () => {
      cancelled = true;
      data.subscription.unsubscribe();
    };
  }, [supabase]);

  const refreshEntitlements = useCallback(async (opts?: { force?: boolean; silent?: boolean }) => {
    if (!supabase) return null;
    const cached = readCachedValue<EntitlementsSnapshot>(window.localStorage, ENTITLEMENTS_CACHE_KEY);
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
      const response = await supabase.functions.invoke("entitlements", { method: "GET" });
      if (response.error) throw response.error;
      const next = (response.data ?? null) as EntitlementsSnapshot | null;
      setEntitlements(next);
      if (next) {
        writeCachedValue(window.localStorage, ENTITLEMENTS_CACHE_KEY, next);
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
  }, [supabase]);

  useEffect(() => {
    if (!supabase) return;
    void refreshEntitlements();
  }, [billingUser, refreshEntitlements, supabase]);

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
    if (!supabase || checkoutStatus !== "success") return;
    let cancelled = false;
    let attempt = 0;
    const maxAttempts = 6;
    let syncStarted = false;

    const syncCheckout = async () => {
      if (syncStarted) return;
      syncStarted = true;
      try {
        const response = await supabase.functions.invoke("billing-sync", {
          body: checkoutSessionId ? { checkout_session_id: checkoutSessionId } : {},
        });
        if (response.error) throw response.error;
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
    supabase,
  ]);

  const getAuthToken = useCallback(async (): Promise<string> => {
    if (!supabase) {
      throw new Error("Supabase is not configured.");
    }
    const { data, error } = await supabase.auth.getSession();
    if (error) throw error;
    const token = data.session?.access_token;
    if (!token) {
      throw new Error("Sign in required to manage mobile access.");
    }
    return token;
  }, [supabase]);

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
  } = useMobileAccessController({ getAuthToken: supabase ? getAuthToken : null });

  useEffect(() => {
    if (active !== "mobile_access") return;
    void refreshMobileAccess();
  }, [active, refreshMobileAccess]);

  const doSignIn = useCallback(async () => {
    if (!supabase) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      const { error } = await supabase.auth.signInWithPassword({
        email: billingEmail.trim(),
        password: billingPassword,
      });
      if (error) throw error;
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [billingEmail, billingPassword, supabase]);

  const doSignUp = useCallback(async () => {
    if (!supabase) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      const { error } = await supabase.auth.signUp({
        email: billingEmail.trim(),
        password: billingPassword,
      });
      if (error) throw error;
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [billingEmail, billingPassword, supabase]);

  const doSignOut = useCallback(async () => {
    if (!supabase) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      const { error } = await supabase.auth.signOut();
      if (error) throw error;
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
    } finally {
      setBillingBusy(false);
    }
  }, [supabase]);

  const startCheckout = useCallback(
    async (interval: "month" | "year") => {
      if (!supabase) return;
      setBillingBusy(true);
      setBillingError(null);
      try {
        const url = await runBillingCheckoutFlow({
          interval,
          returnPath: billingReturnPath,
          invokeCheckout: ({ interval: nextInterval, returnPath }) =>
            supabase.functions.invoke("billing-checkout", {
              body: { interval: nextInterval, return_path: returnPath },
            }),
          trackSubscribeCtaClicked,
          trackCheckoutStarted,
        });
        window.location.href = url;
      } catch (error: unknown) {
        setBillingError(errorMessage(error));
        setBillingBusy(false);
      }
    },
    [billingReturnPath, supabase],
  );

  const openPortal = useCallback(async () => {
    if (!supabase) return;
    setBillingBusy(true);
    setBillingError(null);
    try {
      const response = await supabase.functions.invoke("billing-portal", {
        body: { return_path: billingReturnPath },
      });
      if (response.error) throw response.error;
      const data =
        response.data && typeof response.data === "object" ? (response.data as Record<string, unknown>) : null;
      const url = typeof data?.url === "string" ? data.url.trim() : "";
      if (!url) throw new Error("Portal URL missing.");
      window.location.href = url;
    } catch (error: unknown) {
      setBillingError(errorMessage(error));
      setBillingBusy(false);
    }
  }, [billingReturnPath, supabase]);

  const plan = entitlements?.plan_type ?? "free_local";
  const proEnabled = entitlements?.features?.remote_mobile_access === "enabled";

  return {
    supabaseConfigured: Boolean(supabase),
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
  };
}

export type {
  EntitlementsSnapshot,
  SettingsAccountController,
  SettingsBillingController,
  SettingsMobileAccessController,
};
