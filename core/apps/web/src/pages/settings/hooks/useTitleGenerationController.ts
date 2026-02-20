import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  getInstall,
  getSettings,
  getTitleGenerationLocalStatus,
  installTitleGenerationLocal,
  updateSettings,
  type TitleGenerationLocalStatus,
  type TitleGenerationSettings,
} from "../../../api/client";
import type { InstallSession } from "../../SettingsPage.types";
import { clampPct } from "../../SettingsPage.utils";

type TitleGenerationController = {
  loaded: boolean;
  titleGenMode: TitleGenerationSettings["mode"];
  setTitleGenMode: (mode: TitleGenerationSettings["mode"]) => void;
  titleGenBaseUrl: string;
  setTitleGenBaseUrl: (value: string) => void;
  titleGenApiKey: string;
  setTitleGenApiKey: (value: string) => void;
  titleGenModel: string;
  setTitleGenModel: (value: string) => void;
  titleGenUseJson: boolean;
  setTitleGenUseJson: (next: boolean) => void;
  titleGenLocalModelId: string;
  setTitleGenLocalModelId: (value: string) => void;
  titleGenLocalUseJson: boolean;
  setTitleGenLocalUseJson: (next: boolean) => void;
  titleGenLocalStatus: TitleGenerationLocalStatus | null;
  titleGenLocalStatusBusy: boolean;
  titleGenLocalStatusError: string | null;
  titleGenLocalInstallBusy: boolean;
  localInstall: InstallSession | undefined;
  onInstallTitleGenerationLocal: () => Promise<void>;
};

const messageFromError = (error: unknown): string => {
  if (error instanceof Error && error.message) {
    return error.message;
  }
  return String(error);
};

