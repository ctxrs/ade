import { useEffect, useMemo, useRef, useState } from "react";
import { getSettings, updateSettings, type DictationSettings } from "../../../api/client";
import { isDesktopApp } from "../../../utils/desktop";
import { useTauriSttModelStatus } from "../../../utils/useTauriSttModelStatus";

type DictationController = {
  loaded: boolean;
  dictationEnabled: boolean;
  setDictationEnabled: (next: boolean) => void;
  dictationProvider: DictationSettings["provider"];
  setDictationProvider: (next: DictationSettings["provider"]) => void;
  model: string;
  setModel: (next: string) => void;
  language: string;
  setLanguage: (next: string) => void;
  baseUrl: string;
  setBaseUrl: (next: string) => void;
  apiKey: string;
  setApiKey: (next: string) => void;
  apiSecret: string;
  setApiSecret: (next: string) => void;
  apiSecretSet: boolean;
  dictationCanSave: boolean;
  tauriModelStatus: ReturnType<typeof useTauriSttModelStatus>["modelStatus"];
  startTauriModelDownload: () => Promise<void>;
  isDesktopApp: boolean;
  saveError: string | null;
};

const messageFromError = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return String(error);
};

export function useDictationController(enabled: boolean): DictationController {
  const [loaded, setLoaded] = useState(false);
  const hydrated = useRef(false);

  const [dictationEnabled, setDictationEnabled] = useState(true);
  const [dictationProvider, setDictationProvider] =
    useState<DictationSettings["provider"]>("livekit_inference");
  const [model, setModel] = useState("auto");
  const [language, setLanguage] = useState("en");
  const [baseUrl, setBaseUrl] = useState("https://agent-gateway.livekit.cloud/v1");
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  const [apiSecretSet, setApiSecretSet] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const { modelStatus: tauriModelStatus, startModelDownload: startTauriModelDownload } = useTauriSttModelStatus({
    provider: dictationProvider,
    language,
  });

  const dictationCanSave = useMemo(() => {
    if (!dictationEnabled) return true;
    if (dictationProvider !== "livekit_inference") return true;
    if (!apiKey.trim()) return false;
    if (!apiSecretSet && !apiSecret.trim()) return false;
    return true;
  }, [apiKey, apiSecret, apiSecretSet, dictationEnabled, dictationProvider]);

  const dictationPayload = useMemo((): DictationSettings => {
    return {
      enabled: dictationEnabled,
      provider: dictationProvider,
      livekit: {
        base_url: baseUrl.trim(),
        api_key: apiKey.trim(),
        api_secret: apiSecret.trim() ? apiSecret : null,
        model,
        language: language.trim() || "en",
      },
    };
  }, [apiKey, apiSecret, baseUrl, dictationEnabled, dictationProvider, language, model]);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    getSettings()
      .then((settings) => {
        if (cancelled) return;
        const d = settings.dictation ?? null;
        if (d) {
          const normalizeModel = (m: string): string => {
            const v = String(m || "").trim();
            if (!v || v === "auto") return "auto";
            if (v === "elevenlabs/scribe-v2-realtime") return "elevenlabs/scribe_v2_realtime";
            if (v === "deepgram/flux") return "deepgram/flux-general";
            return v;
          };

          const provider = d.provider ?? "livekit_inference";
          setDictationProvider(provider === "disabled" ? "livekit_inference" : provider);
          setDictationEnabled(d.enabled);
          setModel(normalizeModel(d.livekit?.model ?? "auto"));
          setLanguage(d.livekit?.language ?? "en");
          setBaseUrl(d.livekit?.base_url ?? "https://agent-gateway.livekit.cloud/v1");
          setApiKey(d.livekit?.api_key ?? "");
          setApiSecretSet(Boolean(d.livekit?.api_secret_set));
        }
        setLoaded(true);
      })
      .catch((error) => {
        if (cancelled) return;
        setSaveError(messageFromError(error));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !loaded) return;
    if (!hydrated.current) {
      hydrated.current = true;
      return;
    }
    if (!dictationCanSave) return;
    const timeout = window.setTimeout(() => {
      setSaveError(null);
      updateSettings({ dictation: dictationPayload })
        .then((next) => {
          if (next.dictation?.livekit?.api_secret_set) {
            setApiSecret("");
            setApiSecretSet(true);
          }
        })
        .catch((error) => {
          setSaveError(messageFromError(error));
        });
    }, 450);
    return () => window.clearTimeout(timeout);
  }, [dictationCanSave, dictationPayload, enabled, loaded]);

  return {
    loaded,
    dictationEnabled,
    setDictationEnabled,
    dictationProvider,
    setDictationProvider,
    model,
    setModel,
    language,
    setLanguage,
    baseUrl,
    setBaseUrl,
    apiKey,
    setApiKey,
    apiSecret,
    setApiSecret,
    apiSecretSet,
    dictationCanSave,
    tauriModelStatus,
    startTauriModelDownload,
    isDesktopApp: isDesktopApp(),
    saveError,
  };
}
