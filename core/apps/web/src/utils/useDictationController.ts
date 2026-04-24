import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { setBrowserStreamQueryToken } from "../api/browserStreamAuth";
import { getSettings, updateSettings } from "../api/client";
import type { DictationSettings, UpdateDictationSettingsRequest } from "../api/client";
import { getDaemonConnection, getDaemonWsUrl } from "../api/daemonConnection";
import { isDesktopApp } from "./desktop";
import { startMicPcmStream } from "./micPcmStream";
import {
  type SttApi,
  type SttAvailability,
  type SttPermission,
  type SttStateChange,
  loadSttApi,
  normalizeTauriLanguage,
} from "./tauriStt";
import { readBoolish } from "./boolish";
import { parseWsJson } from "./wsJson";
import { errorMessage } from "./errorMessage";
import { trackFeatureUsed } from "./analytics";
import { useTauriSttModelStatus, type TauriModelStatus } from "./useTauriSttModelStatus";

type DictationProvider = DictationSettings["provider"];

const DEFAULT_LIVEKIT_BASE_URL = "https://agent-gateway.livekit.cloud/v1";
const DEFAULT_LIVEKIT_MODEL = "auto";
const DEFAULT_LIVEKIT_LANGUAGE = "en";
const ONBOARDING_REOPEN_COOLDOWN_MS = 400;

export type DictationOnboardingStage = "choose" | "local_setup" | "cloud_setup";

export type DictationOnboardingCloudDraft = {
  baseUrl: string;
  apiKey: string;
  apiKeySet: boolean;
  apiSecret: string;
  apiSecretSet: boolean;
  model: string;
  language: string;
};

export type DictationOnboardingState = {
  open: boolean;
  stage: DictationOnboardingStage;
  busy: boolean;
  error: string | null;
  cloud: DictationOnboardingCloudDraft;
  localModelStatus: TauriModelStatus;
  localPendingStart: boolean;
  localOptionDisabledReason: string | null;
};

type DictationControllerOptions = {
  text: string;
  setText: (value: string) => void;
  appendSegment: (base: string, addition: string) => string;
};

type DictationController = {
  dictationRecording: boolean;
  dictationError: string | null;
  dictationDebugText: string | null;
  dictationOnboarding: DictationOnboardingState | null;
  dismissDictationOnboarding: () => void;
  backDictationOnboarding: () => void;
  chooseDictationOnboardingLocal: () => void;
  chooseDictationOnboardingCloud: () => void;
  updateDictationOnboardingCloud: (patch: Partial<DictationOnboardingCloudDraft>) => void;
  submitDictationOnboardingLocal: () => Promise<void>;
  submitDictationOnboardingCloud: () => Promise<void>;
  startDictation: () => Promise<void>;
  stopDictation: (opts?: { awaitFinal?: boolean }) => Promise<string>;
};

const asRecord = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return value as Record<string, unknown>;
};

const seedCloudDraft = (settings: DictationSettings | null | undefined): DictationOnboardingCloudDraft => {
  const livekit = settings?.livekit;
  return {
    baseUrl: String(livekit?.base_url ?? DEFAULT_LIVEKIT_BASE_URL),
    apiKey: "",
    apiKeySet: readBoolish(livekit?.api_key_set) ?? false,
    apiSecret: "",
    apiSecretSet: readBoolish(livekit?.api_secret_set) ?? false,
    model: String(livekit?.model ?? DEFAULT_LIVEKIT_MODEL),
    language: String(livekit?.language ?? DEFAULT_LIVEKIT_LANGUAGE),
  };
};

const normalizeCloudDraft = (draft: DictationOnboardingCloudDraft) => {
  const baseUrl = draft.baseUrl.trim() || DEFAULT_LIVEKIT_BASE_URL;
  const apiKey = draft.apiKey.trim();
  const apiSecret = draft.apiSecret.trim();
  const model = draft.model.trim() || DEFAULT_LIVEKIT_MODEL;
  const language = draft.language.trim() || DEFAULT_LIVEKIT_LANGUAGE;
  return { baseUrl, apiKey, apiSecret, model, language };
};

