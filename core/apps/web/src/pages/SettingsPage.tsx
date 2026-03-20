import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import type { User } from "@supabase/supabase-js";
import {
  ExecutionSettings as ApiExecutionSettings,
  DevRestartProvidersResult,
  EnableMobileAccessResponse,
  MobileAccessStatus,
  PublicExecutionSettings,
  ResourceGovernanceLimits,
  ResourceGovernanceSettings,
  ResourceGovernanceStatus,
  ResourceUtilization,
  SandboxingSettings,
  TelemetrySettings,
  UpdateSettingsRequest,
  devRestartProviders,
  disableMobileAccess,
  enableMobileAccess,
  getMobileAccessStatus,
  Workspace,
  getResourceUtilization,
  getSettings,
  idToString,
  listWorkspaces,
  updateSettings,
} from "../api/client";
import {
  type DesktopEditorSettings,
  isDesktopApp,
} from "../utils/desktop";
import { ensureDesktopNotificationPermission } from "../utils/desktopNotifications";
import { errorMessage } from "../utils/errorMessage";
import {
  trackCheckoutStarted,
  trackEntitlementActivated,
  trackFeatureUsed,
  trackPlanViewed,
  trackSubscribeCtaClicked,
} from "../utils/analytics";
import {
  applyTheme,
  readCssVar,
  resolveThemeMode,
  setStoredTheme,
  useThemeVariant,
  type ThemeMode,
} from "../utils/theme";
import {
  ENTITLEMENTS_CACHE_KEY,
  ENTITLEMENTS_CACHE_TTL_MS,
  readCachedValue,
  shouldUseCachedValue,
  writeCachedValue,
} from "../utils/entitlementsCache";
import { getSupabaseClient } from "../utils/supabaseClient";
import {
  getClientSettingsState,
  loadClientSettings,
  subscribeClientSettings,
  updateClientSettings,
} from "../state/clientSettings";
import {
  SECTIONS,
} from "./SettingsPage.constants";
import { SettingsContentRouter } from "./settings/SettingsContentRouter";
import { SettingsShell } from "./settings/SettingsShell";
import { useDesktopEditorSettingsController } from "./settings/hooks/useDesktopEditorSettingsController";
import { useSettingsActions } from "./settings/useSettingsActions";
import { useSettingsState } from "./settings/useSettingsState";
import type { SectionId, SettingsSectionMeta } from "./SettingsPage.types";
import { runBillingCheckoutFlow } from "./settings/billingCheckoutFlow";
import { shouldTrackEntitlementActivated } from "./settings/entitlementAnalytics";
import {
  executionSettingsStableKey,
  formatGiB,
  parseGiB,
  sectionFromHash,
} from "./SettingsPage.utils";

const DEFAULT_MACHINE_IDLE_SHUTDOWN_SECONDS = 60 * 60;
const DEFAULT_MACHINE_HOST_PRESSURE_SWAP_THRESHOLD_MB = 1024;
export const MIN_MACHINE_IDLE_SHUTDOWN_SECONDS = 60;

function defaultExecutionSettings(): ApiExecutionSettings {
  return {
    mode: "host",
    container: {
      runtime: "podman",
      mount_mode: "host_mounted",
      network_mode: "llm_only",
      allowlist: [],
      image: null,
      machine: {
        memory_profile: "economy",
        custom_memory_mb: null,
        idle_shutdown_seconds: DEFAULT_MACHINE_IDLE_SHUTDOWN_SECONDS,
        host_pressure_swap_threshold_mb: DEFAULT_MACHINE_HOST_PRESSURE_SWAP_THRESHOLD_MB,
      },
    },
  };
}

