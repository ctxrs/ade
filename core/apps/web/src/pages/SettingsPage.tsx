import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import {
  DictationSettings,
  InstallInfo,
  ProviderOptions,
  ProviderStatus,
  Settings,
  TelemetrySettings,
  Workspace,
  authenticateProviderForWorkspace,
  getInstall,
  getProviderOptions,
  getSettings,
  idToString,
  installAllProviders,
  installProvider,
  installStreamUrl,
  listInstallEvents,
  listProviders,
  listWorkspaces,
  updateSettings,
  verifyProviderForWorkspace,
} from "../api/client";
import {
  type DesktopEditorSettings,
  desktopGetEditorSettings,
  desktopUpdateEditorSettings,
  isDesktopApp,
} from "../utils/desktop";
import { HARNESS_CATALOG, type HarnessCatalogEntry } from "../utils/harnessCatalog";

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
  | "context_pack"
  | "dictation"
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
  { id: "context_pack", label: "Context Pack", group: "main" },
  { id: "dictation", label: "Dictation", group: "advanced" },
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

export default function SettingsPage() {
  const location = useLocation();
  const [active, setActive] = useState<SectionId>(() => sectionFromHash(window.location.hash) ?? "general");
  const [query, setQuery] = useState("");

  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const saveSeq = useRef(0);
  const telemetryHydrated = useRef(false);
  const dictationHydrated = useRef(false);

  const [telemetryEnabled, setTelemetryEnabled] = useState(true);
  const [telemetryEndpoint, setTelemetryEndpoint] = useState("");

  const [dictationEnabled, setDictationEnabled] = useState(true);
  const [model, setModel] = useState("auto");
  const [language, setLanguage] = useState("en");
  const [baseUrl, setBaseUrl] = useState("https://agent-gateway.livekit.cloud/v1");
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [apiSecretSet, setApiSecretSet] = useState(false);

  const [editorSettings, setEditorSettings] = useState<DesktopEditorSettings>({
    target: "system",
    custom_command: "",
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

  const eventSourcesRef = useRef<Record<string, EventSource>>({});
  const pollTimeoutsRef = useRef<Record<string, number>>({});

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

  const sidebarSections = useMemo(() => {
    const q = query.trim().toLowerCase();
    const all = [...SECTIONS];
    if (!q) return all;
    return all.filter((s) => s.label.toLowerCase().includes(q));
  }, [query]);

  const backLink = useMemo(() => {
    const ws = new URLSearchParams(location.search).get("ws");
    if (ws && ws.trim()) {
      const id = ws.trim();
      return { to: `/workspaces/${encodeURIComponent(id)}`, label: "← Back to Workspace" };
    }
    return { to: "/", label: "← Back to Home" };
  }, [location.search]);

  const anySaving = saving || editorSaving;

  const Main = () => {
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
          </Card>
          {editorError ? <div className="settings-banner settings-banner-error">{editorError}</div> : null}
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
              description="Stored locally in your Context data dir."
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
              description="Installs supported harnesses to ~/.context/providers/agent-servers."
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
            <Main />
          </div>
        </main>
      </div>
    </div>
  );
}