export function useTitleGenerationController(enabled: boolean): TitleGenerationController {
  const [loaded, setLoaded] = useState(false);
  const hydrated = useRef(false);

  const [titleGenMode, setTitleGenMode] = useState<TitleGenerationSettings["mode"]>("remote");
  const [titleGenBaseUrl, setTitleGenBaseUrl] = useState("");
  const [titleGenApiKey, setTitleGenApiKey] = useState("");
  const [titleGenModel, setTitleGenModel] = useState("");
  const [titleGenUseJson, setTitleGenUseJson] = useState(true);
  const [titleGenLocalModelId, setTitleGenLocalModelId] = useState("ggml-org/Qwen3-1.7B-GGUF");
  const [titleGenLocalUseJson, setTitleGenLocalUseJson] = useState(true);

  const [titleGenLocalStatus, setTitleGenLocalStatus] = useState<TitleGenerationLocalStatus | null>(null);
  const [titleGenLocalStatusBusy, setTitleGenLocalStatusBusy] = useState(false);
  const [titleGenLocalStatusError, setTitleGenLocalStatusError] = useState<string | null>(null);
  const [titleGenLocalInstallBusy, setTitleGenLocalInstallBusy] = useState(false);
  const [localInstall, setLocalInstall] = useState<InstallSession | undefined>(undefined);

  const installPollRef = useRef<number | null>(null);

  const titleGenerationPayload = useMemo((): TitleGenerationSettings => {
    return {
      mode: titleGenMode,
      remote: {
        base_url: titleGenBaseUrl.trim(),
        api_key: titleGenApiKey.trim(),
        model: titleGenModel.trim(),
        use_json: titleGenUseJson,
      },
      local: {
        model_id: titleGenLocalModelId.trim(),
        use_json: titleGenLocalUseJson,
      },
    };
  }, [
    titleGenApiKey,
    titleGenBaseUrl,
    titleGenLocalModelId,
    titleGenLocalUseJson,
    titleGenMode,
    titleGenModel,
    titleGenUseJson,
  ]);

  const refreshTitleGenLocalStatus = useCallback(async (opts?: { silent?: boolean }) => {
    if (!opts?.silent) {
      setTitleGenLocalStatusBusy(true);
    }
    setTitleGenLocalStatusError(null);
    try {
      const status = await getTitleGenerationLocalStatus();
      setTitleGenLocalStatus(status);
      return status;
    } catch (error) {
      setTitleGenLocalStatusError(messageFromError(error));
      return null;
    } finally {
      if (!opts?.silent) {
        setTitleGenLocalStatusBusy(false);
      }
    }
  }, []);

  const attachInstall = useCallback(
    async (installId: string) => {
      if (!installId) return;
      if (installPollRef.current) {
        window.clearTimeout(installPollRef.current);
        installPollRef.current = null;
      }

      setLocalInstall((prev) => ({
        installId,
        state: "running",
        pct: prev?.pct ?? null,
        streamError: prev?.streamError,
        error: prev?.error,
      }));

      const poll = async () => {
        try {
          const info = await getInstall(installId);
          const pct =
            typeof info.last_event?.bytes === "number"
            && typeof info.last_event?.total_bytes === "number"
            && info.last_event.total_bytes > 0
              ? clampPct(Math.round((info.last_event.bytes / info.last_event.total_bytes) * 100))
              : null;
          setLocalInstall({
            installId,
            state: info.state,
            pct,
            error: info.error,
          });
          if (info.state !== "running") {
            installPollRef.current = null;
            await refreshTitleGenLocalStatus({ silent: true });
            return;
          }
        } catch {
          // continue polling; transient fetch failures should not terminate the install status loop
        }
        installPollRef.current = window.setTimeout(() => {
          poll().catch(() => {});
        }, 900);
      };

      await poll();
    },
    [refreshTitleGenLocalStatus],
  );

  const onInstallTitleGenerationLocal = useCallback(async () => {
    setTitleGenLocalInstallBusy(true);
    setTitleGenLocalStatusError(null);
    try {
      const { install_id } = await installTitleGenerationLocal();
      await attachInstall(install_id);
    } catch (error) {
      setTitleGenLocalStatusError(messageFromError(error));
    } finally {
      setTitleGenLocalInstallBusy(false);
    }
  }, [attachInstall]);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    setLoaded(false);
    getSettings()
      .then((settings) => {
        if (cancelled) return;
        const tg = settings.title_generation ?? null;
        if (tg) {
          setTitleGenMode(tg.mode ?? "remote");
          setTitleGenBaseUrl(tg.remote?.base_url ?? "");
          setTitleGenApiKey(tg.remote?.api_key ?? "");
          setTitleGenModel(tg.remote?.model ?? "");
          setTitleGenUseJson(Boolean(tg.remote?.use_json));
          setTitleGenLocalModelId(tg.local?.model_id ?? "ggml-org/Qwen3-1.7B-GGUF");
          setTitleGenLocalUseJson(Boolean(tg.local?.use_json));
        }
        setLoaded(true);
      })
      .catch(() => {
        if (cancelled) return;
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
    const timeout = window.setTimeout(() => {
      updateSettings({ title_generation: titleGenerationPayload }).catch(() => {});
    }, 450);
    return () => window.clearTimeout(timeout);
  }, [enabled, loaded, titleGenerationPayload]);

  useEffect(() => {
    if (!enabled) return;
    if (titleGenMode !== "local") return;
    refreshTitleGenLocalStatus().then((status) => {
      if (status?.install_running && status.install_id) {
        attachInstall(status.install_id).catch(() => {});
      }
    }).catch(() => {});
  }, [attachInstall, enabled, refreshTitleGenLocalStatus, titleGenMode]);

  useEffect(() => {
    return () => {
      if (installPollRef.current) {
        window.clearTimeout(installPollRef.current);
      }
    };
  }, []);

  return {
    loaded,
    titleGenMode,
    setTitleGenMode,
    titleGenBaseUrl,
    setTitleGenBaseUrl,
    titleGenApiKey,
    setTitleGenApiKey,
    titleGenModel,
    setTitleGenModel,
    titleGenUseJson,
    setTitleGenUseJson,
    titleGenLocalModelId,
    setTitleGenLocalModelId,
    titleGenLocalUseJson,
    setTitleGenLocalUseJson,
    titleGenLocalStatus,
    titleGenLocalStatusBusy,
    titleGenLocalStatusError,
    titleGenLocalInstallBusy,
    localInstall,
    onInstallTitleGenerationLocal,
  };
}