const cloudSettingsFromDraft = (draft: DictationOnboardingCloudDraft): UpdateDictationSettingsRequest => {
  const normalized = normalizeCloudDraft(draft);
  const livekit: UpdateDictationSettingsRequest["livekit"] = {
    base_url: normalized.baseUrl,
    model: normalized.model,
    language: normalized.language,
  };
  if (normalized.apiKey) {
    livekit.api_key = normalized.apiKey;
  }
  if (normalized.apiSecret) {
    livekit.api_secret = normalized.apiSecret;
  }
  return {
    enabled: true,
    provider: "livekit_inference",
    livekit,
  };
};

const localSettingsFromDraft = (draft: DictationOnboardingCloudDraft): UpdateDictationSettingsRequest => {
  const normalized = normalizeCloudDraft(draft);
  const livekit: UpdateDictationSettingsRequest["livekit"] = {
    base_url: normalized.baseUrl,
    model: normalized.model,
    language: normalized.language,
  };
  if (normalized.apiKey) {
    livekit.api_key = normalized.apiKey;
  }
  if (normalized.apiSecret) {
    livekit.api_secret = normalized.apiSecret;
  }
  return {
    enabled: true,
    provider: "tauri_stt",
    livekit,
  };
};

const runtimeSettingsFromDraft = (
  provider: DictationSettings["provider"],
  draft: DictationOnboardingCloudDraft,
): DictationSettings => {
  const normalized = normalizeCloudDraft(draft);
  return {
    enabled: true,
    provider,
    livekit: {
      base_url: normalized.baseUrl,
      api_key_set: draft.apiKeySet || Boolean(normalized.apiKey),
      api_secret_set: draft.apiSecretSet || Boolean(normalized.apiSecret),
      model: normalized.model,
      language: normalized.language,
    },
  };
};

const needsDictationOnboarding = (settings: DictationSettings | null | undefined): boolean => {
  if (!settings) return true;
  if (!settings.enabled) return true;

  const provider = settings.provider ?? "livekit_inference";
  if (provider === "disabled") return true;

  if (provider === "tauri_stt") {
    return !isDesktopApp();
  }

  if (provider === "livekit_inference") {
    const livekit = settings.livekit;
    const hasKey = readBoolish(livekit?.api_key_set) ?? false;
    const hasSecret = readBoolish(livekit?.api_secret_set) ?? false;
    return !hasKey || !hasSecret;
  }

  return true;
};

