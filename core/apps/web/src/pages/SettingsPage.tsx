import { Fragment, type ReactNode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import type { User } from "@supabase/supabase-js";
import { QRCodeSVG } from "qrcode.react";
import {
  DictationSettings,
  InstallInfo,
  MobileAccessStatus,
  EnableMobileAccessResponse,
  ProviderOptions,
  ProviderStatus,
  ResourceGovernanceLimits,
  ResourceGovernanceSettings,
  ResourceGovernanceStatus,
  ResourceUtilization,
  Settings,
  AgentSystemPromptConfig,
  TelemetrySettings,
  TitleGenerationSettings,
  WorkspaceAttachment,
  createWorkspaceAttachment,
  deleteWorkspaceAttachment,
  disableMobileAccess,
  enableMobileAccess,
  getAgentSystemPrompt,
  getMobileAccessStatus,
  Workspace,
  authenticateProviderForWorkspace,
  getInstall,
  getProviderOptions,
  getResourceUtilization,
  getSettings,
  idToString,
  installAllProviders,
  installProvider,
  installStreamUrl,
  listWorkspaceAttachments,
  listInstallEvents,
  listProviders,
  listWorkspaces,
  syncWorkspaceAttachments,
  updateSettings,
  updateAgentSystemPrompt,
  verifyProviderForWorkspace,
} from "../api/client";
import {
  type DesktopEditorSettings,
  desktopGetEditorSettings,
  desktopUpdateEditorSettings,
  isDesktopApp,
} from "../utils/desktop";
import { HARNESS_CATALOG, type HarnessCatalogEntry } from "../utils/harnessCatalog";
import {
  ENTITLEMENTS_CACHE_KEY,
  ENTITLEMENTS_CACHE_TTL_MS,
  readCachedValue,
  shouldUseCachedValue,
  writeCachedValue,
} from "../utils/entitlementsCache";
import { getSupabaseClient } from "../utils/supabaseClient";

const MODEL_OPTIONS: Array<{ value: string; label: string }> = [
  { value: "auto", label: "Default (Deepgram Nova-3)" },
  { value: "deepgram/flux-general", label: "Deepgram Flux" },
  { value: "deepgram/nova-3", label: "Deepgram Nova-3" },
  { value: "deepgram/nova-3-medical", label: "Deepgram Nova-3 Medical" },
  { value: "deepgram/nova-2", label: "Deepgram Nova-2" },
  { value: "deepgram/nova-2-medical", label: "Deepgram Nova-2 Medical" },
  { value: "deepgram/nova-2-conversationalai", label: "Deepgram Nova-2 Conversational AI" },
  { value: "deepgram/nova-2-phonecall", label: "Deepgram Nova-2 Phonecall" },
  { value: "assemblyai/universal-streaming", label: "AssemblyAI Universal-Streaming" },
  { value: "assemblyai/universal-streaming-multilingual", label: "AssemblyAI Universal-Streaming Multilingual" },
  { value: "cartesia/ink-whisper", label: "Cartesia Ink Whisper" },
  { value: "elevenlabs/scribe_v2_realtime", label: "ElevenLabs Scribe V2 Realtime" },
];

const EDITOR_OPTIONS: Array<{ value: DesktopEditorSettings["target"]; label: string }> = [
  { value: "system", label: "System default" },
  { value: "vscode", label: "Visual Studio Code" },
  { value: "vscode_insiders", label: "Visual Studio Code Insiders" },
  { value: "cursor", label: "Cursor" },
  { value: "windsurf", label: "Windsurf" },
  { value: "antigravity", label: "Google Antigravity" },
  { value: "idea", label: "IntelliJ IDEA" },
  { value: "pycharm", label: "PyCharm" },
  { value: "xcode", label: "Xcode" },
  { value: "android_studio", label: "Android Studio" },
  { value: "custom", label: "Custom command" },
];

type SectionId =
  | "general"
  | "agent_harnesses"
  | "models_routing"
  | "sandboxing"
  | "worktree_bootstrap"
  | "agent_system_prompt"
  | "workspace_attachments"
  | "context_pack"
  | "resource_governance"
  | "mobile_access"
  | "resource_utilization"
  | "dictation"
  | "title_generation"
  | "billing"
  | "team_enterprise"
  | "usage_analytics";

type InstallSession = {
  installId: string;
  state: InstallInfo["state"];
  pct: number | null;
  streamError?: string;
  error?: string;
};

const SECTIONS: Array<{
  id: SectionId;
  label: string;
  group?: "main" | "advanced";
}> = [
  { id: "general", label: "General", group: "main" },
  { id: "agent_harnesses", label: "Agent Harnesses", group: "main" },
  { id: "models_routing", label: "Models & Routing", group: "main" },
  { id: "sandboxing", label: "Sandboxing", group: "main" },
  { id: "worktree_bootstrap", label: "Worktree Bootstrap", group: "main" },
  { id: "agent_system_prompt", label: "Agent System Prompt", group: "main" },
  { id: "workspace_attachments", label: "Workspace Attachments", group: "main" },
  { id: "context_pack", label: "ctx pack", group: "main" },
  { id: "resource_governance", label: "Resource Limits", group: "main" },
  { id: "mobile_access", label: "Mobile Access", group: "main" },
  { id: "resource_utilization", label: "Resource Utilization", group: "main" },
  { id: "dictation", label: "Dictation", group: "advanced" },
  { id: "title_generation", label: "Title Generation", group: "advanced" },
  { id: "billing", label: "Billing", group: "advanced" },
  { id: "team_enterprise", label: "Team & Enterprise", group: "advanced" },
  { id: "usage_analytics", label: "Usage Analytics", group: "advanced" },
];

function sectionFromHash(hash: string): SectionId | null {
  const raw = String(hash || "").replace(/^#/, "").trim();
  if (!raw) return null;
  return (SECTIONS.find((s) => s.id === raw)?.id ?? null) as any;
}

function clampPct(n: number): number {
  if (!Number.isFinite(n)) return 0;
  return Math.max(0, Math.min(100, n));
}

function formatPct(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  return `${Math.round(value as number)}%`;
}

function formatBytes(value?: number | null): string {
  if (!Number.isFinite(value)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  let idx = 0;
  let v = value as number;
  while (v >= 1024 && idx < units.length - 1) {
    v /= 1024;
    idx += 1;
  }
  const precision = v >= 100 ? 0 : v >= 10 ? 1 : 2;
  return `${v.toFixed(precision)} ${units[idx]}`;
}

function formatAge(ms?: number | null): string {
  if (!Number.isFinite(ms)) return "—";
  const totalSeconds = Math.max(0, Math.round((ms as number) / 1000));
  if (totalSeconds < 60) return `${totalSeconds}s`;
  const mins = Math.floor(totalSeconds / 60);
  const secs = totalSeconds % 60;
  return `${mins}m ${secs}s`;
}

function formatGiB(mb?: number | null): string {
  if (!Number.isFinite(mb) || !mb) return "";
  const gb = (mb as number) / 1024;
  const precision = gb >= 10 ? 0 : 1;
  return gb.toFixed(precision);
}

function parseGiB(value: string): number | null {
  const v = Number(value);
  if (!Number.isFinite(v) || v <= 0) return null;
  return Math.round(v * 1024);
}

function truncateText(value: string, maxLen: number): string {
  const s = String(value ?? "");
  if (s.length <= maxLen) return s;
  return `${s.slice(0, Math.max(0, maxLen - 1))}…`;
}

function guessAttachmentName(source: string): string {
  let cleaned = String(source ?? "").trim();
  if (!cleaned) return "";
  cleaned = cleaned.replace(/[\\/]+$/, "");
  const slashIdx = Math.max(cleaned.lastIndexOf("/"), cleaned.lastIndexOf(":"));
  let name = slashIdx >= 0 ? cleaned.slice(slashIdx + 1) : cleaned;
  if (name.endsWith(".git")) name = name.slice(0, -4);
  return name;
}

function Toggle({
  checked,
  disabled,
  onChange,
  ariaLabel,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: (next: boolean) => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      className={`settings-toggle ${checked ? "settings-toggle-on" : ""}`}
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="settings-toggle-thumb" aria-hidden="true" />
    </button>
  );
}

function Row({
  title,
  description,
  control,
}: {
  title: string;
  description?: string;
  control: ReactNode;
}) {
  return (
    <div className="settings-row">
      <div className="settings-row-left">
        <div className="settings-row-title">{title}</div>
        {description ? <div className="settings-row-desc">{description}</div> : null}
      </div>
      <div className="settings-row-right">{control}</div>
    </div>
  );
}

function Card({ children, title }: { title?: string; children: ReactNode }) {
  return (
    <div className="settings-card">
      {title ? <div className="settings-card-title">{title}</div> : null}
      <div className="settings-card-rows">{children}</div>
    </div>
  );
}

function Metric({
  label,
  value,
  sublabel,
  pct,
}: {
  label: string;
  value: string;
  sublabel?: string;
  pct?: number | null;
}) {
  const safePct = pct === null || pct === undefined ? 0 : clampPct(pct);
  return (
    <div className="settings-metric">
      <div className="settings-metric-header">
        <div className="settings-metric-label">{label}</div>
        <div className="settings-metric-value">{value}</div>
      </div>
      <div className="settings-meter-track" role="presentation">
        <div className="settings-meter-fill" style={{ width: `${safePct}%` }} />
      </div>
      {sublabel ? <div className="settings-metric-sub">{sublabel}</div> : null}
    </div>
  );
}

export default function SettingsPage() {
  const location = useLocation();
  const navigate = useNavigate();
  const [active, setActive] = useState<SectionId>(() => sectionFromHash(window.location.hash) ?? "general");
  const [query, setQuery] = useState("");
  const supabase = useMemo(() => getSupabaseClient(), []);

  const billingReturnPath = useMemo(() => {
    const params = new URLSearchParams(location.search);
    params.delete("checkout");
    params.delete("session_id");
    const search = params.toString();
    return `${location.pathname}${search ? `?${search}` : ""}#billing`;
  }, [location.pathname, location.search]);

  const clearCheckoutStatus = useCallback(() => {
    const params = new URLSearchParams(location.search);
    if (!params.has("checkout") && !params.has("session_id")) return;
    params.delete("checkout");
    params.delete("session_id");
    const search = params.toString();
    navigate(
      {
        pathname: location.pathname,
        search: search ? `?${search}` : "",
        hash: location.hash,
      },
      { replace: true },
    );
  }, [location.hash, location.pathname, location.search, navigate]);

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
  const [entitlements, setEntitlements] = useState<EntitlementsSnapshot | null>(() => {
    try {
      const cached = readCachedValue<EntitlementsSnapshot>(window.localStorage, ENTITLEMENTS_CACHE_KEY);
      return cached?.value ?? null;
    } catch {
      return null;
    }
  });
  const [entitlementsBusy, setEntitlementsBusy] = useState(false);

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
  const dictationHydrated = useRef(false);
  const titleGenerationHydrated = useRef(false);

  const [telemetryEnabled, setTelemetryEnabled] = useState(true);
  const [telemetryEndpoint, setTelemetryEndpoint] = useState("");

  const [dictationEnabled, setDictationEnabled] = useState(true);
  const [model, setModel] = useState("auto");
  const [language, setLanguage] = useState("en");
  const [baseUrl, setBaseUrl] = useState("https://agent-gateway.livekit.cloud/v1");
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [apiSecretSet, setApiSecretSet] = useState(false);

  const [titleGenBaseUrl, setTitleGenBaseUrl] = useState("https://openrouter.ai/api/v1");
  const [titleGenApiKey, setTitleGenApiKey] = useState("");
  const [titleGenModel, setTitleGenModel] = useState("google/gemini-3-flash-preview");
  const [titleGenUseJson, setTitleGenUseJson] = useState(true);
  const resourceGovernanceHydrated = useRef(false);
  const [resourceGovernanceEnabled, setResourceGovernanceEnabled] = useState(true);
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

  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [providerOptions, setProviderOptions] = useState<Record<string, ProviderOptions | undefined>>({});
  const [optsBusy, setOptsBusy] = useState<Record<string, boolean>>({});
  const [authBusy, setAuthBusy] = useState<Record<string, boolean>>({});
  const [verifyBusy, setVerifyBusy] = useState<Record<string, boolean>>({});
  const [providerError, setProviderError] = useState<string | null>(null);
  const [installBusy, setInstallBusy] = useState<string | null>(null);
  const [installs, setInstalls] = useState<Record<string, InstallSession>>({});
  const [attachments, setAttachments] = useState<WorkspaceAttachment[]>([]);
  const [attachmentsLoading, setAttachmentsLoading] = useState(false);
  const [attachmentsError, setAttachmentsError] = useState<string | null>(null);
  const [attachmentName, setAttachmentName] = useState("");
  const [attachmentSource, setAttachmentSource] = useState("");
  const [attachmentRevision, setAttachmentRevision] = useState("");
  const [attachmentBusy, setAttachmentBusy] = useState(false);
  const [docsAttachmentName, setDocsAttachmentName] = useState("");
  const [docsAttachmentSource, setDocsAttachmentSource] = useState("");
  const [docsAttachmentBusy, setDocsAttachmentBusy] = useState(false);
  const [attachmentSyncBusy, setAttachmentSyncBusy] = useState(false);
  const [attachmentDeleteBusy, setAttachmentDeleteBusy] = useState<Record<string, boolean>>({});

  const [agentPromptConfig, setAgentPromptConfig] = useState<AgentSystemPromptConfig | null>(null);
  const [agentPromptLoading, setAgentPromptLoading] = useState(false);
  const [agentPromptError, setAgentPromptError] = useState<string | null>(null);
  const [agentPromptSaving, setAgentPromptSaving] = useState(false);
  const [agentPromptUseDefault, setAgentPromptUseDefault] = useState(true);
  const [agentPromptCustom, setAgentPromptCustom] = useState("");

  const [resourceSnapshot, setResourceSnapshot] = useState<ResourceUtilization | null>(null);
  const [resourceLoading, setResourceLoading] = useState(false);
  const [resourceError, setResourceError] = useState<string | null>(null);
  const resourcePollRef = useRef<number | null>(null);
  const [expandedProcessPids, setExpandedProcessPids] = useState<Record<number, boolean>>({});

  const eventSourcesRef = useRef<Record<string, EventSource>>({});
  const pollTimeoutsRef = useRef<Record<string, number>>({});

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

        const d = s.dictation ?? null;
        if (d) {
          const normalizeModel = (m: string): string => {
            const v = String(m || "").trim();
            if (!v || v === "auto") return "auto";
            if (v === "elevenlabs/scribe-v2-realtime") return "elevenlabs/scribe_v2_realtime";
            if (v === "deepgram/flux") return "deepgram/flux-general";
            return v;
          };

          setDictationEnabled(d.enabled);
          setModel(normalizeModel(d.livekit?.model ?? "auto"));
          setLanguage(d.livekit?.language ?? "en");
          setBaseUrl(d.livekit?.base_url ?? "https://agent-gateway.livekit.cloud/v1");
          setApiKey(d.livekit?.api_key ?? "");
          setApiSecretSet(Boolean(d.livekit?.api_secret_set));
        }

        const tg = s.title_generation ?? null;
        if (tg) {
          setTitleGenBaseUrl(tg.base_url ?? "https://openrouter.ai/api/v1");
          setTitleGenApiKey(tg.api_key ?? "");
          setTitleGenModel(tg.model ?? "google/gemini-3-flash-preview");
          setTitleGenUseJson(Boolean(tg.use_json));
        }

        const rg = s.resource_governance ?? null;
        if (rg) {
          setResourceGovernanceEnabled(rg.enabled);
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

  const savePatch = async (patch: Partial<Settings>) => {
    setSaveError(null);
    setSaving(true);
    const seq = ++saveSeq.current;
    try {
      const next = await updateSettings(patch as Settings);
      if (seq !== saveSeq.current) return;
      if (next.dictation?.livekit?.api_secret_set) {
        setApiSecret("");
        setApiSecretSet(true);
      }
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
    } catch (e: any) {
      if (seq !== saveSeq.current) return;
      setSaveError(e?.message ?? String(e));
    } finally {
      if (seq === saveSeq.current) setSaving(false);
    }
  };

  const dictationPayload = useMemo((): DictationSettings => {
    return {
      enabled: dictationEnabled,
      provider: dictationEnabled ? "livekit_inference" : "disabled",
      livekit: {
        base_url: baseUrl.trim(),
        api_key: apiKey.trim(),
        api_secret: apiSecret.trim() ? apiSecret : null,
        model,
        language: language.trim() || "en",
      },
    };
  }, [apiKey, apiSecret, baseUrl, dictationEnabled, language, model]);

  const titleGenerationPayload = useMemo((): TitleGenerationSettings => {
    return {
      base_url: titleGenBaseUrl.trim(),
      api_key: titleGenApiKey.trim(),
      model: titleGenModel.trim(),
      use_json: titleGenUseJson,
    };
  }, [titleGenApiKey, titleGenBaseUrl, titleGenModel, titleGenUseJson]);

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

  const dictationCanSave = useMemo(() => {
    if (!dictationEnabled) return true;
    if (!apiKey.trim()) return false;
    if (!apiSecretSet && !apiSecret.trim()) return false;
    return true;
  }, [apiKey, apiSecret, apiSecretSet, dictationEnabled]);

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
    if (!dictationHydrated.current) {
      dictationHydrated.current = true;
      return;
    }
    if (!dictationCanSave) return;
    const t = window.setTimeout(() => {
      savePatch({ dictation: dictationPayload });
    }, 450);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dictationPayload, loaded, dictationCanSave]);

  useEffect(() => {
    if (!loaded) return;
    if (!titleGenerationHydrated.current) {
      titleGenerationHydrated.current = true;
      return;
    }
    const t = window.setTimeout(() => {
      savePatch({ title_generation: titleGenerationPayload });
    }, 450);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, titleGenerationPayload]);

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

  const refreshProviders = () =>
    listProviders()
      .then(setProviders)
      .catch((e: any) => setProviderError(e?.message ?? String(e)));

  const refreshWorkspaceAttachments = useCallback(
    async (opts?: { refresh?: boolean }) => {
      if (!workspaceId) return;
      setAttachmentsLoading(true);
      setAttachmentsError(null);
      try {
        const next = opts?.refresh
          ? await syncWorkspaceAttachments(workspaceId, true)
          : await listWorkspaceAttachments(workspaceId);
        setAttachments(next);
      } catch (e: any) {
        setAttachmentsError(e?.message ?? String(e));
      } finally {
        setAttachmentsLoading(false);
      }
    },
    [workspaceId],
  );

  const refreshAgentSystemPrompt = useCallback(async () => {
    if (!workspaceId) return;
    setAgentPromptLoading(true);
    setAgentPromptError(null);
    try {
      const next = await getAgentSystemPrompt(workspaceId);
      setAgentPromptConfig(next);
      const baseCustom = next.configured_append ?? next.default_append;
      setAgentPromptCustom(baseCustom ?? "");
      setAgentPromptUseDefault(next.source === "default");
    } catch (e: any) {
      setAgentPromptError(e?.message ?? String(e));
    } finally {
      setAgentPromptLoading(false);
    }
  }, [workspaceId]);

  const handleSaveAgentPrompt = useCallback(async () => {
    if (!workspaceId) return;
    setAgentPromptSaving(true);
    setAgentPromptError(null);
    try {
      const payload = agentPromptUseDefault ? null : agentPromptCustom;
      const next = await updateAgentSystemPrompt(workspaceId, { system_prompt_append: payload });
      setAgentPromptConfig(next);
      const baseCustom = next.configured_append ?? next.default_append;
      setAgentPromptCustom(baseCustom ?? "");
      setAgentPromptUseDefault(next.source === "default");
    } catch (e: any) {
      setAgentPromptError(e?.message ?? String(e));
    } finally {
      setAgentPromptSaving(false);
    }
  }, [workspaceId, agentPromptUseDefault, agentPromptCustom]);

  const syncWorkspaceAttachmentsNow = useCallback(async () => {
    if (!workspaceId) return;
    setAttachmentSyncBusy(true);
    await refreshWorkspaceAttachments({ refresh: true });
    setAttachmentSyncBusy(false);
  }, [workspaceId, refreshWorkspaceAttachments]);

  useEffect(() => {
    refreshProviders();
    listWorkspaces()
      .then((ws) => {
        setWorkspaces(ws);
        if (!workspaceId && ws.length > 0) setWorkspaceId(idToString((ws[0] as any).id));
      })
      .catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    setProviderOptions({});
  }, [workspaceId]);

  useEffect(() => {
    setAttachments([]);
    setAttachmentsError(null);
  }, [workspaceId]);

  useEffect(() => {
    if (active !== "agent_harnesses") return;
    if (!workspaceId) return;
    for (const p of providers) {
      if (p.details?.ui_hidden === "true") continue;
      ensureProviderOpts(p.provider_id).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, workspaceId, providers]);

  useEffect(() => {
    for (const p of providers) {
      const installId = p.details?.install_id;
      const running = p.details?.install_running === "true";
      if (running && installId && !installs[p.provider_id]) {
        attachInstall(p.provider_id, installId);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [providers]);

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

  useEffect(() => {
    if (active !== "agent_system_prompt") return;
    if (!workspaceId) return;
    refreshAgentSystemPrompt().catch(() => {});
  }, [active, workspaceId, refreshAgentSystemPrompt]);

  useEffect(() => {
    if (active !== "workspace_attachments") return;
    if (!workspaceId) return;
    refreshWorkspaceAttachments().catch(() => {});
  }, [active, workspaceId, refreshWorkspaceAttachments]);

  useEffect(() => {
    return () => {
      for (const key of Object.keys(eventSourcesRef.current)) eventSourcesRef.current[key].close();
      for (const key of Object.keys(pollTimeoutsRef.current)) window.clearTimeout(pollTimeoutsRef.current[key]);
      eventSourcesRef.current = {};
      pollTimeoutsRef.current = {};
    };
  }, []);

  const attachInstall = async (providerId: string, installId: string) => {
    if (eventSourcesRef.current[providerId] || pollTimeoutsRef.current[providerId]) return;

    setInstalls((prev) => ({
      ...prev,
      [providerId]: {
        installId,
        state: "running",
        pct: prev[providerId]?.pct ?? null,
        streamError: prev[providerId]?.streamError,
        error: prev[providerId]?.error,
      },
    }));

    try {
      const history = await listInstallEvents(installId);
      const last = history[history.length - 1];
      const pct =
        last && typeof last.bytes === "number" && typeof last.total_bytes === "number" && last.total_bytes > 0
          ? clampPct(Math.round((last.bytes / last.total_bytes) * 100))
          : null;
      setInstalls((prev) => ({
        ...prev,
        [providerId]: {
          installId,
          state: prev[providerId]?.state ?? "running",
          pct,
          streamError: prev[providerId]?.streamError,
          error: prev[providerId]?.error,
        },
      }));
    } catch {
      // ignore
    }

    let eventSource: EventSource | null = null;
    try {
      if (isDesktopApp()) throw new Error("desktop mode uses polling (no EventSource)");
      eventSource = new EventSource(installStreamUrl(installId));
      eventSourcesRef.current[providerId] = eventSource;
      eventSource.addEventListener("progress", (evt: any) => {
        const data = (evt as MessageEvent).data;
        try {
          const ev = JSON.parse(data) as any;
          const pct =
            typeof ev.bytes === "number" && typeof ev.total_bytes === "number" && ev.total_bytes > 0
              ? clampPct(Math.round((ev.bytes / ev.total_bytes) * 100))
              : null;
          setInstalls((prev) => ({
            ...prev,
            [providerId]: {
              installId,
              state: prev[providerId]?.state ?? "running",
              pct,
              streamError: prev[providerId]?.streamError,
              error: prev[providerId]?.error,
            },
          }));
        } catch {
          // ignore
        }
      });
      eventSource.onerror = () => {
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: prev[providerId]?.state ?? "running",
            pct: prev[providerId]?.pct ?? null,
            streamError: "Lost connection to install stream; polling…",
            error: prev[providerId]?.error,
          },
        }));
        eventSource?.close();
        delete eventSourcesRef.current[providerId];
      };
    } catch {
      setInstalls((prev) => ({
        ...prev,
        [providerId]: {
          installId,
          state: prev[providerId]?.state ?? "running",
          pct: prev[providerId]?.pct ?? null,
          streamError: "Failed to open install stream; polling…",
          error: prev[providerId]?.error,
        },
      }));
    }

    const poll = async () => {
      try {
        const info = await getInstall(installId);
        setInstalls((prev) => ({
          ...prev,
          [providerId]: {
            installId,
            state: info.state,
            pct: prev[providerId]?.pct ?? null,
            streamError: prev[providerId]?.streamError,
            error: info.error,
          },
        }));
        if (info.state !== "running") {
          eventSource?.close();
          delete eventSourcesRef.current[providerId];
          const t = pollTimeoutsRef.current[providerId];
          if (t) {
            window.clearTimeout(t);
            delete pollTimeoutsRef.current[providerId];
          }
          await refreshProviders();
          return;
        }
      } catch {
        // ignore
      }
      pollTimeoutsRef.current[providerId] = window.setTimeout(poll, 900);
    };
    poll();
  };

  const ensureProviderOpts = async (providerId: string, opts?: { force?: boolean }) => {
    if (!workspaceId) return;
    if (optsBusy[providerId]) return;
    if (!opts?.force && providerOptions[providerId]) return;
    setOptsBusy((prev) => ({ ...prev, [providerId]: true }));
    try {
      const o = await getProviderOptions(workspaceId, providerId);
      setProviderOptions((prev) => ({ ...prev, [providerId]: o }));
    } catch (e: any) {
      setProviderError(e?.message ?? String(e));
    } finally {
      setOptsBusy((prev) => ({ ...prev, [providerId]: false }));
    }
  };

  const onAuthenticate = async (providerId: string) => {
    if (!workspaceId) return;
    setAuthBusy((prev) => ({ ...prev, [providerId]: true }));
    setProviderError(null);
    try {
      await authenticateProviderForWorkspace(workspaceId, providerId);
    } catch (e: any) {
      setProviderError(e?.message ?? String(e));
    } finally {
      setAuthBusy((prev) => ({ ...prev, [providerId]: false }));
      ensureProviderOpts(providerId, { force: true }).catch(() => {});
    }
  };

  const onVerify = async (providerId: string) => {
    if (!workspaceId) return;
    setVerifyBusy((prev) => ({ ...prev, [providerId]: true }));
    setProviderError(null);
    try {
      await verifyProviderForWorkspace(workspaceId, providerId);
    } catch (e: any) {
      setProviderError(e?.message ?? String(e));
    } finally {
      setVerifyBusy((prev) => ({ ...prev, [providerId]: false }));
      ensureProviderOpts(providerId, { force: true }).catch(() => {});
    }
  };

  const onInstall = async (id: string) => {
    setInstallBusy(id);
    setProviderError(null);
    try {
      const { install_id } = await installProvider(id);
      await attachInstall(id, install_id);
    } catch (e: any) {
      setProviderError(e?.message ?? String(e));
    } finally {
      setInstallBusy(null);
    }
  };

  const onInstallAll = async () => {
    setInstallBusy("all");
    setProviderError(null);
    try {
      const started = await installAllProviders();
      for (const i of started) attachInstall(i.provider_id, i.install_id);
    } catch (e: any) {
      setProviderError(e?.message ?? String(e));
    } finally {
      setInstallBusy(null);
    }
  };

  const handleAddAttachment = useCallback(async () => {
    if (!workspaceId) return;
    const source = attachmentSource.trim();
    const revision = attachmentRevision.trim();
    const name = attachmentName.trim() || guessAttachmentName(source);
    if (!source) {
      setAttachmentsError("Repository URL is required.");
      return;
    }
    if (!name) {
      setAttachmentsError("Attachment name is required.");
      return;
    }
    setAttachmentBusy(true);
    setAttachmentsError(null);
    try {
      const next = await createWorkspaceAttachment(workspaceId, {
        kind: "reference_repo",
        name,
        source,
        revision: revision || null,
      });
      setAttachments(next);
      setAttachmentName("");
      setAttachmentSource("");
      setAttachmentRevision("");
    } catch (e: any) {
      setAttachmentsError(e?.message ?? String(e));
    } finally {
      setAttachmentBusy(false);
    }
  }, [workspaceId, attachmentSource, attachmentRevision, attachmentName, createWorkspaceAttachment]);

  const handleAddDocsAttachment = useCallback(async () => {
    if (!workspaceId) return;
    const source = docsAttachmentSource.trim();
    const name = docsAttachmentName.trim() || guessAttachmentName(source);
    if (!source) {
      setAttachmentsError("Docs URL is required.");
      return;
    }
    if (!name) {
      setAttachmentsError("Attachment name is required.");
      return;
    }
    setDocsAttachmentBusy(true);
    setAttachmentsError(null);
    try {
      const next = await createWorkspaceAttachment(workspaceId, {
        kind: "doc_mirror",
        name,
        source,
      });
      setAttachments(next);
      setDocsAttachmentName("");
      setDocsAttachmentSource("");
    } catch (e: any) {
      setAttachmentsError(e?.message ?? String(e));
    } finally {
      setDocsAttachmentBusy(false);
    }
  }, [workspaceId, docsAttachmentSource, docsAttachmentName, createWorkspaceAttachment]);

  const handleRemoveAttachment = useCallback(
    async (attachment: WorkspaceAttachment) => {
      if (!workspaceId) return;
      const confirmed = window.confirm(`Remove "${attachment.name}" from workspace attachments?`);
      if (!confirmed) return;
      const id = idToString(attachment.id);
      setAttachmentDeleteBusy((prev) => ({ ...prev, [id]: true }));
      setAttachmentsError(null);
      try {
        const next = await deleteWorkspaceAttachment(workspaceId, {
          kind: attachment.kind,
          name: attachment.name,
        });
        setAttachments(next);
      } catch (e: any) {
        setAttachmentsError(e?.message ?? String(e));
      } finally {
        setAttachmentDeleteBusy((prev) => {
          const copy = { ...prev };
          delete copy[id];
          return copy;
        });
      }
    },
    [workspaceId, deleteWorkspaceAttachment, idToString],
  );

  const sidebarSections = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = [...SECTIONS];
    if (!q) return all;
    return all.filter((s) => s.label.toLowerCase().includes(q));
  }, [query]);

  const workspaceFromQuery = useMemo(() => {
    const ws = new URLSearchParams(location.search).get("ws");
    if (!ws) return null;
    const trimmed = ws.trim();
    return trimmed ? trimmed : null;
  }, [location.search]);

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

  const anySaving = saving || editorSaving || agentPromptSaving;

  const vscodeRemoteTargets: DesktopEditorSettings["target"][] = [
    "vscode",
    "vscode_insiders",
    "cursor",
    "windsurf",
    "antigravity",
  ];
  const showRemoteAuthority = vscodeRemoteTargets.includes(editorSettings.target);

  const renderMain = () => {
    if (!loaded) return <div className="settings-empty">Loading…</div>;
    if (loadError) return <div className="settings-empty settings-empty-error">{loadError}</div>;

    if (active === "general") {
      return (
        <>
          <Card>
            <Row
              title="Telemetry"
              description="Share anonymous usage metrics (no code, prompts, or file paths)."
              control={
                <Toggle checked={telemetryEnabled} disabled={!loaded} onChange={setTelemetryEnabled} ariaLabel="Telemetry" />
              }
            />
            <Row
              title="Default IDE"
              description={isDesktopApp() ? "Used for open-in-editor links." : "Available in the desktop app."}
              control={
                <select
                  className="settings-control settings-select"
                  value={editorSettings.target}
                  onChange={(e) =>
                    setEditorSettings((prev) => ({
                      ...prev,
                      target: e.target.value as DesktopEditorSettings["target"],
                    }))
                  }
                  disabled={!isDesktopApp() || !editorLoaded}
                >
                  {EDITOR_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              }
            />
            {editorSettings.target === "custom" ? (
              <Row
                title="Custom IDE command"
                description="Command to run when opening files."
                control={
                  <input
                    className="settings-control settings-control-wide"
                    value={editorSettings.custom_command ?? ""}
                    onChange={(e) => setEditorSettings((prev) => ({ ...prev, custom_command: e.target.value }))}
                    disabled={!isDesktopApp() || !editorLoaded}
                    placeholder="code --goto {path}:{line}:{col}"
                  />
                }
              />
            ) : null}
            {showRemoteAuthority ? (
              <Row
                title="VS Code Remote Authority"
                description="Optional: ssh-remote+my-host for remote worktrees."
                control={
                  <input
                    className="settings-control settings-control-wide"
                    value={editorSettings.remote_authority ?? ""}
                    onChange={(e) => setEditorSettings((prev) => ({ ...prev, remote_authority: e.target.value }))}
                    disabled={!isDesktopApp() || !editorLoaded}
                    placeholder="ssh-remote+my-host"
                  />
                }
              />
            ) : null}
          </Card>
          {editorError ? <div className="settings-banner settings-banner-error">{editorError}</div> : null}
        </>
      );
    }

    if (active === "worktree_bootstrap") {
      const anyWorkspace = workspaces.length > 0;
      const selectedWorkspace = workspaces.find((ws) => idToString((ws as any).id) === workspaceId) ?? null;
      const configPath = selectedWorkspace ? `${selectedWorkspace.root_path}/.ctx/config.toml` : ".ctx/config.toml";
      const example = `[worktree.bootstrap]\nsetup_worktree = [\"pnpm install\", \"cargo fetch --locked\"]\nsetup_worktree_unix = \"scripts/worktree_bootstrap_unix.sh\"\nsetup_worktree_windows = \"scripts/worktree_bootstrap_windows.ps1\"\ntimeout_sec = 60\nwait_for_completion = false\n`;

      return (
        <>
          <Card title="Worktree Bootstrap">
            <Row
              title="Workspace"
              description="Choose the repo to edit."
              control={
                <select
                  className="settings-control settings-select"
                  value={workspaceId ?? ""}
                  onChange={(e) => setWorkspaceId(e.target.value || null)}
                  disabled={!anyWorkspace}
                >
                  {workspaces.map((ws) => {
                    const id = idToString((ws as any).id);
                    return (
                      <option key={id} value={id}>
                        {ws.name}
                      </option>
                    );
                  })}
                </select>
              }
            />
            <Row
              title="Config file"
              description="Repo-scoped worktree bootstrap configuration."
              control={<span className="settings-pill wb-mono">{configPath}</span>}
            />
            <Row
              title="Example"
              description="Add this section to enable bootstrap."
              control={<pre className="settings-code-block">{example}</pre>}
            />
          </Card>
        </>
      );
    }

    if (active === "agent_system_prompt") {
      const anyWorkspace = workspaces.length > 0;
      const selectedWorkspace = workspaces.find((ws) => idToString((ws as any).id) === workspaceId) ?? null;
      const configPath =
        agentPromptConfig?.config_path ??
        (selectedWorkspace ? `${selectedWorkspace.root_path}/.ctx/config.toml` : ".ctx/config.toml");
      const statusLabel =
        agentPromptConfig?.source === "config"
          ? "Custom"
          : agentPromptConfig?.source === "disabled"
            ? "Disabled"
            : "Default";
      const baseCustom = agentPromptConfig?.configured_append ?? agentPromptConfig?.default_append ?? "";
      const baseUseDefault = agentPromptConfig?.source === "default";
      const promptDirty =
        agentPromptConfig &&
        (agentPromptUseDefault !== baseUseDefault ||
          (!agentPromptUseDefault && agentPromptCustom.trim() !== baseCustom.trim()));
      const canSave = Boolean(workspaceId) && !agentPromptSaving && Boolean(promptDirty);

      return (
        <>
          <Card title="Agent System Prompt">
            <Row
              title="Workspace"
              description="Choose the repo to configure."
              control={
                <select
                  className="settings-control settings-select"
                  value={workspaceId ?? ""}
                  onChange={(e) => setWorkspaceId(e.target.value || null)}
                  disabled={!anyWorkspace}
                >
                  {workspaces.map((ws) => {
                    const id = idToString((ws as any).id);
                    return (
                      <option key={id} value={id}>
                        {ws.name}
                      </option>
                    );
                  })}
                </select>
              }
            />
            <Row
              title="Config file"
              description="Repo-scoped agent prompt configuration."
              control={<span className="settings-pill wb-mono">{configPath}</span>}
            />
            <Row
              title="Status"
              description="Default prompt applies when no override is set."
              control={<span className="settings-pill">{statusLabel}</span>}
            />
            <Row
              title="Default prompt"
              description="Applied when the config has no override."
              control={<pre className="settings-code-block">{agentPromptConfig?.default_append ?? ""}</pre>}
            />
            <Row
              title="Use default"
              description="Turn off to provide a custom override."
              control={
                <Toggle
                  checked={agentPromptUseDefault}
                  disabled={!workspaceId || agentPromptLoading}
                  onChange={setAgentPromptUseDefault}
                  ariaLabel="Use default agent system prompt"
                />
              }
            />
            <Row
              title="Custom override"
              description="Saved to .ctx/config.toml when you save."
              control={
                <textarea
                  className="settings-control settings-control-wide"
                  rows={5}
                  value={agentPromptCustom}
                  onChange={(e) => setAgentPromptCustom(e.target.value)}
                  disabled={!workspaceId || agentPromptUseDefault || agentPromptLoading}
                  placeholder="Add a custom system prompt append."
                />
              }
            />
            <Row
              title="Actions"
              control={
                <button
                  type="button"
                  className="settings-btn"
                  onClick={() => handleSaveAgentPrompt().catch(() => {})}
                  disabled={!canSave}
                >
                  {agentPromptSaving ? "Saving…" : "Save"}
                </button>
              }
            />
          </Card>
          {agentPromptLoading ? <div className="settings-banner">Loading agent prompt…</div> : null}
          {agentPromptError ? <div className="settings-banner settings-banner-error">{agentPromptError}</div> : null}
        </>
      );
    }

    if (active === "workspace_attachments") {
      const anyWorkspace = workspaces.length > 0;
      const selectedWorkspace = workspaces.find((ws) => idToString((ws as any).id) === workspaceId) ?? null;
      const configPath = selectedWorkspace ? `${selectedWorkspace.root_path}/.ctx/attachments.toml` : ".ctx/attachments.toml";
      const canAdd = Boolean(workspaceId && attachmentSource.trim());
      const canAddDocs = Boolean(workspaceId && docsAttachmentSource.trim());

      return (
        <>
          <Card title="Workspace Attachments">
            <Row
              title="Workspace"
              description="Choose the repo to configure."
              control={
                <select
                  className="settings-control settings-select"
                  value={workspaceId ?? ""}
                  onChange={(e) => setWorkspaceId(e.target.value || null)}
                  disabled={!anyWorkspace}
                >
                  {workspaces.map((ws) => {
                    const id = idToString((ws as any).id);
                    return (
                      <option key={id} value={id}>
                        {ws.name}
                      </option>
                    );
                  })}
                </select>
              }
            />
            <Row
              title="Config file"
              description="Repo-scoped attachments configuration."
              control={<span className="settings-pill wb-mono">{configPath}</span>}
            />
            <Row
              title="Mount paths"
              description="Reference repos are mounted inside each track."
              control={<span className="settings-pill wb-mono">.ctx/.refs/&lt;name&gt;</span>}
            />
          </Card>

          <Card title="Reference Repos">
            <div className="settings-card-block">
              <div className="settings-attachments-form">
                <div className="settings-attachments-field">
                  <label className="settings-attachments-label" htmlFor="attachments-source">
                    Repository URL
                  </label>
                  <input
                    id="attachments-source"
                    className="settings-control"
                    value={attachmentSource}
                    onChange={(e) => setAttachmentSource(e.target.value)}
                    placeholder="git@github.com:org/repo.git"
                  />
                </div>
                <div className="settings-attachments-field">
                  <label className="settings-attachments-label" htmlFor="attachments-name">
                    Display name
                  </label>
                  <input
                    id="attachments-name"
                    className="settings-control"
                    value={attachmentName}
                    onChange={(e) => setAttachmentName(e.target.value)}
                    placeholder={guessAttachmentName(attachmentSource) || "reference"}
                  />
                </div>
                <div className="settings-attachments-field">
                  <label className="settings-attachments-label" htmlFor="attachments-revision">
                    Revision (optional)
                  </label>
                  <input
                    id="attachments-revision"
                    className="settings-control"
                    value={attachmentRevision}
                    onChange={(e) => setAttachmentRevision(e.target.value)}
                    placeholder="main or tag"
                  />
                </div>
              </div>
              <div className="settings-attachments-actions">
                <button
                  type="button"
                  className="settings-btn"
                  onClick={() => handleAddAttachment().catch(() => {})}
                  disabled={!canAdd || attachmentBusy || !workspaceId}
                >
                  {attachmentBusy ? "Adding…" : "Add repo"}
                </button>
                <button
                  type="button"
                  className="settings-btn settings-btn-secondary"
                  onClick={() => syncWorkspaceAttachmentsNow().catch(() => {})}
                  disabled={!workspaceId || attachmentSyncBusy}
                >
                  {attachmentSyncBusy ? "Syncing…" : "Sync now"}
                </button>
              </div>
              <div className="settings-attachments-hint">
                Use SSH URLs for private repos. The daemon must have access to your SSH keys.
              </div>
            </div>
            <div className="settings-card-block">
              {attachmentsLoading ? <div className="settings-empty-compact">Loading attachments…</div> : null}
              {!attachmentsLoading && attachments.length === 0 ? (
                <div className="settings-empty-compact">No workspace attachments yet.</div>
              ) : null}
              {!attachmentsLoading && attachments.length > 0 ? (
                <div className="settings-table settings-table-attachments">
                  <div className="settings-table-head">
                    <div>Attachment</div>
                    <div>Source</div>
                    <div>Mount</div>
                    <div>Updated</div>
                    <div />
                  </div>
                  {attachments.map((attachment) => {
                    const updatedMs = Date.parse(attachment.updated_at);
                    const updatedLabel = Number.isFinite(updatedMs)
                      ? `${formatAge(Date.now() - updatedMs)} ago`
                      : "—";
                    const deleteBusy = attachmentDeleteBusy[idToString(attachment.id)] ?? false;
                    return (
                      <div key={idToString(attachment.id)} className="settings-table-row">
                        <div>
                          <div className="settings-table-title">{attachment.name}</div>
                          <div className="settings-table-sub">
                            {attachment.kind === "reference_repo" ? "Reference repo" : "Docs mirror"}
                          </div>
                        </div>
                        <div className="settings-table-mono" title={attachment.source}>
                          {truncateText(attachment.source, 64)}
                        </div>
                        <div className="settings-table-mono" title={attachment.mount_relpath}>
                          {truncateText(attachment.mount_relpath, 32)}
                        </div>
                        <div className="settings-table-sub">{updatedLabel}</div>
                        <div className="settings-row-right">
                          <button
                            type="button"
                            className="settings-btn settings-btn-secondary settings-btn-compact"
                            onClick={() => handleRemoveAttachment(attachment).catch(() => {})}
                            disabled={deleteBusy}
                          >
                            {deleteBusy ? "Removing…" : "Remove"}
                          </button>
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : null}
            </div>
          </Card>

          <Card title="Docs">
            <div className="settings-card-block">
              <div className="settings-attachments-form">
                <div className="settings-attachments-field">
                  <label className="settings-attachments-label" htmlFor="attachments-docs-source">
                    Docs URL
                  </label>
                  <input
                    id="attachments-docs-source"
                    className="settings-control"
                    value={docsAttachmentSource}
                    onChange={(e) => setDocsAttachmentSource(e.target.value)}
                    placeholder="https://docs.example.com/"
                  />
                </div>
                <div className="settings-attachments-field">
                  <label className="settings-attachments-label" htmlFor="attachments-docs-name">
                    Display name
                  </label>
                  <input
                    id="attachments-docs-name"
                    className="settings-control"
                    value={docsAttachmentName}
                    onChange={(e) => setDocsAttachmentName(e.target.value)}
                    placeholder={guessAttachmentName(docsAttachmentSource) || "docs"}
                  />
                </div>
              </div>
              <div className="settings-attachments-actions">
                <button
                  type="button"
                  className="settings-btn"
                  onClick={() => handleAddDocsAttachment().catch(() => {})}
                  disabled={!canAddDocs || docsAttachmentBusy || !workspaceId}
                >
                  {docsAttachmentBusy ? "Adding…" : "Add docs"}
                </button>
              </div>
              <div className="settings-attachments-hint">
                Paste any docs page URL. The daemon will infer the crawl entrypoint and mirror it.
              </div>
            </div>
          </Card>
          {attachmentsError ? <div className="settings-banner settings-banner-error">{attachmentsError}</div> : null}
        </>
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
                <select
                  className="settings-control settings-select"
                  value={resourceGovernanceMode}
                  onChange={(e) => setResourceGovernanceMode(e.target.value as ResourceGovernanceSettings["mode"])}
                  disabled={!resourceGovernanceEnabled}
                >
                  <option value="auto">Auto (recommended)</option>
                  <option value="custom">Custom</option>
                </select>
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
                  fgColor="#f5f7ff"
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
      return (
        <>
          <Card>
            <Row
              title="Enable dictation"
              description="LiveKit Inference STT streams transcription while you speak."
              control={
                <Toggle
                  checked={dictationEnabled}
                  disabled={!loaded}
                  onChange={setDictationEnabled}
                  ariaLabel="Enable dictation"
                />
              }
            />
            <Row
              title="Model"
              description="Transcription model used by the provider."
              control={
                <select
                  className="settings-control settings-select"
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  disabled={!dictationEnabled}
                >
                  {MODEL_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              }
            />
            <Row
              title="Language"
              description="BCP-47 code (e.g. en, es, multi)."
              control={
                <input
                  className="settings-control"
                  value={language}
                  onChange={(e) => setLanguage(e.target.value)}
                  disabled={!dictationEnabled}
                  placeholder="en"
                />
              }
            />
            <Row
              title="Inference base URL"
              description="LiveKit Agent Gateway endpoint."
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  disabled={!dictationEnabled}
                  placeholder="https://agent-gateway.livekit.cloud/v1"
                />
              }
            />
            <Row
              title="LiveKit API key"
              description="Stored locally in your ctx data dir."
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  disabled={!dictationEnabled}
                  placeholder="APIK…"
                />
              }
            />
            <Row
              title="LiveKit API secret"
              description={apiSecretSet ? "Secret is stored; enter a new value to rotate." : "Required."}
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={apiSecret}
                  onChange={(e) => setApiSecret(e.target.value)}
                  disabled={!dictationEnabled}
                  placeholder={apiSecretSet ? "(set)" : "MAB…"}
                  type="password"
                />
              }
            />
          </Card>
          {!dictationCanSave && dictationEnabled ? (
            <div className="settings-banner settings-banner-error">Enter an API key and secret to enable dictation.</div>
          ) : null}
        </>
      );
    }

    if (active === "title_generation") {
      return (
        <>
          <Card>
            <Row
              title="Base URL"
              description="OpenAI-compatible endpoint for title generation (best-effort; falls back to truncating the prompt)."
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={titleGenBaseUrl}
                  onChange={(e) => setTitleGenBaseUrl(e.target.value)}
                  placeholder="https://openrouter.ai/api/v1"
                />
              }
            />
            <Row
              title="API key"
              description="Stored locally in your ctx data dir."
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={titleGenApiKey}
                  onChange={(e) => setTitleGenApiKey(e.target.value)}
                  placeholder="sk-..."
                  type="password"
                />
              }
            />
            <Row
              title="Model"
              description="Model used for generating session titles."
              control={
                <input
                  className="settings-control settings-control-wide"
                  value={titleGenModel}
                  onChange={(e) => setTitleGenModel(e.target.value)}
                  placeholder="google/gemini-3-flash-preview"
                />
              }
            />
            <Row
              title="Structured output (JSON)"
              description="Enable when the model supports JSON schema output."
              control={
                <Toggle
                  checked={titleGenUseJson}
                  disabled={!loaded}
                  onChange={setTitleGenUseJson}
                  ariaLabel="Structured output"
                />
              }
            />
          </Card>
        </>
      );
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
          const res = await supabase.functions.invoke("billing-checkout", {
            body: { interval, return_path: billingReturnPath },
          });
          if (res.error) throw res.error;
          const url = String((res.data as any)?.url ?? "").trim();
          if (!url) throw new Error("Checkout URL missing.");
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
      const anyWorkspace = workspaces.length > 0;
      const visibleProviders = providers.filter((p) => p.details?.ui_hidden !== "true").slice();
      const providersById = new Map<string, ProviderStatus>(visibleProviders.map((p) => [p.provider_id, p]));

      const order = new Map<string, number>(HARNESS_CATALOG.map((h, idx) => [h.id, idx]));
      const curated = HARNESS_CATALOG.filter((h) => providersById.has(h.id));
      const extras: HarnessCatalogEntry[] = visibleProviders
        .filter((p) => !order.has(p.provider_id))
        .map((p) => ({ id: p.provider_id, label: p.provider_id, logoSrc: "" }))
        .sort((a, b) => a.id.localeCompare(b.id));

      const harnesses = [...curated, ...extras];

      return (
        <>
          <Card>
            <Row
              title="Install all"
              description="Installs supported harnesses to ~/.ctx/providers/agent-servers."
              control={
                <button type="button" className="settings-btn" onClick={onInstallAll} disabled={installBusy !== null}>
                  {installBusy === "all" ? "Installing…" : "Install all"}
                </button>
              }
            />
            <Row
              title="Workspace"
              description="Used for authenticate/verify checks."
              control={
                <select
                  className="settings-control settings-select"
                  value={workspaceId ?? ""}
                  onChange={(e) => setWorkspaceId(e.target.value || null)}
                  disabled={!anyWorkspace}
                >
                  {workspaces.map((ws) => {
                    const id = idToString((ws as any).id);
                    return (
                      <option key={id} value={id}>
                        {ws.name}
                      </option>
                    );
                  })}
                </select>
              }
            />
          </Card>

          <div className="settings-card settings-harness-list">
            <div className="settings-card-rows">
              {harnesses.map((h) => {
                const id = h.id;
                const p = providersById.get(id);
                if (!p) return null;

                const installed = p.installed === true && p.health === "ok";
                const installSupported = p.details?.install_supported === "true";
                const installUi = installs[id];
                const installRunning = installUi?.state === "running" || p.details?.install_running === "true";
                const installBusyLocal = installBusy !== null || installRunning;
                const installLabel =
                  installBusyLocal && installUi?.pct !== null
                    ? `${clampPct(installUi.pct)}%`
                    : installBusyLocal
                      ? "Installing…"
                      : p.installed
                        ? "Update"
                        : "Install";

                const opts = providerOptions[id];
                const verifyStatus = String((opts as any)?.verify?.status ?? "");
                const needsAuth = Boolean(opts?.auth_required || verifyStatus === "auth_required");
                const showVerify = !needsAuth && verifyStatus !== "ok";

                const statusPill = (() => {
                  if (!opts) return null;
                  if (needsAuth) return <span className="settings-pill settings-pill-warn">Auth required</span>;
                  if (verifyStatus === "network_error") return <span className="settings-pill settings-pill-warn">Offline</span>;
                  if (verifyStatus === "error") return <span className="settings-pill settings-pill-err">Error</span>;
                  if (opts.probe_ok === false) return <span className="settings-pill settings-pill-err">Unhealthy</span>;
                  if (verifyStatus === "ok") return <span className="settings-pill settings-pill-ok">Verified</span>;
                  return null;
                })();

                return (
                  <div key={id} className={`settings-row settings-harness-row ${installed ? "" : "settings-harness-row-disabled"}`}>
                    <div className="settings-row-left">
                      <div className="settings-row-title settings-harness-title">
                        {h.logoSrc ? (
                          <img
                            className={`settings-harness-logo ${h.invertInDark ? "wb-invert" : ""}`}
                            src={h.logoSrc}
                            alt=""
                          />
                        ) : (
                          <span className="settings-harness-logo-fallback" aria-hidden="true" />
                        )}
                        <span className="settings-harness-name">{h.label}</span>
                        {statusPill ? <span className="settings-harness-status">{statusPill}</span> : null}
                      </div>
                      <div className="settings-row-desc">
                        {installed ? "Installed" : "Not installed"}
                        {p.version ? ` · ${p.version}` : ""}
                      </div>
                    </div>
                    <div className="settings-row-right settings-harness-actions">
                      {!installed ? (
                        <button
                          type="button"
                          className="settings-btn settings-btn-secondary"
                          onClick={() => onInstall(id)}
                          disabled={!installSupported || installBusyLocal}
                          style={
                            installBusyLocal && installUi?.pct !== null
                              ? ({ ["--settings-install-pct" as any]: `${clampPct(installUi.pct)}%` } as any)
                              : undefined
                          }
                          title={!installSupported ? "Install not supported yet" : "Install this harness"}
                        >
                          {installLabel}
                        </button>
                      ) : (
                        <>
                          {needsAuth ? (
                            <button
                              type="button"
                              className="settings-btn settings-btn-secondary"
                              onClick={() => onAuthenticate(id)}
                              disabled={!anyWorkspace || authBusy[id]}
                              title="Authenticate this provider"
                            >
                              {authBusy[id] ? "Auth…" : "Authenticate"}
                            </button>
                          ) : null}
                          {showVerify ? (
                            <button
                              type="button"
                              className="settings-btn settings-btn-secondary"
                              onClick={() => onVerify(id)}
                              disabled={!anyWorkspace || verifyBusy[id]}
                              title="Send a tiny prompt to confirm credentials and connectivity"
                            >
                              {verifyBusy[id] ? "Verifying…" : "Verify"}
                            </button>
                          ) : null}
                          <button
                            type="button"
                            className="settings-btn settings-btn-secondary"
                            onClick={() => ensureProviderOpts(id, { force: true }).catch(() => {})}
                            disabled={!anyWorkspace || optsBusy[id]}
                            title="Probe to refresh status"
                          >
                            {optsBusy[id] ? "Checking…" : "Check"}
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
              {harnesses.length === 0 ? <div className="settings-empty">No harnesses.</div> : null}
            </div>
          </div>

          {providerError ? <div className="settings-banner settings-banner-error">{providerError}</div> : null}
        </>
      );
    }

    if (
      active === "models_routing" ||
      active === "sandboxing" ||
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
                    onClick={() => {
                      setActive(s.id);
                      window.location.hash = s.id;
                    }}
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
                    onClick={() => {
                      setActive(s.id);
                      window.location.hash = s.id;
                    }}
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
              <div className="settings-main-sub">{anySaving ? "Saving…" : saveError ? "Not saved" : " "}</div>
            </div>

            {saveError ? <div className="settings-banner settings-banner-error">{saveError}</div> : null}
            {renderMain()}
          </div>
        </main>
      </div>
    </div>
  );
}
