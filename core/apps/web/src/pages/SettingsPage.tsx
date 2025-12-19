import { useEffect, useMemo, useState } from "react";
import { Link } from "react-router-dom";
import { DictationSettings, TelemetrySettings, getSettings, updateSettings } from "../api/client";

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

export default function SettingsPage() {
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const [dictationEnabled, setDictationEnabled] = useState(true);
  const [model, setModel] = useState("auto");
  const [language, setLanguage] = useState("en");
  const [baseUrl, setBaseUrl] = useState("https://agent-gateway.livekit.cloud/v1");
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [apiSecretSet, setApiSecretSet] = useState(false);
  const [telemetryEnabled, setTelemetryEnabled] = useState(false);
  const [telemetryEndpoint, setTelemetryEndpoint] = useState("");
  const [telemetrySaving, setTelemetrySaving] = useState(false);
  const [telemetryError, setTelemetryError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const s = await getSettings();
        if (cancelled) return;
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
        const t = s.telemetry ?? null;
        if (t) {
          setTelemetryEnabled(t.enabled);
          setTelemetryEndpoint(t.endpoint ?? "");
        } else {
          setTelemetryEnabled(false);
          setTelemetryEndpoint("");
        }
        setLoaded(true);
      } catch (e: any) {
        if (cancelled) return;
        setError(e?.message ?? String(e));
        setLoaded(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const onSaveTelemetry = async () => {
    setTelemetryError(null);
    setTelemetrySaving(true);
    try {
      const next: TelemetrySettings = {
        enabled: telemetryEnabled,
        endpoint: telemetryEndpoint.trim() || telemetryEndpoint,
      };
      await updateSettings({ telemetry: next });
    } catch (e: any) {
      setTelemetryError(e?.message ?? String(e));
    } finally {
      setTelemetrySaving(false);
    }
  };

  const canSave = useMemo(() => {
    if (!dictationEnabled) return true;
    if (!apiKey.trim()) return false;
    if (!apiSecretSet && !apiSecret.trim()) return false;
    return true;
  }, [dictationEnabled, apiKey, apiSecret, apiSecretSet]);

  const onSave = async () => {
    setError(null);
    setSaving(true);
    try {
      const next: DictationSettings = {
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
      await updateSettings({ dictation: next });
      setApiSecret("");
      setApiSecretSet(true);
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="page">
      <div className="header">
        <Link to="/workspaces">← Workspaces</Link>
      </div>

      <h1>Settings</h1>

      <div className="card">
        <div className="row">
          <strong>Providers / BYOK</strong>
        </div>
        <div className="muted">Configure credentials for cloud services used by Context.</div>
      </div>

      <div className="card">
        <div className="row">
          <strong>Telemetry</strong>
          <label className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
            <input
              type="checkbox"
              checked={telemetryEnabled}
              onChange={(e) => setTelemetryEnabled(e.target.checked)}
            />
            Enabled
          </label>
        </div>
        <div className="muted" style={{ marginTop: 8 }}>
          Share anonymous usage metrics (no code, prompts, or file paths). Enabled by default; disable to opt out.
        </div>
        {telemetryError && <div className="error" style={{ marginTop: 12 }}>{telemetryError}</div>}
        <div className="row" style={{ marginTop: 12 }}>
          <button type="button" onClick={onSaveTelemetry} disabled={!loaded || telemetrySaving}>
            {telemetrySaving ? "Saving…" : "Save"}
          </button>
        </div>
      </div>

      <div className="card">
        <div className="row">
          <strong>Dictation</strong>
          <label className="muted" style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
            <input
              type="checkbox"
              checked={dictationEnabled}
              onChange={(e) => setDictationEnabled(e.target.checked)}
            />
            Enabled
          </label>
        </div>

        <div className="muted" style={{ marginTop: 8 }}>
          LiveKit Inference STT streams transcription while you speak.
        </div>

        <div style={{ display: "grid", gap: 10, marginTop: 12 }}>
          <label>
            <div className="muted">Model</div>
            <select
              value={model}
              onChange={(e) => setModel(e.target.value)}
              disabled={!dictationEnabled}
              style={{ width: "100%", marginTop: 6 }}
            >
              {MODEL_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>

          <label>
            <div className="muted">Language</div>
            <input
              value={language}
              onChange={(e) => setLanguage(e.target.value)}
              disabled={!dictationEnabled}
              placeholder="en, es, multi…"
              style={{ width: "100%", marginTop: 6 }}
            />
          </label>

          <label>
            <div className="muted">Inference Base URL</div>
            <input
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              disabled={!dictationEnabled}
              placeholder="https://agent-gateway.livekit.cloud/v1"
              style={{ width: "100%", marginTop: 6 }}
            />
          </label>

          <label>
            <div className="muted">LiveKit API Key</div>
            <input
              value={apiKey}
              onChange={(e) => se