export const useDictationController = (opts: DictationControllerOptions): DictationController => {
  const { text, setText, appendSegment } = opts;
  const textRef = useRef(text);
  useEffect(() => {
    textRef.current = text;
  }, [text]);

  const [dictationSettings, setDictationSettings] = useState<DictationSettings | null>(null);
  const [dictationRecording, setDictationRecording] = useState(false);
  const [dictationError, setDictationError] = useState<string | null>(null);
  const dictationWsRef = useRef<WebSocket | null>(null);
  const dictationMicRef = useRef<{ stop: () => Promise<void> } | null>(null);
  const dictationBaseRef = useRef<string>("");
  const dictationCommittedRef = useRef<string>("");
  const dictationInterimRef = useRef<string>("");
  const dictationDebugEnabled = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("dictation_debug") === "1";
    } catch {
      return false;
    }
  }, []);
  const [dictationDebugText, setDictationDebugText] = useState<string | null>(null);
  const dictationAudioBytesRef = useRef(0);
  const dictationAudioChunksRef = useRef(0);
  const dictationReadyRef = useRef(false);
  const dictationAudioStartedRef = useRef(false);
  const dictationTranscriptMsgsRef = useRef(0);
  const dictationFinalizeWaiterRef = useRef<{ promise: Promise<void>; resolve: () => void } | null>(null);
  const dictationSuppressUpdatesRef = useRef(false);
  const dictationProviderRef = useRef<DictationProvider | null>(null);
  const dictationTauriStateRef = useRef<SttStateChange["state"]>("idle");
  const dictationTauriListenersRef = useRef<Array<() => void> | null>(null);
  const dictationTauriApiRef = useRef<SttApi | null>(null);

  const [onboardingOpen, setOnboardingOpen] = useState(false);
  const [onboardingStage, setOnboardingStage] = useState<DictationOnboardingStage>("choose");
  const [onboardingBusy, setOnboardingBusy] = useState(false);
  const [onboardingError, setOnboardingError] = useState<string | null>(null);
  const [onboardingCloud, setOnboardingCloud] = useState<DictationOnboardingCloudDraft>(() => seedCloudDraft(null));
  const [onboardingLocalPendingStart, setOnboardingLocalPendingStart] = useState(false);
  const onboardingReopenCooldownUntilRef = useRef(0);
  const onboardingOpenRef = useRef(false);
  const onboardingSubmissionIdRef = useRef(0);

  useEffect(() => {
    onboardingOpenRef.current = onboardingOpen;
  }, [onboardingOpen]);

  const localOptionDisabledReason = useMemo(
    () => (isDesktopApp() ? null : "Local model is only available in the desktop app."),
    [],
  );

  const onboardingModelProvider: DictationSettings["provider"] =
    onboardingOpen && onboardingStage === "local_setup" ? "tauri_stt" : "livekit_inference";
  const {
    modelStatus: onboardingLocalModelStatus,
    startModelDownload: startOnboardingLocalModelDownload,
  } = useTauriSttModelStatus({
    provider: onboardingModelProvider,
    language: onboardingCloud.language,
  });

  const cleanupTauriListeners = useCallback(() => {
    const listeners = dictationTauriListenersRef.current;
    if (!listeners) return;
    for (const unlisten of listeners) {
      try {
        unlisten();
      } catch {
        // ignore
      }
    }
    dictationTauriListenersRef.current = null;
  }, []);

  const resetSegments = useCallback((baseText: string) => {
    dictationBaseRef.current = baseText;
    dictationCommittedRef.current = "";
    dictationInterimRef.current = "";
    dictationAudioBytesRef.current = 0;
    dictationAudioChunksRef.current = 0;
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;
    dictationTranscriptMsgsRef.current = 0;
    dictationFinalizeWaiterRef.current = null;
    dictationSuppressUpdatesRef.current = false;
    dictationTauriStateRef.current = "idle";
  }, []);

  const updateTextFromSegments = useCallback(() => {
    if (dictationSuppressUpdatesRef.current) return;
    const next = appendSegment(
      appendSegment(dictationBaseRef.current, dictationCommittedRef.current),
      dictationInterimRef.current,
    );
    if (next !== textRef.current) {
      setText(next);
    }
  }, [appendSegment, setText]);

  const persistDictationSettings = useCallback(async (settings: UpdateDictationSettingsRequest): Promise<DictationSettings> => {
    const next = await updateSettings({ dictation: settings });
    const persisted = next.dictation ?? null;
    if (!persisted) {
      throw new Error("Daemon returned no dictation settings.");
    }
    setDictationSettings(persisted);
    return persisted;
  }, []);

  const beginOnboardingSubmission = useCallback((): number => {
    const nextId = onboardingSubmissionIdRef.current + 1;
    onboardingSubmissionIdRef.current = nextId;
    return nextId;
  }, []);

  const isOnboardingSubmissionActive = useCallback((submissionId: number): boolean => {
    return onboardingSubmissionIdRef.current === submissionId && onboardingOpenRef.current;
  }, []);

  const dismissDictationOnboarding = useCallback(() => {
    onboardingSubmissionIdRef.current += 1;
    onboardingOpenRef.current = false;
    setOnboardingOpen(false);
    setOnboardingStage("choose");
    setOnboardingBusy(false);
    setOnboardingError(null);
    setOnboardingLocalPendingStart(false);
    onboardingReopenCooldownUntilRef.current = Date.now() + ONBOARDING_REOPEN_COOLDOWN_MS;
  }, []);

  const openDictationOnboarding = useCallback(
    (settings: DictationSettings | null | undefined) => {
      if (onboardingOpen) return;
      if (Date.now() < onboardingReopenCooldownUntilRef.current) return;
      onboardingSubmissionIdRef.current += 1;
      onboardingOpenRef.current = true;
      setOnboardingCloud(seedCloudDraft(settings));
      setOnboardingStage("choose");
      setOnboardingBusy(false);
      setOnboardingError(null);
      setOnboardingLocalPendingStart(false);
      setOnboardingOpen(true);
    },
    [onboardingOpen],
  );

  const stopDictation = useCallback(async (opts?: { awaitFinal?: boolean }): Promise<string> => {
    const awaitFinal = opts?.awaitFinal === true;
    const ws = dictationWsRef.current;
    const mic = dictationMicRef.current;
    const base = dictationBaseRef.current;
    const committed = dictationCommittedRef.current;
    const interim = dictationInterimRef.current;
    const provider = dictationProviderRef.current;

    const hasActiveDictation =
      dictationRecording ||
      !!mic ||
      (ws ? ws.readyState !== WebSocket.CLOSED : false) ||
      !!dictationTauriListenersRef.current ||
      !!base ||
      !!committed ||
      !!interim;

    if (!hasActiveDictation) return textRef.current;

    setDictationRecording(false);
    dictationMicRef.current = null;

    try {
      await mic?.stop();
    } catch {
      // ignore
    }

    let finalizeWaiter = dictationFinalizeWaiterRef.current;
    if (provider === "tauri_stt" && awaitFinal && !finalizeWaiter) {
      let resolve: () => void = () => {};
      const promise = new Promise<void>((res) => {
        resolve = res;
      });
      finalizeWaiter = { promise, resolve };
      dictationFinalizeWaiterRef.current = finalizeWaiter;
    }

    if (provider === "tauri_stt") {
      const stt = dictationTauriApiRef.current ?? (await loadSttApi());
      dictationTauriApiRef.current = stt;
      try {
        await stt?.stopListening();
      } catch {
        // ignore
      }
    } else {
      if (ws && ws.readyState !== WebSocket.CLOSED) {
        if (!finalizeWaiter) {
          let resolve: () => void = () => {};
          const promise = new Promise<void>((res) => {
            resolve = res;
          });
          finalizeWaiter = { promise, resolve };
          dictationFinalizeWaiterRef.current = finalizeWaiter;
        }
      }
      try {
        ws?.send(JSON.stringify({ type: "stop" }));
      } catch {
        // ignore
      }
    }

    if (awaitFinal && finalizeWaiter) {
      await Promise.race([
        finalizeWaiter.promise,
        new Promise<void>((resolve) => window.setTimeout(resolve, 8000)),
      ]);
    }

    if (provider === "tauri_stt") {
      cleanupTauriListeners();
    }

    const next = appendSegment(
      appendSegment(dictationBaseRef.current, dictationCommittedRef.current),
      dictationInterimRef.current,
    );
    if (next !== textRef.current) {
      setText(next);
    }
    if (awaitFinal) {
      dictationSuppressUpdatesRef.current = true;
      dictationBaseRef.current = "";
      dictationCommittedRef.current = "";
      dictationInterimRef.current = "";
    }
    dictationInterimRef.current = "";
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;
    dictationProviderRef.current = null;

    return next;
  }, [appendSegment, cleanupTauriListeners, dictationRecording, setText]);

  const stopDictationRef = useRef(stopDictation);
  useEffect(() => {
    stopDictationRef.current = stopDictation;
  }, [stopDictation]);

  useEffect(() => {
    return () => {
      stopDictationRef.current().catch(() => {});
    };
  }, []);

  const startDictationWithSettings = useCallback(
    async (settings: DictationSettings): Promise<boolean> => {
      setDictationError(null);
      const provider = settings.provider ?? "livekit_inference";
      if (provider === "tauri_stt") {
        if (!isDesktopApp()) {
          setDictationError("Tauri dictation is only available in the desktop app.");
          return false;
        }
        if (dictationRecording) return true;
        if (dictationTauriListenersRef.current) return true;

        const stt = dictationTauriApiRef.current ?? (await loadSttApi());
        dictationTauriApiRef.current = stt;
        if (!stt) {
          setDictationError("Tauri dictation is unavailable. Reinstall the desktop app.");
          return false;
        }

        const availability = (await stt.isAvailable().catch(() => null)) as SttAvailability | null;
        if (!availability?.available) {
          setDictationError(
            availability?.reason ? `Dictation unavailable: ${availability.reason}` : "Dictation is unavailable.",
          );
          return false;
        }

        const permissionOk = (perm: SttPermission | null | undefined) =>
          Boolean(perm && perm.microphone === "granted" && perm.speechRecognition === "granted");
        let perm = await stt.checkPermission().catch(() => null);
        if (!permissionOk(perm)) {
          perm = await stt.requestPermission().catch(() => null);
        }
        if (!permissionOk(perm)) {
          setDictationError("Microphone or speech recognition permission denied.");
          return false;
        }

        cleanupTauriListeners();
        dictationProviderRef.current = "tauri_stt";
        resetSegments(textRef.current);

        const unlistenResult = await stt.onResult((result) => {
          const transcript = String(result?.transcript ?? "");
          dictationTranscriptMsgsRef.current += 1;
          if (result?.isFinal) {
            dictationCommittedRef.current = appendSegment(dictationCommittedRef.current, transcript);
            dictationInterimRef.current = "";
            dictationFinalizeWaiterRef.current?.resolve();
            dictationFinalizeWaiterRef.current = null;
          } else {
            dictationInterimRef.current = transcript;
          }
          updateTextFromSegments();
        });

        const unlistenState = await stt.onStateChange((event) => {
          dictationTauriStateRef.current = event.state;
        });

        const unlistenError = await stt.onError((error) => {
          setDictationError(`[${error.code}] ${error.message}`);
          dictationFinalizeWaiterRef.current?.resolve();
          dictationFinalizeWaiterRef.current = null;
          stopDictationRef.current().catch(() => {});
        });

        dictationTauriListenersRef.current = [unlistenResult, unlistenState, unlistenError];

        try {
          const language = normalizeTauriLanguage(settings.livekit?.language ?? DEFAULT_LIVEKIT_LANGUAGE);
          await stt.startListening({ language, interimResults: true, continuous: true });
          dictationReadyRef.current = true;
          setDictationRecording(true);
          return true;
        } catch (e: unknown) {
          cleanupTauriListeners();
          dictationProviderRef.current = null;
          setDictationError(errorMessage(e));
          setDictationRecording(false);
          return false;
        }
      }

      if (provider !== "livekit_inference") {
        setDictationError("Dictation is disabled. Configure it in Settings.");
        return false;
      }

      const existing = dictationWsRef.current;
      if (existing && existing.readyState !== WebSocket.CLOSED) return true;
      if (dictationRecording) return true;

      const query = new URLSearchParams();
      let wsUrl = "";
      try {
        await setBrowserStreamQueryToken(query, getDaemonConnection().authToken, {
          kind: "dictation_livekit",
        });
        wsUrl = getDaemonWsUrl("/api/dictation/livekit/stream", query);
      } catch (err: unknown) {
        setDictationError(errorMessage(err));
        return false;
      }
      const ws = new WebSocket(wsUrl);
      ws.binaryType = "arraybuffer";
      dictationWsRef.current = ws;
      dictationFinalizeWaiterRef.current = null;
      dictationSuppressUpdatesRef.current = false;
      dictationProviderRef.current = "livekit_inference";

      resetSegments(textRef.current);

      const openPromise = new Promise<void>((resolve, reject) => {
        ws.addEventListener("open", () => resolve(), { once: true });
        ws.addEventListener("error", () => reject(new Error("Failed to connect to dictation stream.")), { once: true });
      });

      ws.addEventListener("message", (ev) => {
        void parseWsJson((ev as MessageEvent).data).then((data) => {
          if (!data) return;
          const payload = asRecord(data);
          const t = String(payload.type ?? "");
          if (t === "ready") {
            dictationReadyRef.current = true;
            return;
          } else if (t === "audio_started") {
            dictationAudioStartedRef.current = true;
            return;
          } else if (t === "interim") {
            dictationTranscriptMsgsRef.current += 1;
            dictationInterimRef.current = String(payload.text ?? "");
          } else if (t === "final") {
            dictationTranscriptMsgsRef.current += 1;
            dictationCommittedRef.current = appendSegment(dictationCommittedRef.current, String(payload.text ?? ""));
            dictationInterimRef.current = "";
          } else if (t === "done") {
            dictationFinalizeWaiterRef.current?.resolve();
            dictationFinalizeWaiterRef.current = null;
            try {
              ws.close();
            } catch {
              // ignore
            }
            return;
          } else if (t === "error") {
            setDictationError(String(payload.message ?? "Dictation error"));
            dictationFinalizeWaiterRef.current?.resolve();
            dictationFinalizeWaiterRef.current = null;
            stopDictationRef.current().catch(() => {});
            return;
          } else {
            return;
          }

          updateTextFromSegments();
        });
      });

      ws.addEventListener("close", () => {
        dictationWsRef.current = null;
        setDictationRecording(false);
        dictationFinalizeWaiterRef.current?.resolve();
        dictationFinalizeWaiterRef.current = null;
        dictationProviderRef.current = null;
      });

      try {
        await openPromise;
        setDictationRecording(true);
        dictationMicRef.current = await startMicPcmStream({
          onPcmChunk: (pcm16) => {
            dictationAudioChunksRef.current += 1;
            dictationAudioBytesRef.current += pcm16.byteLength;
            if (ws.readyState === WebSocket.OPEN) ws.send(pcm16);
          },
          onError: (err) => {
            setDictationError(err.message);
            stopDictationRef.current().catch(() => {});
          },
        });
        return true;
      } catch (e: unknown) {
        setDictationError(errorMessage(e));
        try {
          ws.close();
        } catch {
          // ignore
        }
        dictationWsRef.current = null;
        setDictationRecording(false);
        dictationProviderRef.current = null;
        return false;
      }
    },
    [appendSegment, cleanupTauriListeners, dictationRecording, resetSegments, updateTextFromSegments],
  );

  const startDictation = useCallback(async () => {
    setDictationError(null);

    let settings = dictationSettings;
    try {
      const s = await getSettings();
      settings = s.dictation ?? null;
      setDictationSettings(settings);
    } catch (e: unknown) {
      if (!settings) {
        setDictationError(errorMessage(e) || "Failed to load dictation settings.");
        return;
      }
    }

    if (needsDictationOnboarding(settings)) {
      trackFeatureUsed("dictation_started", {
        provider: settings?.provider ?? "unknown",
        path: "onboarding_required",
      });
      openDictationOnboarding(settings);
      return;
    }

    trackFeatureUsed("dictation_started", {
      provider: settings?.provider ?? "unknown",
      path: "configured",
    });
    await startDictationWithSettings(settings as DictationSettings);
  }, [dictationSettings, openDictationOnboarding, startDictationWithSettings]);

  const chooseDictationOnboardingLocal = useCallback(() => {
    if (localOptionDisabledReason) {
      setOnboardingError(localOptionDisabledReason);
      return;
    }
    setOnboardingError(null);
    setOnboardingStage("local_setup");
  }, [localOptionDisabledReason]);

  const chooseDictationOnboardingCloud = useCallback(() => {
    setOnboardingError(null);
    setOnboardingStage("cloud_setup");
  }, []);

  const backDictationOnboarding = useCallback(() => {
    onboardingSubmissionIdRef.current += 1;
    setOnboardingError(null);
    setOnboardingBusy(false);
    setOnboardingLocalPendingStart(false);
    setOnboardingStage("choose");
  }, []);

  const updateDictationOnboardingCloud = useCallback((patch: Partial<DictationOnboardingCloudDraft>) => {
    setOnboardingCloud((prev) => ({ ...prev, ...patch }));
  }, []);

  const finishLocalOnboardingStart = useCallback(
    async (submissionId: number) => {
      const localSettings = localSettingsFromDraft(onboardingCloud);
      const started = await startDictationWithSettings(runtimeSettingsFromDraft("tauri_stt", onboardingCloud));
      if (!isOnboardingSubmissionActive(submissionId)) {
        if (started) {
          await stopDictationRef.current().catch(() => {});
        }
        return;
      }
      if (!started) {
        setOnboardingError("Failed to start local dictation.");
        return;
      }

      try {
        await persistDictationSettings(localSettings);
      } catch (e: unknown) {
        if (!isOnboardingSubmissionActive(submissionId)) {
          await stopDictationRef.current().catch(() => {});
          return;
        }
        await stopDictationRef.current().catch(() => {});
        setOnboardingError(errorMessage(e) || "Failed to save local dictation settings.");
        return;
      }

      if (!isOnboardingSubmissionActive(submissionId)) {
        await stopDictationRef.current().catch(() => {});
        return;
      }

      dismissDictationOnboarding();
    },
    [
      dismissDictationOnboarding,
      isOnboardingSubmissionActive,
      onboardingCloud,
      persistDictationSettings,
      startDictationWithSettings,
    ],
  );

  const submitDictationOnboardingCloud = useCallback(async () => {
    if (!onboardingOpen || onboardingBusy) return;

    const normalized = normalizeCloudDraft(onboardingCloud);
    if (!onboardingCloud.apiKeySet && !normalized.apiKey) {
      setOnboardingError("API key is required.");
      return;
    }
    if (!onboardingCloud.apiSecretSet && !normalized.apiSecret) {
      setOnboardingError("API secret is required.");
      return;
    }

    const submissionId = beginOnboardingSubmission();
    setOnboardingBusy(true);
    setOnboardingError(null);
    try {
      const persisted = await persistDictationSettings(cloudSettingsFromDraft(onboardingCloud));
      if (!isOnboardingSubmissionActive(submissionId)) return;
      const started = await startDictationWithSettings(persisted);
      if (!isOnboardingSubmissionActive(submissionId)) {
        if (started) {
          await stopDictationRef.current().catch(() => {});
        }
        return;
      }
      if (!started) {
        setOnboardingError("Failed to start cloud dictation.");
        return;
      }
      dismissDictationOnboarding();
    } catch (e: unknown) {
      if (isOnboardingSubmissionActive(submissionId)) {
        setOnboardingError(errorMessage(e) || "Failed to configure cloud dictation.");
      }
    } finally {
      if (onboardingSubmissionIdRef.current === submissionId) {
        setOnboardingBusy(false);
      }
    }
  }, [
    beginOnboardingSubmission,
    dismissDictationOnboarding,
    isOnboardingSubmissionActive,
    onboardingBusy,
    onboardingCloud,
    onboardingOpen,
    persistDictationSettings,
    startDictationWithSettings,
  ]);

  const submitDictationOnboardingLocal = useCallback(async () => {
    if (!onboardingOpen || onboardingBusy) return;
    if (localOptionDisabledReason) {
      setOnboardingError(localOptionDisabledReason);
      return;
    }
    if (onboardingLocalModelStatus.status === "checking" || onboardingLocalModelStatus.status === "idle") {
      setOnboardingError("Checking local model status. Please wait.");
      return;
    }

    const submissionId = beginOnboardingSubmission();
    setOnboardingBusy(true);
    setOnboardingError(null);

    try {
      if (onboardingLocalModelStatus.installed || onboardingLocalModelStatus.status === "ready") {
        await finishLocalOnboardingStart(submissionId);
        return;
      }

      setOnboardingLocalPendingStart(true);
      await startOnboardingLocalModelDownload();
    } catch (e: unknown) {
      if (isOnboardingSubmissionActive(submissionId)) {
        setOnboardingLocalPendingStart(false);
        setOnboardingError(errorMessage(e) || "Failed to configure local dictation.");
      }
    } finally {
      if (onboardingSubmissionIdRef.current === submissionId) {
        setOnboardingBusy(false);
      }
    }
  }, [
    beginOnboardingSubmission,
    finishLocalOnboardingStart,
    isOnboardingSubmissionActive,
    localOptionDisabledReason,
    onboardingBusy,
    onboardingLocalModelStatus.status,
    onboardingLocalModelStatus.installed,
    onboardingOpen,
    startOnboardingLocalModelDownload,
  ]);

  useEffect(() => {
    if (!onboardingOpen) return;
    if (onboardingStage !== "local_setup") return;
    if (!onboardingLocalPendingStart) return;

    if (onboardingLocalModelStatus.status === "error") {
      setOnboardingLocalPendingStart(false);
      setOnboardingError(onboardingLocalModelStatus.error ?? "Local model download failed.");
      return;
    }

    if (!onboardingLocalModelStatus.installed && onboardingLocalModelStatus.status !== "ready") {
      return;
    }

    const submissionId = beginOnboardingSubmission();
    setOnboardingLocalPendingStart(false);
    setOnboardingBusy(true);
    setOnboardingError(null);

    void (async () => {
      try {
        await finishLocalOnboardingStart(submissionId);
      } catch (e: unknown) {
        if (!isOnboardingSubmissionActive(submissionId)) return;
        setOnboardingError(errorMessage(e) || "Failed to configure local dictation.");
      } finally {
        if (onboardingSubmissionIdRef.current === submissionId) {
          setOnboardingBusy(false);
        }
      }
    })();
  }, [
    beginOnboardingSubmission,
    finishLocalOnboardingStart,
    isOnboardingSubmissionActive,
    onboardingLocalModelStatus.error,
    onboardingLocalModelStatus.installed,
    onboardingLocalModelStatus.status,
    onboardingLocalPendingStart,
    onboardingOpen,
    onboardingStage,
  ]);

  useEffect(() => {
    if (!dictationDebugEnabled) return;
    if (!dictationRecording) {
      setDictationDebugText(null);
      return;
    }
    const timer = window.setInterval(() => {
      if (dictationProviderRef.current === "tauri_stt") {
        setDictationDebugText(
          `Dictation debug\nprovider=tauri_stt state=${dictationTauriStateRef.current}\ntranscript_msgs=${dictationTranscriptMsgsRef.current}`,
        );
        return;
      }
      const ws = dictationWsRef.current;
      const wsState = ws ? ws.readyState : -1;
      setDictationDebugText(
        `Dictation debug\nws_state=${wsState} ready=${dictationReadyRef.current} audio_started=${dictationAudioStartedRef.current}\naudio_chunks=${dictationAudioChunksRef.current} audio_bytes=${dictationAudioBytesRef.current} transcript_msgs=${dictationTranscriptMsgsRef.current}`,
      );
    }, 500);
    return () => window.clearInterval(timer);
  }, [dictationDebugEnabled, dictationRecording]);

  const dictationOnboarding = useMemo<DictationOnboardingState | null>(() => {
    if (!onboardingOpen) return null;
    return {
      open: onboardingOpen,
      stage: onboardingStage,
      busy: onboardingBusy,
      error: onboardingError,
      cloud: onboardingCloud,
      localModelStatus: onboardingLocalModelStatus,
      localPendingStart: onboardingLocalPendingStart,
      localOptionDisabledReason,
    };
  }, [
    localOptionDisabledReason,
    onboardingBusy,
    onboardingCloud,
    onboardingError,
    onboardingLocalModelStatus,
    onboardingLocalPendingStart,
    onboardingOpen,
    onboardingStage,
  ]);

  return {
    dictationRecording,
    dictationError,
    dictationDebugText,
    dictationOnboarding,
    dismissDictationOnboarding,
    backDictationOnboarding,
    chooseDictationOnboardingLocal,
    chooseDictationOnboardingCloud,
    updateDictationOnboardingCloud,
    submitDictationOnboardingLocal,
    submitDictationOnboardingCloud,
    startDictation,
    stopDictation,
  };
};
