import { Fragment, useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import type { User } from "@supabase/supabase-js";
import { QRCodeSVG } from "qrcode.react";
import {
  DevRestartProvidersResult,
  MobileAccessStatus,
  EnableMobileAccessResponse,
  ResourceGovernanceLimits,
  ResourceGovernanceSettings,
  ResourceGovernanceStatus,
  ResourceUtilization,
  SandboxingSettings,
  Settings,
  TelemetrySettings,
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
  desktopGetEditorSettings,
  desktopUpdateEditorSettings,
  isDesktopApp,
} from "../utils/desktop";
import { ensureDesktopNotificationPermission } from "../utils/desktopNotifications";
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
import { Card, Metric, Row, Toggle } from "./SettingsPage.components";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "../components/ui/select";
import { GeneralSection } from "./settings/sections/GeneralSection";
import { GeneralSettingsSection } from "./settings/sections/GeneralSettingsSection";
import { NotificationsSettingsSection } from "./settings/sections/NotificationsSettingsSection";
import { AnalyticsSettingsSection } from "./settings/sections/AnalyticsSettingsSection";
import { DevToolsSection } from "./settings/sections/DevToolsSection";
import { WorktreeBootstrapSection } from "./settings/sections/WorktreeBootstrapSection";
import { AgentSystemPromptSection } from "./settings/sections/AgentSystemPromptSection";
import { ContainerNetworkSection } from "./settings/sections/ContainerNetworkSection";
import { MergeQueueSection } from "./settings/sections/MergeQueueSection";
import { DictationSection } from "./settings/sections/DictationSection";
import { TitleGenerationSection } from "./settings/sections/TitleGenerationSection";
import { WorkspaceAttachmentsSection } from "./settings/sections/WorkspaceAttachmentsSection";
import { HarnessAuthenticationSection } from "./settings/sections/HarnessAuthenticationSection";
import { CodexAccountsSection } from "./settings/sections/CodexAccountsSection";
import { useSettingsActions } from "./settings/useSettingsActions";
import { useSettingsState } from "./settings/useSettingsState";
import type { SectionId } from "./SettingsPage.types";
import { runBillingCheckoutFlow } from "./settings/billingCheckoutFlow";
import { shouldTrackEntitlementActivated } from "./settings/entitlementAnalytics";
import {
  formatAge,
  formatBytes,
  formatGiB,
  formatPct,
  isLinuxPlatform,
  parseGiB,
  sectionFromHash,
  truncateText,
} from "./SettingsPage.utils";

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
  const [resourceGovernanceEnabled, setResourceGovernanceEnabled] = useState(true);
  const [providerControlMode, setProviderControlMode] = useState<SandboxingSettings["provider_control_mode"]>("full");
  const [resourceGovernanceMode, setResourceGovernanceMode] =
    useState<ResourceGovernanceSettings["mode"]>("auto");
  const [resourceCpuQuotaPct, setResourceCpuQuotaPct] = useState("");
  const [resourceMemoryHighGb, setResourceMemoryHighGb] = useState("");
  const [resourceMemoryMaxGb, setResourceMemoryMaxGb] = useState("");
  const [resourceEffective, setResourceEffective] = useState<ResourceGovernanceLimits | null>(null);
  const [resourceStatus, setResourceStatus] = useState<ResourceGovernanceStatus | null>(null);

  const [editorSettings, setEditorSettings] = useState<DesktopEditorSettings>({
    target: "system",
    custom_command: "",
    remote_authority: "",
  });
  const [editorLoaded, setEditorLoaded] = useState(false);
  const [editorSaving, setEditorSaving] = useState(false);
  const [editorError, setEditorError] = useState<string | null>(null);
  const editorHydrated = useRef(false);

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
      const next = (res.data ?? null) as any;
      setEntitlements(next);
      if (next) writeCachedValue(window.localStorage, ENTITLEMENTS_CACHE_KEY, next);
      return next;
    } catch (e: any) {
      if (!opts?.silent) {
        setBillingError(e?.message ?? String(e));
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
    } catch (e: any) {
      setMobileStatusError(e?.message ?? String(e));
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
    } catch (e: any) {
      setMobileEnableError(e?.message ?? String(e));
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
    } catch (e: any) {
      setMobileEnableError(e?.message ?? String(e));
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
      } catch (e: any) {
        setBillingError(e?.message ?? String(e));
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

        if (rg) {
          setResourceGovernanceMode(rg.mode ?? "auto");
          setResourceCpuQuotaPct(rg.cpu_quota_pct ? String(rg.cpu_quota_pct) : "");
          setResourceMemoryHighGb(formatGiB(rg.memory_high_mb));
          setResourceMemoryMaxGb(formatGiB(rg.memory_max_mb));
          setResourceEffective(rg.effective ?? null);
          setResourceStatus(rg.status ?? null);
        }

        setLoaded(true);
      } catch (e: any) {
        if (cancelled) return;
        setLoadError(e?.message ?? String(e));
        setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!isDesktopApp()) return;
    let cancelled = false;
    desktopGetEditorSettings()
      .then((settings) => {
        if (cancelled) return;
        setEditorSettings(settings);
        setEditorLoaded(true);
      })
      .catch((e: any) => {
        if (cancelled) return;
        setEditorError(e?.message ?? String(e));
        setEditorLoaded(true);
      });
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

  const savePatch = async (patch: Partial<Settings>) => {
    setSaveError(null);
    setSaving(true);
    const seq = ++saveSeq.current;
    try {
      const next = await updateSettings(patch as Settings);
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
    } catch (e: any) {
      if (seq !== saveSeq.current) return;
      setSaveError(e?.message ?? String(e));
    } finally {
      if (seq === saveSeq.current) setSaving(false);
    }
  };

  const sandboxingPayload = useMemo((): SandboxingSettings => {
    return {
      provider_control_mode: providerControlMode,
    };
  }, [providerControlMode]);

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
    if (!isDesktopApp()) return;
    if (!editorLoaded) return;
    if (!editorHydrated.current) {
      editorHydrated.current = true;
      return;
    }
    const t = window.setTimeout(() => {
      setEditorSaving(true);
      setEditorError(null);
      const next: DesktopEditorSettings = {
        target: editorSettings.target,
        custom_command:
          editorSettings.target === "custom"
            ? editorSettings.custom_command?.trim() || null
            : null,
        remote_authority: editorSettings.remote_authority?.trim() || null,
      };
      desktopUpdateEditorSettings(next)
        .then((next) => setEditorSettings(next))
        .catch((e: any) => setEditorError(e?.message ?? String(e)))
        .finally(() => setEditorSaving(false));
    }, 350);
    return () => window.clearTimeout(t);
  }, [editorSettings, editorLoaded]);

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
      } catch (e: any) {
        if (!cancelled) setResourceError(e?.message ?? String(e));
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

  const sidebarSections = useMemo(() => {
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
      } catch (err: any) {
        setDevRestartError(err?.message ?? String(err));
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

  const anySaving =
    saving
    || editorSaving
    || clientSettingsSaving;

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
      } catch (err: any) {
        setClientSettingsError(err?.message ?? String(err));
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

  const renderMain = () => {
    if (!loaded) return <div className="settings-empty">Loading…</div>;
    if (loadError) return <div className="settings-empty settings-empty-error">{loadError}</div>;

    if (active === "general") {
      return (
        <GeneralSettingsSection
          theme={theme}
          onThemeChange={onThemeChange}
          editorSettings={editorSettings}
          setEditorSettings={setEditorSettings}
          editorLoaded={editorLoaded}
          editorError={editorError}
          clientSettingsError={clientSettingsError}
          showRemoteAuthority={showRemoteAuthority}
          isDesktopApp={isDesktopApp}
        />
      );
    }

    if (active === "notifications") {
      return (
        <NotificationsSettingsSection
          isDesktopApp={isDesktopApp}
          desktopTurnNotifications={desktopTurnNotifications}
          clientSettingsState={clientSettingsState}
          clientSettingsSaving={clientSettingsSaving}
          clientSettingsError={clientSettingsError}
          onToggleTurnNotifications={handleToggleTurnNotifications}
        />
      );
    }

    if (active === "analytics") {
      return (
        <AnalyticsSettingsSection
          telemetryEnabled={telemetryEnabled}
          loaded={loaded}
          setTelemetryEnabled={setTelemetryEnabled}
        />
      );
    }

    if (active === "worktree_bootstrap") {
      return (
        <WorktreeBootstrapSection
          workspaceId={workspaceId}
          active={active === "worktree_bootstrap"}
        />
      );
    }

    if (active === "agent_system_prompt") {
      return (
        <AgentSystemPromptSection
          workspaceId={workspaceId}
          active={active === "agent_system_prompt"}
          themeVariant={themeVariant}
        />
      );
    }

    if (active === "workspace_attachments") {
      return (
        <WorkspaceAttachmentsSection
          workspaceId={workspaceId}
          active={active === "workspace_attachments"}
        />
      );
    }

    if (active === "container_network") {
      return (
        <ContainerNetworkSection
          workspaceId={workspaceId}
          active={active === "container_network"}
          themeVariant={themeVariant}
        />
      );
    }

    if (active === "merge_queue") {
      return (
        <MergeQueueSection
          workspaceId={workspaceId}
          active={active === "merge_queue"}
        />
      );
    }

    if (active === "resource_governance") {
      const effectiveCpu = resourceEffective?.cpu_quota_pct ?? null;
      const effectiveHigh = resourceEffective?.memory_high_mb ?? null;
      const effectiveMax = resourceEffective?.memory_max_mb ?? null;
      const statusState = resourceStatus?.state ?? (resourceGovernanceEnabled ? "pending" : "disabled");
      const statusLabel =
        statusState === "disabled"
          ? "Disabled"
          : statusState === "applied"
            ? "Applied"
            : statusState === "unsupported"
              ? "Unsupported"
              : statusState === "error"
                ? "Error"
                : "Pending";
      const statusMessage = resourceStatus?.message ?? null;
      const showApplyNow = statusState === "pending" && Boolean(resourceStatus?.can_apply_now);
      const showRestart = Boolean(resourceStatus?.requires_restart);

      return (
        <>
          <Card title="Resource Governance">
            <Row
              title="Enable resource limits"
              description="Keep the host responsive by throttling agent workloads."
              control={
                <Toggle
                  checked={resourceGovernanceEnabled}
                  disabled={!loaded}
                  onChange={setResourceGovernanceEnabled}
                  ariaLabel="Enable resource limits"
                />
              }
            />
            <Row
              title="Mode"
              description="Auto picks safe limits for this machine."
              control={
                <Select
                  value={resourceGovernanceMode}
                  onValueChange={(value) =>
                    setResourceGovernanceMode(value as ResourceGovernanceSettings["mode"])
                  }
                  disabled={!resourceGovernanceEnabled}
                >
                  <SelectTrigger className="tw-min-w-[10rem]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto">Auto (recommended)</SelectItem>
                    <SelectItem value="custom">Custom</SelectItem>
                  </SelectContent>
                </Select>
              }
            />
            {resourceGovernanceMode === "custom" ? (
              <>
                <Row
                  title="CPU quota (%)"
                  description="100% = 1 core. Leave empty to use auto."
                  control={
                    <input
                      className="settings-control"
                      type="number"
                      min={50}
                      step={10}
                      value={resourceCpuQuotaPct}
                      onChange={(e) => setResourceCpuQuotaPct(e.target.value)}
                      disabled={!resourceGovernanceEnabled}
                      placeholder="300"
                    />
                  }
                />
                <Row
                  title="Memory high (GiB)"
                  description="Soft limit for reclaim pressure."
                  control={
                    <input
                      className="settings-control"
                      type="number"
                      min={0}
                      step={0.5}
                      value={resourceMemoryHighGb}
                      onChange={(e) => setResourceMemoryHighGb(e.target.value)}
                      disabled={!resourceGovernanceEnabled}
                      placeholder="48"
                    />
                  }
                />
                <Row
                  title="Memory max (GiB)"
                  description="Hard limit; processes are killed when exceeded."
                  control={
                    <input
                      className="settings-control"
                      type="number"
                      min={0}
                      step={0.5}
                      value={resourceMemoryMaxGb}
                      onChange={(e) => setResourceMemoryMaxGb(e.target.value)}
                      disabled={!resourceGovernanceEnabled}
                      placeholder="54"
                    />
                  }
                />
              </>
            ) : null}
          </Card>

          <Card title="Effective limits">
            <Row
              title="CPU quota"
              description="Applied to the daemon and its child processes."
              control={
                <span className="settings-pill wb-mono">
                  {effectiveCpu ? `${effectiveCpu}%` : "—"}
                </span>
              }
            />
            <Row
              title="Memory high / max"
              description="High is the soft threshold; max is the hard cap."
              control={
                <span className="settings-pill wb-mono">
                  {effectiveHigh ? `${formatGiB(effectiveHigh)} GiB` : "—"} /{" "}
                  {effectiveMax ? `${formatGiB(effectiveMax)} GiB` : "—"}
                </span>
              }
            />
            <Row
              title="Apply status"
              description={statusMessage ?? "Apply changes to update live limits."}
              control={
                <div className="row" style={{ gap: 8 }}>
                  <span className="settings-pill">{statusLabel}</span>
                  {showRestart ? <span className="settings-pill settings-pill-warn">Restart required</span> : null}
                  {showApplyNow ? (
                    <button
                      type="button"
                      className="settings-btn settings-btn-secondary"
                      onClick={() => savePatch({ resource_governance: resourceGovernancePayload })}
                      disabled={saving || !resourceGovernanceCanSave}
                    >
                      Apply now
                    </button>
                  ) : null}
                </div>
              }
            />
          </Card>

          {!resourceGovernanceCanSave && resourceGovernanceMode === "custom" ? (
            <div className="settings-banner settings-banner-error">Memory high must be less than or equal to memory max.</div>
          ) : null}
        </>
      );
    }

    if (active === "mobile_access") {
      if (!supabase) {
        return (
          <div className="settings-empty">
            Mobile access requires Supabase config. Set <code>VITE_SUPABASE_URL</code> and <code>VITE_SUPABASE_ANON_KEY</code>.
          </div>
        );
      }

      const proEnabled = entitlements?.features?.remote_mobile_access === "enabled";
      const status = mobileStatus;
      const statusLabel = mobileStatusBusy ? "Loading" : status?.enabled ? "Enabled" : "Disabled";
      const tunnelState = status?.tunnel_state ?? "idle";

      return (
        <>
          <Card title="Remote Mobile Access">
            <Row
              title="Entitlement"
              description={entitlementsBusy ? "Loading…" : proEnabled ? "Pro enabled" : "Pro required"}
              control={<div className="settings-pill">{proEnabled ? "Enabled" : "Disabled"}</div>}
            />
            <Row
              title="Status"
              description="Mobile access tunnel status on this daemon."
              control={<div className="settings-pill">{statusLabel}</div>}
            />
            <Row
              title="Tunnel state"
              description={status?.last_error ?? "Router tunnel lifecycle state."}
              control={<div className="settings-pill">{tunnelState}</div>}
            />
            {status?.public_base_url ? (
              <Row title="Public URL" control={<span className="settings-pill wb-mono">{status.public_base_url}</span>} />
            ) : null}
            {status?.tunnel_id ? (
              <Row title="Tunnel ID" control={<span className="settings-pill wb-mono">{status.tunnel_id}</span>} />
            ) : null}
            <Row
              title="Actions"
              description={!billingUser ? "Sign in to enable or revoke mobile access." : "Manage remote tunnel access."}
              control={
                <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
                  <button
                    type="button"
                    className="settings-btn settings-btn-secondary"
                    onClick={handleEnableMobile}
                    disabled={!billingUser || !proEnabled || mobileEnableBusy}
                  >
                    {status?.enabled ? "Show QR" : "Enable"}
                  </button>
                  {status?.enabled ? (
                    <button
                      type="button"
                      className="settings-btn"
                      onClick={handleDisableMobile}
                      disabled={!billingUser || mobileEnableBusy}
                    >
                      Disable
                    </button>
                  ) : null}
                </div>
              }
            />
            {!proEnabled ? (
              <Row
                title="Upgrade"
                description="Remote mobile access is a Pro feature."
                control={<Link to="#billing">Go to billing</Link>}
              />
            ) : null}
          </Card>

          {mobileStatusBusy ? <div className="settings-banner">Loading mobile access status…</div> : null}
          {mobileStatusError ? <div className="settings-banner settings-banner-error">{mobileStatusError}</div> : null}
          {mobileEnableError ? <div className="settings-banner settings-banner-error">{mobileEnableError}</div> : null}

          {mobileQr ? (
            <Card title="Pair a mobile device">
              <div className="settings-card-block" style={{ display: "flex", gap: 24, alignItems: "center", flexWrap: "wrap" }}>
                <QRCodeSVG
                  value={JSON.stringify(mobileQr.qr_payload)}
                  size={220}
                  bgColor="transparent"
                  fgColor={qrFgColor}
                />
                <div style={{ minWidth: 240 }}>
                  <div className="settings-row-title" style={{ marginBottom: 6 }}>Scan with ctx mobile</div>
                  <div className="settings-row-desc">
                    This QR code pairs a device using end-to-end encryption. It expires at {new Date(mobileQr.pairing_expires_at).toLocaleString()}.
                  </div>
                </div>
              </div>
            </Card>
          ) : null}
        </>
      );
    }

    if (active === "resource_utilization") {
      if (!workspaceId) {
        return <div className="settings-empty">No workspace selected.</div>;
      }

      const snapshot = resourceSnapshot;
      const system = snapshot?.system;
      const disk = snapshot?.workspace?.disk;
      const workspaceName =
        workspaces.find((ws) => idToString((ws as any).id) === workspaceId)?.name ?? "Workspace";

      const memoryPct =
        system && system.memory_total_bytes > 0
          ? (system.memory_used_bytes / system.memory_total_bytes) * 100
          : null;
      const swapPct =
        system && system.swap_total_bytes > 0
          ? (system.swap_used_bytes / system.swap_total_bytes) * 100
          : null;
      const diskPct =
        disk && disk.total_bytes > 0
          ? ((disk.total_bytes - disk.available_bytes) / disk.total_bytes) * 100
          : null;

      const overviewUpdated =
        snapshot && Number.isFinite(snapshot.cache_age_ms)
          ? `Updated ${formatAge(snapshot.cache_age_ms)} ago`
          : "Awaiting resource data…";
      const diskUpdated =
        snapshot && Number.isFinite(snapshot.workspace.size_cache_age_ms)
          ? `Disk scan ${formatAge(snapshot.workspace.size_cache_age_ms)} ago`
          : "Disk scan pending…";

      const processRows = (() => {
        if (!snapshot?.processes) return [];
        const rows = [];
        if (snapshot.processes.daemon) {
          rows.push(snapshot.processes.daemon);
        }
        const providers = [...snapshot.processes.providers].sort((a, b) => a.label.localeCompare(b.label));
        rows.push(...providers);
        return rows;
      })();

      const worktreeRows = snapshot?.workspace.worktrees ?? [];

      const toggleExpanded = (pid: number) => {
        setExpandedProcessPids((prev) => ({ ...prev, [pid]: !prev[pid] }));
      };

      return (
        <>
          <Card title="Overview">
            <div className="settings-card-block">
              <div className="settings-metrics-grid">
                <Metric
                  label="CPU"
                  value={formatPct(system?.cpu_pct)}
                  sublabel="System CPU usage"
                  pct={system?.cpu_pct ?? null}
                />
                <Metric
                  label="Memory"
                  value={
                    system
                      ? `${formatBytes(system.memory_used_bytes)} / ${formatBytes(system.memory_total_bytes)}`
                      : "—"
                  }
                  sublabel="Physical memory"
                  pct={memoryPct}
                />
                <Metric
                  label="Swap"
                  value={
                    system
                      ? `${formatBytes(system.swap_used_bytes)} / ${formatBytes(system.swap_total_bytes)}`
                      : "—"
                  }
                  sublabel="Swap usage"
                  pct={swapPct}
                />
                <Metric
                  label="Disk"
                  value={
                    disk
                      ? `${formatBytes(disk.available_bytes)} free / ${formatBytes(disk.total_bytes)}`
                      : "—"
                  }
                  sublabel={disk ? `${disk.mount_point} · ${disk.file_system}` : "Workspace volume"}
                  pct={diskPct}
                />
              </div>
              <div className="settings-meta-line">{resourceLoading ? "Refreshing…" : overviewUpdated}</div>
            </div>
          </Card>

          <Card title="Processes">
            <div className="settings-card-block">
              {processRows.length === 0 ? (
                <div className="settings-empty">No process metrics yet.</div>
              ) : (
                <div className="settings-table settings-table-processes">
                  <div className="settings-table-head">
                    <div>Process</div>
                    <div>CPU</div>
                    <div>Memory</div>
                    <div>PID</div>
                  </div>
                  {processRows.map((p) => {
                    const expanded = !!expandedProcessPids[p.pid];
                    const hasChildren = (p.children?.length ?? 0) > 0 || p.child_count > 0;
                    const childCountLabel = `${p.child_count} child process${p.child_count === 1 ? "" : "es"}`;
                    return (
                      <Fragment key={`${p.label}-${p.pid}`}>
                        <div className="settings-table-row">
                          <div className="settings-process-cell">
                            <button
                              type="button"
                              className="settings-process-expand"
                              onClick={() => toggleExpanded(p.pid)}
                              disabled={!hasChildren}
                              aria-label={expanded ? "Collapse process children" : "Expand process children"}
                              aria-expanded={expanded}
                            >
                              {hasChildren ? (expanded ? "▾" : "▸") : "·"}
                            </button>
                            <div>
                              <div className="settings-table-title">{p.label}</div>
                              <div className="settings-table-sub">
                                {childCountLabel}
                                {p.children_truncated ? " (truncated)" : ""}
                              </div>
                            </div>
                          </div>
                          <div>{formatPct(p.cpu_pct)}</div>
                          <div>{formatBytes(p.memory_bytes)}</div>
                          <div className="settings-table-mono">{p.pid}</div>
                        </div>
                        {expanded ? (
                          <div className="settings-process-children">
                            {p.child_count === 0 ? (
                              <div className="settings-empty settings-empty-compact">No child processes.</div>
                            ) : (
                              <>
                                <div className="settings-process-children-meta">
                                  {p.children_truncated
                                    ? `Showing ${p.children.length} of ${p.child_count} descendants (sorted by memory)`
                                    : `${p.children.length} descendants`}
                                </div>
                                <div className="settings-table settings-table-process-children">
                                  <div className="settings-table-head">
                                    <div>Child process</div>
                                    <div>CPU</div>
                                    <div>Memory</div>
                                    <div>PID</div>
                                  </div>
                                  {p.children.map((c) => (
                                    <div key={`${p.pid}-${c.pid}`} className="settings-table-row">
                                      <div>
                                        <div className="settings-table-title">{c.name}</div>
                                        <div className="settings-table-sub">
                                          {c.cmdline
                                            ? `ppid ${c.parent_pid ?? "—"} · ${truncateText(c.cmdline, 120)}`
                                            : `ppid ${c.parent_pid ?? "—"}`}
                                        </div>
                                      </div>
                                      <div>{formatPct(c.cpu_pct)}</div>
                                      <div>{formatBytes(c.memory_bytes)}</div>
                                      <div className="settings-table-mono">{c.pid}</div>
                                    </div>
                                  ))}
                                </div>
                              </>
                            )}
                          </div>
                        ) : null}
                      </Fragment>
                    );
                  })}
                </div>
              )}
            </div>
          </Card>

          <Card title="Workspace Disk">
            <div className="settings-card-block">
              <div className="settings-workspace-header">
                <div className="settings-workspace-title">{workspaceName}</div>
                <div className="settings-workspace-path">{snapshot?.workspace.root_path ?? "—"}</div>
                <div className="settings-workspace-meta">
                  {snapshot ? `${formatBytes(snapshot.workspace.size_bytes)} total · ${diskUpdated}` : "Sizing…"}
                </div>
              </div>

              {worktreeRows.length === 0 ? (
                <div className="settings-empty">No worktrees found.</div>
              ) : (
                <div className="settings-table settings-table-worktrees">
                  <div className="settings-table-head">
                    <div>Worktree</div>
                    <div>Size</div>
                  </div>
                  {worktreeRows.map((wt) => (
                    <div key={wt.worktree_id} className="settings-table-row">
                      <div>
                        <div className="settings-table-title">{wt.worktree_id}</div>
                        <div className="settings-table-sub">{wt.root_path}</div>
                      </div>
                      <div>{formatBytes(wt.size_bytes)}</div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </Card>

          {resourceError ? <div className="settings-banner settings-banner-error">{resourceError}</div> : null}
        </>
      );
    }

    if (active === "dictation") {
      return <DictationSection active={active === "dictation"} />;
    }

    if (active === "title_generation") {
      return <TitleGenerationSection active={active === "title_generation"} />;
    }

    if (active === "billing") {
      if (!supabase) {
        return (
          <div className="settings-empty">
            Billing is not configured. Set <code>VITE_SUPABASE_URL</code> and <code>VITE_SUPABASE_ANON_KEY</code> for the web app.
          </div>
        );
      }

      const plan = entitlements?.plan_type ?? "free_local";
      const proEnabled = entitlements?.features?.remote_mobile_access === "enabled";
      const doSignIn = async () => {
        setBillingBusy(true);
        setBillingError(null);
        try {
          const { error } = await supabase.auth.signInWithPassword({
            email: billingEmail.trim(),
            password: billingPassword,
          });
          if (error) throw error;
        } catch (e: any) {
          setBillingError(e?.message ?? String(e));
        } finally {
          setBillingBusy(false);
        }
      };

      const doSignUp = async () => {
        setBillingBusy(true);
        setBillingError(null);
        try {
          const { error } = await supabase.auth.signUp({
            email: billingEmail.trim(),
            password: billingPassword,
          });
          if (error) throw error;
        } catch (e: any) {
          setBillingError(e?.message ?? String(e));
        } finally {
          setBillingBusy(false);
        }
      };

      const doSignOut = async () => {
        setBillingBusy(true);
        setBillingError(null);
        try {
          const { error } = await supabase.auth.signOut();
          if (error) throw error;
        } catch (e: any) {
          setBillingError(e?.message ?? String(e));
        } finally {
          setBillingBusy(false);
        }
      };

      const startCheckout = async (interval: "month" | "year") => {
        setBillingBusy(true);
        setBillingError(null);
        try {
          const url = await runBillingCheckoutFlow({
            interval,
            returnPath: billingReturnPath,
            invokeCheckout: ({ interval: nextInterval, returnPath }) => supabase.functions.invoke(
              "billing-checkout",
              {
                body: { interval: nextInterval, return_path: returnPath },
              },
            ),
            trackSubscribeCtaClicked,
            trackCheckoutStarted,
          });
          window.location.href = url;
        } catch (e: any) {
          setBillingError(e?.message ?? String(e));
          setBillingBusy(false);
        }
      };

      const openPortal = async () => {
        setBillingBusy(true);
        setBillingError(null);
        try {
          const res = await supabase.functions.invoke("billing-portal", {
            body: { return_path: billingReturnPath },
          });
          if (res.error) throw res.error;
          const url = String((res.data as any)?.url ?? "").trim();
          if (!url) throw new Error("Portal URL missing.");
          window.location.href = url;
        } catch (e: any) {
          setBillingError(e?.message ?? String(e));
          setBillingBusy(false);
        }
      };

      return (
        <>
          {checkoutStatus === "success" ? (
            <div className="settings-banner">Checkout complete. Confirming subscription…</div>
          ) : null}
          {checkoutStatus === "cancel" ? (
            <div className="settings-banner settings-banner-error">Checkout canceled.</div>
          ) : null}
          <Card title="Account">
            {billingUser ? (
              <Row
                title="Signed in"
                description={billingUser.email ?? "Signed in"}
                control={
                  <button type="button" className="settings-btn settings-btn-secondary" onClick={doSignOut} disabled={billingBusy}>
                    Sign out
                  </button>
                }
              />
            ) : (
              <>
                <Row
                  title="Email"
                  control={
                    <input
                      className="settings-control settings-control-wide"
                      value={billingEmail}
                      onChange={(e) => setBillingEmail(e.target.value)}
                      placeholder="you@company.com"
                    />
                  }
                />
                <Row
                  title="Password"
                  control={
                    <input
                      className="settings-control settings-control-wide"
                      value={billingPassword}
                      onChange={(e) => setBillingPassword(e.target.value)}
                      type="password"
                      placeholder="••••••••"
                    />
                  }
                />
                <Row
                  title="Sign in / Create account"
                  description="Subscriptions are purchased via Stripe on desktop; mobile devices inherit access when connected."
                  control={
                    <div style={{ display: "flex", gap: 8 }}>
                      <button type="button" className="settings-btn settings-btn-secondary" onClick={doSignIn} disabled={billingBusy}>
                        Sign in
                      </button>
                      <button type="button" className="settings-btn" onClick={doSignUp} disabled={billingBusy}>
                        Create account
                      </button>
                    </div>
                  }
                />
              </>
            )}
          </Card>

          <Card title="Subscription">
            <Row
              title="Plan"
              description={entitlementsBusy ? "Loading…" : proEnabled ? "Pro enabled" : "Free/Local"}
              control={<div className="settings-pill">{plan}</div>}
            />
            <Row
              title="Remote mobile access"
              description="Stable remote access + push notifications are Pro features. Purchase on desktop."
              control={<div className="settings-pill">{proEnabled ? "Enabled" : "Disabled"}</div>}
            />
            <Row
              title="Subscribe"
              description="USD only. CTX Pro is $20/month or $200/year."
              control={
                <div style={{ display: "flex", gap: 8, flexWrap: "wrap", justifyContent: "flex-end" }}>
                  <button type="button" className="settings-btn settings-btn-secondary" onClick={() => startCheckout("month")} disabled={!billingUser || billingBusy}>
                    $20 / month
                  </button>
                  <button type="button" className="settings-btn settings-btn-secondary" onClick={() => startCheckout("year")} disabled={!billingUser || billingBusy}>
                    $200 / year
                  </button>
                  <button type="button" className="settings-btn" onClick={openPortal} disabled={!billingUser || billingBusy}>
                    Manage
                  </button>
                </div>
              }
            />
          </Card>

          {billingError ? <div className="settings-banner settings-banner-error">{billingError}</div> : null}
        </>
      );
    }

    if (active === "agent_harnesses") {
      return (
        <HarnessAuthenticationSection
          workspaceId={workspaceId}
          active={active === "agent_harnesses"}
        />
      );
    }

    if (active === "harness_subscriptions") {
      return <CodexAccountsSection active={active === "harness_subscriptions"} />;
    }

    if (active === "dev_tools") {
      return (
        <DevToolsSection
          devToolsEnabled={devToolsEnabled}
          devRestartBusy={devRestartBusy}
          devRestartError={devRestartError}
          devRestartResults={devRestartResults}
          onRestart={handleDevRestart}
        />
      );
    }

    if (active === "sandboxing") {
      return (
        <>
          <Card title="Sandboxing">
            <Row
              title="Provider control"
              description="Default is full capability. Switch to honor the harness's native permission settings."
              control={
                <Select
                  value={providerControlMode}
                  onValueChange={(value) => setProviderControlMode(value as SandboxingSettings["provider_control_mode"])}
                  disabled={!loaded}
                >
                  <SelectTrigger className="tw-min-w-[10rem]">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="full">Full capability</SelectItem>
                    <SelectItem value="harness_native">Harness-native permissions</SelectItem>
                    <SelectItem value="ctx_enforced">ctx-enforced (coming soon)</SelectItem>
                  </SelectContent>
                </Select>
              }
            />
          </Card>
        </>
      );
    }

    if (
      active === "models_routing" ||
      active === "context_pack" ||
      active === "team_enterprise" ||
      active === "usage_analytics"
    ) {
      return null;
    }

    return <div className="settings-empty">No settings yet.</div>;
  };

  const headerLabel = SECTIONS.find((s) => s.id === active)?.label ?? "Settings";

  return (
    <div className="settings-root">
      <div className="settings-shell">
        <aside className="settings-sidebar">
          <div className="settings-sidebar-header">
            <Link className="settings-backlink" to={backLink.to}>
              {backLink.label}
            </Link>
            <div className="settings-sidebar-title">Settings</div>
          </div>

          <div className="settings-search">
            <input
              className="settings-search-input"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search settings ⌘F"
            />
          </div>

          <nav className="settings-nav" aria-label="Settings sections">
            <div className="settings-nav-group">
              {sidebarSections
                .filter((s) => s.group === "main")
                .map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className={`settings-nav-item ${active === s.id ? "settings-nav-item-active" : ""}`}
                    onClick={() => handleSectionChange(s.id)}
                  >
                    {s.label}
                  </button>
                ))}
            </div>
            <div className="settings-nav-sep" aria-hidden="true" />
            <div className="settings-nav-group">
              {sidebarSections
                .filter((s) => s.group === "advanced")
                .map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className={`settings-nav-item ${active === s.id ? "settings-nav-item-active" : ""}`}
                    onClick={() => handleSectionChange(s.id)}
                  >
                    {s.label}
                  </button>
                ))}
            </div>
          </nav>
        </aside>

        <main className="settings-main">
          <div className="settings-main-inner">
            <div className="settings-main-header">
              <div className="settings-main-title">{headerLabel}</div>
              {anySaving || saveError ? (
                <div className="settings-main-sub">{anySaving ? "Saving…" : "Not saved"}</div>
              ) : null}
            </div>

            {saveError ? <div className="settings-banner settings-banner-error">{saveError}</div> : null}
            {renderMain()}
          </div>
        </main>
      </div>
    </div>
  );
}