function normalizeExecutionSettings(
  value: ApiExecutionSettings | PublicExecutionSettings | null | undefined,
): ApiExecutionSettings {
  const fallback = defaultExecutionSettings();
  return {
    mode: value?.mode ?? fallback.mode,
    container: {
      runtime: value?.container?.runtime ?? fallback.container.runtime,
      mount_mode: value?.container?.mount_mode ?? fallback.container.mount_mode,
      network_mode: value?.container?.network_mode ?? fallback.container.network_mode,
      allowlist: value?.container?.allowlist ?? fallback.container.allowlist,
      image: value?.container?.image ?? fallback.container.image,
      machine: {
        memory_profile: value?.container?.machine?.memory_profile ?? fallback.container.machine.memory_profile,
        custom_memory_mb: value?.container?.machine?.custom_memory_mb ?? fallback.container.machine.custom_memory_mb,
        idle_shutdown_seconds:
          value?.container?.machine?.idle_shutdown_seconds ?? fallback.container.machine.idle_shutdown_seconds,
        host_pressure_swap_threshold_mb:
          value?.container?.machine?.host_pressure_swap_threshold_mb
          ?? fallback.container.machine.host_pressure_swap_threshold_mb,
      },
    },
  };
}

function parseUnsignedInteger(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed) || parsed < 0) return null;
  return Math.round(parsed);
}

export function canSaveSandboxMachineSettings({
  machineHostPressureSwapThresholdMb,
  machineIdleShutdownSeconds,
}: {
  machineHostPressureSwapThresholdMb: string;
  machineIdleShutdownSeconds: string;
}): boolean {
  const idle = parseUnsignedInteger(machineIdleShutdownSeconds);
  if (idle === null || idle < MIN_MACHINE_IDLE_SHUTDOWN_SECONDS) return false;
  const threshold = parseUnsignedInteger(machineHostPressureSwapThresholdMb);
  return threshold !== null;
}

export default function SettingsPage() {
  const location = useLocation();
  const navigate = useNavigate();
  const { active, setActive, query, setQuery } = useSettingsState({
    initialActive: sectionFromHash(window.location.hash) ?? "general",
  });
  const supabase = useMemo(() => getSupabaseClient(), []);
  const devToolsEnabled = import.meta.env.DEV;
  const [theme, setTheme] = useState<ThemeMode>(() => resolveThemeMode());
  const themeVariant = useThemeVariant();
  const qrFgColor = readCssVar("--text", themeVariant === "dark" ? "#d4d4d4" : "#3b3b3b");

  const billingReturnPath = useMemo(() => {
    const params = new URLSearchParams(location.search);
    params.delete("checkout");
    params.delete("session_id");
    const search = params.toString();
    return `${location.pathname}${search ? `?${search}` : ""}#billing`;
  }, [location.pathname, location.search]);

  const { clearCheckoutStatus } = useSettingsActions({
    pathname: location.pathname,
    search: location.search,
    hash: location.hash,
    navigate,
  });

  type EntitlementsSnapshot = {
    plan_type: "free_local" | "pro" | "team" | "enterprise";
    features: Record<string, "enabled" | "disabled">;
    expires_at?: string | null;
    grace_expires_at?: string | null;
  };

  const [billingUser, setBillingUser] = useState<User | null>(null);
  const [billingEmail, setBillingEmail] = useState("");
  const [billingPassword, setBillingPassword] = useState("");
  const [billingBusy, setBillingBusy] = useState(false);
  const [billingError, setBillingError] = useState<string | null>(null);
  const billingViewTrackedRef = useRef(false);
  const [entitlements, setEntitlements] = useState<EntitlementsSnapshot | null>(() => {
    try {
      const cached = readCachedValue<EntitlementsSnapshot>(window.localStorage, ENTITLEMENTS_CACHE_KEY);
      return cached?.value ?? null;
    } catch {
      return null;
    }
  });
  const priorPlanRef = useRef<EntitlementsSnapshot["plan_type"] | null>(entitlements?.plan_type ?? null);
  const [entitlementsBusy, setEntitlementsBusy] = useState(false);
  const [devRestartBusy, setDevRestartBusy] = useState(false);
  const [devRestartError, setDevRestartError] = useState<string | null>(null);
  const [devRestartResults, setDevRestartResults] = useState<DevRestartProvidersResult[] | null>(null);

  const [mobileStatus, setMobileStatus] = useState<MobileAccessStatus | null>(null);
  const [mobileStatusBusy, setMobileStatusBusy] = useState(false);
  const [mobileStatusError, setMobileStatusError] = useState<string | null>(null);
  const [mobileEnableBusy, setMobileEnableBusy] = useState(false);
  const [mobileEnableError, setMobileEnableError] = useState<string | null>(null);
  const [mobileQr, setMobileQr] = useState<EnableMobileAccessResponse | null>(null);

  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const saveSeq = useRef(0);
  const telemetryHydrated = useRef(false);

  const [telemetryEnabled, setTelemetryEnabled] = useState(true);
  const [telemetryEndpoint, setTelemetryEndpoint] = useState("");
  const resourceGovernanceHydrated = useRef(false);
  const sandboxingHydrated = useRef(false);
  const executionHydrated = useRef(false);
  const savedExecutionPayloadKey = useRef<string | null>(null);
  const [resourceGovernanceEnabled, setResourceGovernanceEnabled] = useState(true);
  const [providerControlMode, setProviderControlMode] = useState<SandboxingSettings["provider_control_mode"]>("full");
  const [executionSettings, setExecutionSettings] = useState<ApiExecutionSettings>(() => defaultExecutionSettings());
  const [machineResolvedMemoryMb, setMachineResolvedMemoryMb] = useState<number | null>(null);
  const [machineIdleShutdownSeconds, setMachineIdleShutdownSeconds] = useState(String(DEFAULT_MACHINE_IDLE_SHUTDOWN_SECONDS));
  const [machineHostPressureSwapThresholdMb, setMachineHostPressureSwapThresholdMb] = useState(
    String(DEFAULT_MACHINE_HOST_PRESSURE_SWAP_THRESHOLD_MB),
  );
  const [resourceGovernanceMode, setResourceGovernanceMode] =
    useState<ResourceGovernanceSettings["mode"]>("auto");
  const [resourceCpuQuotaPct, setResourceCpuQuotaPct] = useState("");
  const [resourceMemoryHighGb, setResourceMemoryHighGb] = useState("");
  const [resourceMemoryMaxGb, setResourceMemoryMaxGb] = useState("");
  const [resourceEffective, setResourceEffective] = useState<ResourceGovernanceLimits | null>(null);
  const [resourceStatus, setResourceStatus] = useState<ResourceGovernanceStatus | null>(null);

  const {
    editorSettings,
    setEditorSettings,
    editorLoaded,
    editorError,
  } = useDesktopEditorSettingsController(isDesktopApp());

  const clientSettingsState = useSyncExternalStore(
    subscribeClientSettings,
    getClientSettingsState,
    getClientSettingsState,
  );
  const [clientSettingsSaving, setClientSettingsSaving] = useState(false);
  const [clientSettingsError, setClientSettingsError] = useState<string | null>(null);

  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);

  const [resourceSnapshot, setResourceSnapshot] = useState<ResourceUtilization | null>(null);
  const [resourceLoading, setResourceLoading] = useState(false);
  const [resourceError, setResourceError] = useState<string | null>(null);
  const resourcePollRef = useRef<number | null>(null);
  const [expandedProcessPids, setExpandedProcessPids] = useState<Record<number, boolean>>({});


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
    const { data } = supabase.auth.onAuthStateChange((_evt, session) => {
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
      const res = await supabase.functions.invoke("entitlements", { method: "GET" });
      if (res.error) throw res.error;
      const next = (res.data ?? null) as EntitlementsSnapshot | null;
      setEntitlements(next);
      if (next) writeCachedValue(window.localStorage, ENTITLEMENTS_CACHE_KEY, next);
      return next;
    } catch (e: unknown) {
      if (!opts?.silent) {
        setBillingError(errorMessage(e));
      }
      return null;
    } finally {
      if (!opts?.silent) {
        setEntitlementsBusy(false);
      }
    }
  }, [supabase]);

  const refreshMobileAccess = useCallback(async () => {
    setMobileStatusBusy(true);
    setMobileStatusError(null);
    try {
      const status = await getMobileAccessStatus();
      setMobileStatus(status);
    } catch (e: unknown) {
      setMobileStatusError(errorMessage(e));
    } finally {
      setMobileStatusBusy(false);
    }
  }, []);

  const getSupabaseToken = useCallback(async (): Promise<string> => {
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

  const handleEnableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await getSupabaseToken();
      const resp = await enableMobileAccess(token);
      setMobileQr(resp);
      setMobileStatus(resp.status);
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [getSupabaseToken]);

  const handleDisableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await getSupabaseToken();
      await disableMobileAccess(token);
      setMobileQr(null);
      await refreshMobileAccess();
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [getSupabaseToken, refreshMobileAccess]);

  const checkoutStatus = useMemo(
    () => new URLSearchParams(location.search).get("checkout"),
    [location.search],
  );
  const checkoutSessionId = useMemo(
    () => new URLSearchParams(location.search).get("session_id"),
    [location.search],
  );

  const isPaidPlan = useCallback((snapshot: EntitlementsSnapshot | null) => {
    return Boolean(snapshot?.plan_type && snapshot.plan_type !== "free_local");
  }, []);

  useEffect(() => {
    if (!supabase) return;
    refreshEntitlements().catch(() => {});
  }, [supabase, billingUser, refreshEntitlements]);

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
        const res = await supabase.functions.invoke("billing-sync", {
          body: checkoutSessionId ? { checkout_session_id: checkoutSessionId } : {},
        });
        if (res.error) throw res.error;
      } catch (e: unknown) {
        setBillingError(errorMessage(e));
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

    poll().catch(() => {});
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

  useEffect(() => {
    if (active !== "mobile_access") return;
    refreshMobileAccess().catch(() => {});
  }, [active, refreshMobileAccess]);

  useEffect(() => {
    const onHash = () => {
      const next = sectionFromHash(window.location.hash);
      if (next) setActive(next);
    };
    window.addEventListener("hashchange", onHash);
    return () => window.removeEventListener("hashchange", onHash);
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const s = await getSettings();
        if (cancelled) return;

        const t = s.telemetry ?? null;
        if (t) {
          setTelemetryEnabled(t.enabled);
          setTelemetryEndpoint(t.endpoint ?? "");
        } else {
          setTelemetryEnabled(true);
          setTelemetryEndpoint("");
        }

        const rg = s.resource_governance ?? null;
        if (rg) {
          setResourceGovernanceEnabled(rg.enabled);
        }

        const sb = s.sandboxing ?? null;
        if (sb?.provider_control_mode) {
          setProviderControlMode(sb.provider_control_mode);
        }
        const execution = normalizeExecutionSettings(s.execution ?? null);
        setMachineResolvedMemoryMb(s.execution?.container.machine.target_memory_mb ?? null);
        savedExecutionPayloadKey.current = executionSettingsStableKey(execution);
        setExecutionSettings(execution);
        setMachineIdleShutdownSeconds(String(execution.container.machine.idle_shutdown_seconds));
        setMachineHostPressureSwapThresholdMb(
          String(execution.container.machine.host_pressure_swap_threshold_mb),
        );

        if (rg) {
          setResourceGovernanceMode(rg.mode ?? "auto");
          setResourceCpuQuotaPct(rg.cpu_quota_pct ? String(rg.cpu_quota_pct) : "");
          setResourceMemoryHighGb(formatGiB(rg.memory_high_mb));
          setResourceMemoryMaxGb(formatGiB(rg.memory_max_mb));
          setResourceEffective(rg.effective ?? null);
          setResourceStatus(rg.status ?? null);
        }

        setLoaded(true);
      } catch (e: unknown) {
        if (cancelled) return;
        setLoadError(errorMessage(e));
        setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (clientSettingsState.loaded) return;
    loadClientSettings().catch((err) => {
      setClientSettingsError(err?.message ?? String(err));
    });
  }, [clientSettingsState.loaded]);

  const savePatch = async (patch: UpdateSettingsRequest) => {
    setSaveError(null);
    setSaving(true);
    const seq = ++saveSeq.current;
    try {
      const next = await updateSettings(patch);
      if (seq !== saveSeq.current) return;
      if (next.resource_governance) {
        setResourceGovernanceEnabled(next.resource_governance.enabled);
        setResourceGovernanceMode(next.resource_governance.mode ?? "auto");
        setResourceCpuQuotaPct(
          next.resource_governance.cpu_quota_pct ? String(next.resource_governance.cpu_quota_pct) : "",
        );
        setResourceMemoryHighGb(formatGiB(next.resource_governance.memory_high_mb));
        setResourceMemoryMaxGb(formatGiB(next.resource_governance.memory_max_mb));
        setResourceEffective(next.resource_governance.effective ?? null);
        setResourceStatus(next.resource_governance.status ?? null);
      }
      if (next.sandboxing?.provider_control_mode) {
        setProviderControlMode(next.sandboxing.provider_control_mode);
      }
      const execution = normalizeExecutionSettings(next.execution ?? null);
      setMachineResolvedMemoryMb(next.execution?.container.machine.target_memory_mb ?? null);
      savedExecutionPayloadKey.current = executionSettingsStableKey(execution);
      setExecutionSettings(execution);
      setMachineIdleShutdownSeconds(String(execution.container.machine.idle_shutdown_seconds));
      setMachineHostPressureSwapThresholdMb(
        String(execution.container.machine.host_pressure_swap_threshold_mb),
      );
    } catch (e: unknown) {
      if (seq !== saveSeq.current) return;
      setSaveError(errorMessage(e));
    } finally {
      if (seq === saveSeq.current) setSaving(false);
    }
  };

  const sandboxingPayload = useMemo((): SandboxingSettings => {
    return {
      provider_control_mode: providerControlMode,
    };
  }, [providerControlMode]);

  const executionPayload = useMemo((): ApiExecutionSettings => {
    const idleShutdownSeconds =
      parseUnsignedInteger(machineIdleShutdownSeconds) ?? DEFAULT_MACHINE_IDLE_SHUTDOWN_SECONDS;
    const hostPressureSwapThresholdMb =
      parseUnsignedInteger(machineHostPressureSwapThresholdMb)
      ?? DEFAULT_MACHINE_HOST_PRESSURE_SWAP_THRESHOLD_MB;

    return {
      ...executionSettings,
      container: {
        ...executionSettings.container,
        machine: {
          memory_profile: executionSettings.container.machine.memory_profile,
          custom_memory_mb: executionSettings.container.machine.custom_memory_mb,
          idle_shutdown_seconds: idleShutdownSeconds,
          host_pressure_swap_threshold_mb: hostPressureSwapThresholdMb,
        },
      },
    };
  }, [
    executionSettings,
    machineHostPressureSwapThresholdMb,
    machineIdleShutdownSeconds,
  ]);

  const executionPayloadKey = useMemo(
    () => executionSettingsStableKey(executionPayload),
    [executionPayload],
  );

  const resourceGovernancePayload = useMemo((): ResourceGovernanceSettings => {
    const cpuQuota = Number(resourceCpuQuotaPct);
    const cpuQuotaPct = Number.isFinite(cpuQuota) && cpuQuota > 0 ? Math.round(cpuQuota) : null;
    const memoryHighMb = parseGiB(resourceMemoryHighGb);
    const memoryMaxMb = parseGiB(resourceMemoryMaxGb);
    return {
      enabled: resourceGovernanceEnabled,
      mode: resourceGovernanceMode,
      cpu_quota_pct: resourceGovernanceMode === "custom" ? cpuQuotaPct : null,
      memory_high_mb: resourceGovernanceMode === "custom" ? memoryHighMb : null,
      memory_max_mb: resourceGovernanceMode === "custom" ? memoryMaxMb : null,
    };
  }, [
    resourceCpuQuotaPct,
    resourceGovernanceEnabled,
    resourceGovernanceMode,
    resourceMemoryHighGb,
    resourceMemoryMaxGb,
  ]);

  const resourceGovernanceCanSave = useMemo(() => {
    if (!resourceGovernanceEnabled) return true;
    if (resourceGovernanceMode !== "custom") return true;
    const high = parseGiB(resourceMemoryHighGb);
    const max = parseGiB(resourceMemoryMaxGb);
    if (high && max && high > max) return false;
    return true;
  }, [resourceGovernanceEnabled, resourceGovernanceMode, resourceMemoryHighGb, resourceMemoryMaxGb]);

  const sandboxMachineCanSave = useMemo(() => {
    return canSaveSandboxMachineSettings({
      machineHostPressureSwapThresholdMb,
      machineIdleShutdownSeconds,
    });
  }, [
    machineHostPressureSwapThresholdMb,
    machineIdleShutdownSeconds,
  ]);

  useEffect(() => {
    if (!loaded) return;
    if (!telemetryHydrated.current) {
      telemetryHydrated.current = true;
      return;
    }
    const t = window.setTimeout(() => {
      const next: TelemetrySettings = {
        enabled: telemetryEnabled,
        endpoint: telemetryEndpoint.trim() || telemetryEndpoint,
      };
      savePatch({ telemetry: next });
    }, 250);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [telemetryEnabled, telemetryEndpoint, loaded]);

  useEffect(() => {
    if (!loaded) return;
    if (!sandboxingHydrated.current) {
      sandboxingHydrated.current = true;
      return;
    }
    const t = window.setTimeout(() => {
      savePatch({ sandboxing: sandboxingPayload });
    }, 450);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, sandboxingPayload]);

  useEffect(() => {
    if (!loaded) return;
    if (!executionHydrated.current) {
      executionHydrated.current = true;
      return;
    }
    if (!sandboxMachineCanSave) return;
    if (savedExecutionPayloadKey.current === executionPayloadKey) return;
    const t = window.setTimeout(() => {
      savePatch({ execution: executionPayload });
    }, 450);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [executionPayload, executionPayloadKey, loaded, sandboxMachineCanSave]);

  useEffect(() => {
    if (!loaded) return;
    if (!resourceGovernanceHydrated.current) {
      resourceGovernanceHydrated.current = true;
      return;
    }
    if (!resourceGovernanceCanSave) return;
    const t = window.setTimeout(() => {
      savePatch({ resource_governance: resourceGovernancePayload });
    }, 450);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, resourceGovernancePayload, resourceGovernanceCanSave]);

  useEffect(() => {
    listWorkspaces()
      .then((ws) => {
        setWorkspaces(ws);
        if (!workspaceId && ws.length > 0) setWorkspaceId(idToString((ws[0] as { id?: string | null }).id));
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!workspaces.length) return;
    if (!workspaceId) {
      setWorkspaceId(idToString((workspaces[0] as { id?: string | null }).id));
      return;
    }
    const found = workspaces.some((ws) => idToString((ws as { id?: string | null }).id) === workspaceId);
    if (!found) {
      setWorkspaceId(idToString((workspaces[0] as { id?: string | null }).id));
    }
  }, [workspaces, workspaceId]);

  useEffect(() => {
    if (active !== "resource_utilization") return;
    if (!workspaceId) return;
    let cancelled = false;

    const poll = async () => {
      if (cancelled) return;
      setResourceLoading(true);
      setResourceError(null);
      try {
        const snapshot = await getResourceUtilization(workspaceId);
        if (!cancelled) setResourceSnapshot(snapshot);
      } catch (e: unknown) {
        if (!cancelled) setResourceError(errorMessage(e));
      } finally {
        if (!cancelled) setResourceLoading(false);
      }
      if (!cancelled) {
        resourcePollRef.current = window.setTimeout(poll, 3000);
      }
    };

    poll();
    return () => {
      cancelled = true;
      if (resourcePollRef.current) {
        window.clearTimeout(resourcePollRef.current);
        resourcePollRef.current = null;
      }
    };
  }, [active, workspaceId]);

  const sidebarSections = useMemo<SettingsSectionMeta[]>(() => {
    const q = query.trim().toLowerCase();
    const all = SECTIONS.filter(
      (section) => !section.navHidden && (devToolsEnabled || section.id !== "dev_tools"),
    );
    if (!q) return all;
    return all.filter((s) => s.label.toLowerCase().includes(q));
  }, [query, devToolsEnabled]);

  const workspaceFromQuery = useMemo(() => {
    const ws = new URLSearchParams(location.search).get("ws");
    if (!ws) return null;
    const trimmed = ws.trim();
    return trimmed ? trimmed : null;
  }, [location.search]);
  const handleSectionChange = useCallback(
    (nextSection: SectionId) => {
      if (nextSection === active) return;
      setActive(nextSection);
      window.location.hash = nextSection;
    },
    [active, setActive],
  );

  const handleDevRestart = useCallback(
    async (mode: "drain" | "immediate") => {
      if (!devToolsEnabled || devRestartBusy) return;
      if (mode === "immediate") {
        const confirmed = window.confirm("Immediate restart will interrupt running provider work. Continue?");
        if (!confirmed) return;
      }
      setDevRestartBusy(true);
      setDevRestartError(null);
      setDevRestartResults(null);
      try {
        const response = await devRestartProviders(mode);
        setDevRestartResults(response.results);
      } catch (err: unknown) {
        setDevRestartError(errorMessage(err));
      } finally {
        setDevRestartBusy(false);
      }
    },
    [devToolsEnabled, devRestartBusy, devRestartProviders],
  );

  const backLink = useMemo(() => {
    const ws = new URLSearchParams(location.search).get("ws");
    if (ws && ws.trim()) {
      const id = ws.trim();
      return { to: `/workspaces/${encodeURIComponent(id)}`, label: "← Back to Workspace" };
    }
    return { to: "/", label: "← Back to Home" };
  }, [location.search]);

  useEffect(() => {
    if (!workspaceFromQuery) return;
    if (workspaceId !== workspaceFromQuery) {
      setWorkspaceId(workspaceFromQuery);
    }
  }, [workspaceFromQuery, workspaceId]);

  const vscodeRemoteTargets: DesktopEditorSettings["target"][] = [
    "vscode",
    "vscode_insiders",
    "cursor",
    "windsurf",
    "antigravity",
  ];
  const showRemoteAuthority = vscodeRemoteTargets.includes(editorSettings.target);
  const desktopTurnNotifications = clientSettingsState.settings.desktopNotifications.turnCompleted;

  const handleToggleTurnNotifications = useCallback(
    async (next: boolean) => {
      if (clientSettingsSaving) return;
      setClientSettingsSaving(true);
      setClientSettingsError(null);
      try {
        if (next && isDesktopApp()) {
          const granted = await ensureDesktopNotificationPermission();
          if (!granted) {
            setClientSettingsError("Notification permission denied.");
            return;
          }
        }
        await updateClientSettings({ desktopNotifications: { turnCompleted: next } });
      } catch (err: unknown) {
        setClientSettingsError(errorMessage(err));
      } finally {
        setClientSettingsSaving(false);
      }
    },
    [clientSettingsSaving],
  );
  const onThemeChange = useCallback((next: ThemeMode) => {
    setTheme(next);
    applyTheme(next);
    setStoredTheme(next);
  }, []);

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

  const toggleExpandedProcess = useCallback((pid: number) => {
    setExpandedProcessPids((prev) => ({ ...prev, [pid]: !prev[pid] }));
  }, []);

  const headerLabel = SECTIONS.find((section) => section.id === active)?.label ?? "Settings";
  const supabaseConfigured = Boolean(supabase);
  const plan = entitlements?.plan_type ?? "free_local";
  const proEnabled = entitlements?.features?.remote_mobile_access === "enabled";

  return (
    <SettingsShell
      backLink={backLink}
      query={query}
      onQueryChange={setQuery}
      sidebarSections={sidebarSections}
      active={active}
      onSectionChange={handleSectionChange}
      headerLabel={headerLabel}
      saveError={saveError}
    >
      <SettingsContentRouter
        active={active}
        loaded={loaded}
        loadError={loadError}
        theme={theme}
        onThemeChange={onThemeChange}
        editorSettings={editorSettings}
        setEditorSettings={setEditorSettings}
        editorLoaded={editorLoaded}
        editorError={editorError}
        clientSettingsError={clientSettingsError}
        showRemoteAuthority={showRemoteAuthority}
        isDesktopApp={isDesktopApp}
        desktopTurnNotifications={desktopTurnNotifications}
        clientSettingsState={clientSettingsState}
        clientSettingsSaving={clientSettingsSaving}
        onToggleTurnNotifications={handleToggleTurnNotifications}
        telemetryEnabled={telemetryEnabled}
        setTelemetryEnabled={setTelemetryEnabled}
        workspaceId={workspaceId}
        themeVariant={themeVariant}
        resourceGovernance={{
          enabled: resourceGovernanceEnabled,
          setEnabled: setResourceGovernanceEnabled,
          mode: resourceGovernanceMode,
          setMode: setResourceGovernanceMode,
          cpuQuotaPct: resourceCpuQuotaPct,
          setCpuQuotaPct: setResourceCpuQuotaPct,
          memoryHighGb: resourceMemoryHighGb,
          setMemoryHighGb: setResourceMemoryHighGb,
          memoryMaxGb: resourceMemoryMaxGb,
          setMemoryMaxGb: setResourceMemoryMaxGb,
          effective: resourceEffective,
          status: resourceStatus,
          canSave: resourceGovernanceCanSave,
          payload: resourceGovernancePayload,
          onApplyNow: (payload) => savePatch({ resource_governance: payload }),
        }}
        saving={saving}
        supabaseConfigured={supabaseConfigured}
        billing={{
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
        }}
        mobileAccess={{
          billingUser,
          entitlementsBusy,
          proEnabled,
          mobileStatus,
          mobileStatusBusy,
          mobileStatusError,
          mobileEnableBusy,
          mobileEnableError,
          mobileQr,
          qrFgColor,
          onEnable: handleEnableMobile,
          onDisable: handleDisableMobile,
        }}
        resourceUtilization={{
          workspaces,
          snapshot: resourceSnapshot,
          loading: resourceLoading,
          error: resourceError,
          expandedProcessPids,
          onToggleExpanded: toggleExpandedProcess,
        }}
        providerControlMode={providerControlMode}
        setProviderControlMode={setProviderControlMode}
        machineResolvedMemoryMb={machineResolvedMemoryMb}
        machineIdleShutdownSeconds={machineIdleShutdownSeconds}
        setMachineIdleShutdownSeconds={setMachineIdleShutdownSeconds}
        machineHostPressureSwapThresholdMb={machineHostPressureSwapThresholdMb}
        setMachineHostPressureSwapThresholdMb={setMachineHostPressureSwapThresholdMb}
        sandboxMachineCanSave={sandboxMachineCanSave}
        devTools={{
          enabled: devToolsEnabled,
          restartBusy: devRestartBusy,
          restartError: devRestartError,
          restartResults: devRestartResults,
          onRestart: handleDevRestart,
        }}
      />
    </SettingsShell>
  );
}
