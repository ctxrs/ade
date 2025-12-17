import { forwardRef, useCallback, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { GroupedVirtuoso, GroupedVirtuosoHandle, Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Link, useParams } from "react-router-dom";
import {
  cancelSession,
  deleteMessage,
  DictationSettings,
  getDaemonBaseUrl,
  blobUrl,
  Message,
  MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  setSessionMode,
  setSessionModel,
  authenticateSession,
  getSettings,
  idToString,
  interruptSession,
  uploadBlob,
} from "../api/client";
import { useOpenSession, useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { DiffReviewPane } from "../components/DiffReviewPane";
import { ComposerAutocompleteMenu } from "../components/ComposerAutocompleteMenu";
import { useComposerAutocomplete, type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import { shouldSendOnEnter } from "../utils/keyboard";
import { HARNESS_CATALOG } from "../utils/harnessCatalog";
import { WorkbenchComposer as UnifiedWorkbenchComposer, type WorkbenchModeId } from "../components/WorkbenchComposer";
import { startMicPcmStream } from "../utils/micPcmStream";
import { parseWsJson } from "../utils/wsJson";
import { buildModelCatalog, composeModelId, formatEffortLabel, parseModelId } from "../utils/modelEffort";
import { imageFilesToBlobRefAttachments, imageFilesToInlineAttachments } from "../utils/messageAttachments";
import { registerDropScope } from "../utils/dragDropScopes";

type ThreadItem =
  | {
      kind: "message";
      id: string;
      role: "user" | "assistant";
      content: string;
      attachments: MessageAttachment[];
      created_at: string;
      delivery?: "immediate" | "queued";
    }
  | {
      kind: "spacer";
      id: string;
      created_at: string;
    }
  | {
      kind: "assistant";
      id: string;
      created_at: string;
      content: string;
      thought: string;
      is_complete: boolean;
      thought_seconds?: number;
    }
  | {
      kind: "tool";
      id: string;
      created_at: string;
      updated_at: string;
      tool_call_id: string;
      tool_kind: string;
      title: string;
      status: string;
      locations: Array<{ path?: string; range?: any }>;
      input: any;
      output_text: string;
      raw: any;
      updates_seen: number;
    };

type WorkbenchTurnHeader = {
  id: string;
  content: string;
  attachments: MessageAttachment[];
  created_at: string;
};

function imageAttachmentSrc(a: MessageAttachment): string {
  return a.kind === "image_ref" ? blobUrl(a.blob_id) : `data:${a.mime_type};base64,${a.data_base64}`;
}

function attachmentDisplayName(name?: string | null) {
  const n = String(name ?? "").trim();
  if (!n) return "image";
  return n.split(/[\\/]/).pop() || "image";
}

function appendSegment(base: string, addition: string): string {
  const trimmed = addition.trim();
  if (!trimmed) return base;
  if (!base) return trimmed;
  const needsSpace = /\S$/.test(base) && !/^[,.;!?]/.test(trimmed);
  return `${base}${needsSpace ? " " : ""}${trimmed}`;
}

type WorkbenchThreadView = {
  groups: Array<{
    key: string;
    header: WorkbenchTurnHeader | null;
    items: ThreadItem[];
  }>;
  debugEvents: SessionEvent[];
};

export default function SessionPage() {
  const { id } = useParams<{ id: string }>();
  if (!id) return null;
  return <SessionView sessionId={id} variant="legacy" showDiffPane />;
}

export type SessionViewVariant = "legacy" | "workbench";

export function SessionView({
  sessionId,
  variant,
  showDiffPane,
}: {
  sessionId: string;
  variant: SessionViewVariant;
  showDiffPane: boolean;
}) {
  const id = sessionId;
  const supervisor = useSessionSupervisor();
  const supervisorSnap = useSessionCacheSnapshot();
  const showDebug = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("debug") === "1";
    } catch {
      return false;
    }
  }, [id]);
  const perfEnabled = useMemo(() => {
    try {
      return new URLSearchParams(window.location.search).get("perf") === "1";
    } catch {
      return false;
    }
  }, [id]);
  const perfStartRef = useRef<number>(0);
  const [input, setInput] = useState("");
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [dropActive, setDropActive] = useState(false);
  const [workbenchMode, setWorkbenchMode] = useState<WorkbenchModeId>("default");
  const [atBottom, setAtBottom] = useState(true);
  const [hasNewActivity, setHasNewActivity] = useState(false);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [expandedTurnHeaders, setExpandedTurnHeaders] = useState<Record<string, boolean>>({});
  const [expandedThoughtByAssistantId, setExpandedThoughtByAssistantId] = useState<Record<string, boolean>>({});
  const [expandedToolById, setExpandedToolById] = useState<Record<string, boolean>>({});
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const groupedVirtuosoRef = useRef<GroupedVirtuosoHandle>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const didInitialScrollRef = useRef(false);
  const dropHideTimerRef = useRef<number | null>(null);
  useOpenSession(id ?? "", { watchDiff: true });

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

  const entry = useSessionEntry(id ?? "");
  const session: Session | null = entry?.session ?? null;
  const events: SessionEvent[] = entry?.events ?? [];
  const messages: Message[] = entry?.messages ?? [];
  const queue: Message[] = entry?.queue ?? [];
  const diff = entry?.diff ?? "";
  const eventsKey = `${entry?.lastEventId ?? ""}:${events.length}`;
  const streamConnected = supervisorSnap.connection === "connected";

  const interruptBanner = useMemo(() => {
    const last = [...events].reverse().find((e) => e.event_type === "turn_interrupted");
    if (!last) return null;
    return `Interrupted at ${new Date(last.created_at).toLocaleTimeString()}.`;
  }, [eventsKey]);

  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((s) => {
        if (cancelled) return;
        setDictationSettings(s.dictation ?? null);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  const stopDictation = useCallback(async (): Promise<string> => {
    setDictationRecording(false);

    const ws = dictationWsRef.current;
    dictationWsRef.current = null;

    const mic = dictationMicRef.current;
    dictationMicRef.current = null;

    try {
      await mic?.stop();
    } catch {}

    try {
      ws?.send(JSON.stringify({ type: "stop" }));
    } catch {}

    try {
      ws?.close(1000, "client stop");
    } catch {}

    const base = dictationBaseRef.current;
    const committed = dictationCommittedRef.current;
    const interim = dictationInterimRef.current;
    const next = appendSegment(appendSegment(base, committed), interim);
    setInput(next);
    dictationInterimRef.current = "";
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;

    return next;
  }, []);

  const startDictation = useCallback(async () => {
    setDictationError(null);

    const enabled =
      Boolean(dictationSettings?.enabled) && dictationSettings?.provider === "livekit_inference";
    if (!enabled) {
      setDictationError("Dictation is disabled. Configure it in Settings.");
      return;
    }
    const existing = dictationWsRef.current;
    if (existing && existing.readyState !== WebSocket.CLOSED) return;
    if (dictationRecording) return;

    const token = (() => {
      try {
        return sessionStorage.getItem("contextAuthToken");
      } catch {
        return null;
      }
    })();
    const base = getDaemonBaseUrl();
    const wsBase = base
      ? base.startsWith("https://")
        ? base.replace(/^https:\/\//, "wss://")
        : base.replace(/^http:\/\//, "ws://")
      : `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}`;
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const ws = new WebSocket(`${wsBase}/api/dictation/livekit/stream${qs}`);
    ws.binaryType = "arraybuffer";
    dictationWsRef.current = ws;

    dictationBaseRef.current = input;
    dictationCommittedRef.current = "";
    dictationInterimRef.current = "";
    dictationAudioBytesRef.current = 0;
    dictationAudioChunksRef.current = 0;
    dictationReadyRef.current = false;
    dictationAudioStartedRef.current = false;
    dictationTranscriptMsgsRef.current = 0;

    const openPromise = new Promise<void>((resolve, reject) => {
      let settled = false;
      const settleResolve = () => {
        if (settled) return;
        settled = true;
        resolve();
      };
      const settleReject = (err: Error) => {
        if (settled) return;
        settled = true;
        reject(err);
      };

      ws.addEventListener("open", settleResolve, { once: true });
      ws.addEventListener("error", () => settleReject(new Error("Failed to connect to dictation stream.")), { once: true });
      ws.addEventListener("close", () => settleReject(new Error("Dictation stream closed before connecting.")), { once: true });
    });

    ws.addEventListener("message", (ev) => {
      void parseWsJson((ev as MessageEvent).data).then((data) => {
        if (dictationWsRef.current !== ws) return;
        if (!data) return;
        const t = String(data.type ?? "");
        if (t === "ready") {
          dictationReadyRef.current = true;
          return;
        } else if (t === "audio_started") {
          dictationAudioStartedRef.current = true;
          return;
        } else if (t === "interim") {
          dictationTranscriptMsgsRef.current += 1;
          dictationInterimRef.current = String(data.text ?? "");
        } else if (t === "final") {
          dictationTranscriptMsgsRef.current += 1;
          dictationCommittedRef.current = appendSegment(dictationCommittedRef.current, String(data.text ?? ""));
          dictationInterimRef.current = "";
        } else if (t === "done") {
          try {
            ws.close();
          } catch {}
          return;
        } else if (t === "error") {
          setDictationError(String(data.message ?? "Dictation error"));
          stopDictation().catch(() => {});
          return;
        } else {
          return;
        }

        const base = dictationBaseRef.current;
        const committed = dictationCommittedRef.current;
        const interim = dictationInterimRef.current;
        setInput(appendSegment(appendSegment(base, committed), interim));
      });
    });

    ws.addEventListener("close", () => {
      if (dictationWsRef.current !== ws) return;
      dictationWsRef.current = null;
      setDictationRecording(false);
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
          stopDictation().catch(() => {});
        },
      });
    } catch (e: any) {
      if (dictationWsRef.current === ws) setDictationError(e?.message ?? String(e));
      try {
        ws.close();
      } catch {}
      if (dictationWsRef.current === ws) {
        dictationWsRef.current = null;
        setDictationRecording(false);
      }
    }
  }, [dictationSettings, dictationRecording, input, stopDictation]);

  useEffect(() => {
    return () => {
      stopDictation().catch(() => {});
    };
  }, [stopDictation]);

  useEffect(() => {
    if (!dictationDebugEnabled) return;
    if (!dictationRecording) {
      setDictationDebugText(null);
      return;
    }
    const timer = window.setInterval(() => {
      const ws = dictationWsRef.current;
      const wsState = ws ? ws.readyState : -1;
      setDictationDebugText(
        `Dictation debug\nws_state=${wsState} ready=${dictationReadyRef.current} audio_started=${dictationAudioStartedRef.current}\naudio_chunks=${dictationAudioChunksRef.current} audio_bytes=${dictationAudioBytesRef.current} transcript_msgs=${dictationTranscriptMsgsRef.current}`,
      );
    }, 500);
    return () => window.clearInterval(timer);
  }, [dictationDebugEnabled, dictationRecording]);

  useEffect(() => {
    if (!perfEnabled) return;
    perfStartRef.current = performance.now();
  }, [id, perfEnabled]);

  useEffect(() => {
    if (!perfEnabled) return;
    if (!perfStartRef.current) return;
    if (!entry) return;
    if (entry.loading) return;
    // eslint-disable-next-line no-console
    console.log(
      `[perf] session_ready_ms=${(performance.now() - perfStartRef.current).toFixed(1)} events=${entry.events.length} diff_bytes=${(entry.diff ?? "").length}`,
    );
    perfStartRef.current = 0;
  }, [perfEnabled, entry?.loading, entry?.events.length, entry?.diff]);

  const legacyThreadView = useMemo(() => buildThreadViewModel(events), [eventsKey]);
  const workbenchThreadView = useMemo(
    () => buildWorkbenchThreadViewModel(events, messages),
    // messages are canonical for turn headers; include in memo key
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [eventsKey, messages.length],
  );

  const debugEvents = variant === "workbench" ? workbenchThreadView.debugEvents : legacyThreadView.debugEvents;
  const threadItems = variant === "workbench" ? [] : legacyThreadView.items;
  const wbGroups = variant === "workbench" ? workbenchThreadView.groups : [];
  const wbGroupCounts = useMemo(() => wbGroups.map((g) => g.items.length), [wbGroups]);
  const wbFlatItems = useMemo(() => wbGroups.flatMap((g) => g.items), [wbGroups]);

  useEffect(() => {
    if (didInitialScrollRef.current) return;
    if (variant === "workbench") {
      if (wbFlatItems.length === 0) return;
      groupedVirtuosoRef.current?.scrollToIndex({ index: "LAST", align: "end" });
    } else {
      if (threadItems.length === 0) return;
      virtuosoRef.current?.scrollToIndex({ index: threadItems.length - 1, align: "end" });
    }
    didInitialScrollRef.current = true;
  }, [variant, wbFlatItems.length, threadItems.length]);

  const contextIndicator = useMemo(() => {
    const done = [...events]
      .reverse()
      .find((e) => e.event_type === "done" && e.payload_json?.context_window);
    if (!done) return null;
    return done.payload_json.context_window as {
      context_tokens_estimate: number;
      remaining_fraction: number;
    };
  }, [eventsKey]);

  const planEntries = useMemo(() => {
    const last = [...events].reverse().find((e) => e.event_type === "plan");
    const update = last?.payload_json?.acp_update ?? last?.payload_json;
    const entries = update?.entries ?? [];
    return Array.isArray(entries) ? (entries as any[]) : [];
  }, [eventsKey]);

  const authUi = useMemo(() => deriveAuthUi(events), [eventsKey]);

  useEffect(() => {
    if (authMethodId) return;
    if (authUi.methods.length > 0) {
      setAuthMethodId(authUi.methods[0].id);
    }
  }, [authUi.methods, authMethodId]);

  const acpSessionInfo = useMemo(() => {
    return [...events]
      .reverse()
      .find((e) => e.event_type === "init" && (e.payload_json?.models || e.payload_json?.modes));
  }, [eventsKey]);

  const acpModels = acpSessionInfo?.payload_json?.models;
  const acpModes = acpSessionInfo?.payload_json?.modes;

  const modelOptions = useMemo(() => {
    const list =
      acpModels?.availableModels ??
      acpModels?.available_models ??
      acpModels?.available_models ??
      [];
    if (!Array.isArray(list)) return [];
    return list
      .map((m: any) => ({
        id: m.modelId ?? m.model_id ?? m.id,
        name: m.name ?? (m.modelId ?? m.model_id ?? m.id),
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0);
  }, [acpModels]);

  const modeOptions = useMemo(() => {
    const list =
      acpModes?.availableModes ??
      acpModes?.available_modes ??
      acpModes?.available_modes ??
      [];
    if (!Array.isArray(list)) return [];
    return list
      .map((m: any) => ({
        id: m.id,
        name: m.name ?? m.id,
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0);
  }, [acpModes]);

  const currentModelId =
    session?.model_id ??
    acpModels?.currentModelId ??
    acpModels?.current_model_id ??
    "";
  const currentModeId =
    acpModes?.currentModeId ??
    acpModes?.current_mode_id ??
    "";

  const modelCatalog = useMemo(() => buildModelCatalog(modelOptions), [modelOptions]);
  const parsedModel = useMemo(() => parseModelId(currentModelId, modelCatalog), [currentModelId, modelCatalog]);
  const currentBase = parsedModel.base || modelCatalog.baseIds[0] || "";
  const currentEffort = parsedModel.effort;
  const effortOptions = modelCatalog.effortsByBase[currentBase] ?? [];

  const pickDefaultEffort = useCallback((efforts: string[]) => {
    if (efforts.includes("medium")) return "medium";
    return efforts[0] ?? null;
  }, []);

  const deriveFullModelIdForBase = useCallback(
    (base: string, preferredEffort: string | null) => {
      const efforts = modelCatalog.effortsByBase[base] ?? [];
      if (efforts.length === 0) return base;
      const eff = preferredEffort && efforts.includes(preferredEffort) ? preferredEffort : pickDefaultEffort(efforts);
      if (!eff) return base;
      return modelCatalog.fullIdByBaseEffort[base]?.[eff] ?? composeModelId(base, eff);
    },
    [modelCatalog, pickDefaultEffort],
  );

  const threadActivityCount = variant === "workbench" ? wbFlatItems.length : threadItems.length;

  useEffect(() => {
    if (!atBottom && threadActivityCount > 0) {
      setHasNewActivity(true);
    }
  }, [threadActivityCount, atBottom]);

  const sendNow = async () => {
    if (!id) return;
    const text = (dictationRecording ? await stopDictation() : input).trim();
    if (!text) return;
    await postMessage(id, text, undefined, draftAttachments);
    setInput("");
    setDraftAttachments([]);
    supervisor.refreshQueue(id);
    supervisor.refreshSession(id, { watchDiff: true });
  };

  const onSend = async (e: React.FormEvent) => {
    e.preventDefault();
    await sendNow();
  };

  const onRemoveQueued = async (messageId: string) => {
    await deleteMessage(messageId);
    supervisor.refreshQueue(id ?? "");
  };

  const insertIntoComposer = (text: string) => {
    const el = textareaRef.current;
    if (!el) {
      setInput((v) => v + text);
      return;
    }
    const start = el.selectionStart ?? input.length;
    const end = el.selectionEnd ?? input.length;
    const next = input.slice(0, start) + text + input.slice(end);
    setInput(next);
    requestAnimationFrame(() => {
      el.focus();
      const cursor = start + text.length;
      el.setSelectionRange(cursor, cursor);
      requestAnimationFrame(() => composerAutocomplete.syncFromDom());
    });
  };

  const acpAvailableCommands = useMemo<SlashCommandDescriptor[]>(() => {
    const last = [...events].reverse().find((e) => {
      const update = e.payload_json?.acp_update;
      return update?.sessionUpdate === "available_commands_update";
    });
    const update = last?.payload_json?.acp_update ?? {};
    const list = update.availableCommands ?? update.available_commands ?? [];
    if (!Array.isArray(list)) return [];
    return list
      .map((c: any) => ({
        name: String(c?.name ?? "").replace(/^\//, ""),
        description: typeof c?.description === "string" ? c.description : undefined,
      }))
      .filter((c: any) => typeof c.name === "string" && c.name.length > 0);
  }, [eventsKey]);

  const fallbackSlashCommands = useMemo<SlashCommandDescriptor[]>(() => {
    const provider = session?.provider_id;
    if (provider === "codex") {
      return [
        { name: "review", description: "Review my current changes and find issues" },
        { name: "review-branch", description: "Review a branch" },
        { name: "review-commit", description: "Review a commit" },
        { name: "init", description: "Create an AGENTS.md file" },
        { name: "compact", description: "Summarize conversation to save context" },
        { name: "logout", description: "Log out" },
      ];
    }
    if (provider === "claude") {
      return [
        { name: "login", description: "Log in" },
        { name: "logout", description: "Log out" },
        { name: "compact", description: "Summarize conversation to save context" },
        { name: "help", description: "Show help" },
      ];
    }
    return [{ name: "compact", description: "Summarize conversation to save context" }];
  }, [session?.provider_id]);

  const slashCommands = acpAvailableCommands.length > 0 ? acpAvailableCommands : fallbackSlashCommands;

  const composerAutocomplete = useComposerAutocomplete({
    sessionId: id ?? null,
    workspaceId: null,
    value: input,
    setValue: setInput,
    textareaRef,
    slashCommands,
  });

  const virtuosoStyle =
    variant === "workbench" ? ({ flex: 1, minHeight: 0 } as const) : ({ height: "70vh" } as const);

  const wrapperClass = variant === "workbench" ? "wb-session-view" : "page split";
  const leftClass = variant === "workbench" ? "wb-session-left" : "left";

  const dropScopeRef = useRef<HTMLDivElement | null>(null);

  const onDropFiles = useCallback(
    async (files: File[]) => {
      if (files.length === 0) return;
      const next =
        variant === "workbench"
          ? await imageFilesToInlineAttachments(files)
          : await imageFilesToBlobRefAttachments(files);
      if (next.length === 0) return;
      setDraftAttachments((prev) => [...prev, ...next]);
    },
    [variant],
  );

  const showDropOverlay = useCallback(() => {
    setDropActive(true);
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = window.setTimeout(() => setDropActive(false), 140);
  }, []);

  const hideDropOverlay = useCallback(() => {
    if (dropHideTimerRef.current) window.clearTimeout(dropHideTimerRef.current);
    dropHideTimerRef.current = null;
    setDropActive(false);
  }, []);

  const extractFilesFromTransfer = useCallback(
    (dt: DataTransfer | null): File[] => {
      if (!dt) return [];
      const out: File[] = [];
      const files = dt.files ? Array.from(dt.files) : [];
      out.push(...files);
      const items = dt.items;
      if (out.length === 0 && items && items.length > 0) {
        for (const item of Array.from(items as any) as DataTransferItem[]) {
          if (item.kind !== "file") continue;
          const f = item.getAsFile?.();
          if (f) out.push(f);
        }
      }
      return out;
    },
    [],
  );

  const extractFirstUrlFromTransfer = useCallback((dt: DataTransfer | null): string | null => {
    if (!dt) return null;
    const uriRaw = (dt.getData?.("text/uri-list") ?? "").trim();
    if (uriRaw) {
      for (const line of uriRaw.split("\n")) {
        const v = line.trim();
        if (!v || v.startsWith("#")) continue;
        return v;
      }
    }
    const html = (dt.getData?.("text/html") ?? "").trim();
    if (html) {
      const m = html.match(/<img[^>]*\ssrc=("([^"]+)"|'([^']+)'|([^\s>]+))/i);
      const src = (m?.[2] ?? m?.[3] ?? m?.[4] ?? "").trim();
      if (src) return src;
    }
    const text = (dt.getData?.("text/plain") ?? "").trim();
    if (text && /^(https?:|data:image\/|blob:)/i.test(text)) return text;
    return null;
  }, []);

  const urlToImageFile = useCallback(async (url: string): Promise<File | null> => {
    try {
      const res = await fetch(url);
      if (!res.ok) return null;
      const blob = await res.blob();
      const type = blob.type || "";
      if (!type.startsWith("image/")) return null;
      const baseName = (() => {
        try {
          const u = new URL(url, window.location.href);
          const last = u.pathname.split("/").filter(Boolean).pop() || "image";
          return last.replace(/[?#].*$/, "") || "image";
        } catch {
          return "image";
        }
      })();
      const ext = type.split("/")[1] || "";
      const name = ext && !baseName.toLowerCase().endsWith(`.${ext.toLowerCase()}`) ? `${baseName}.${ext}` : baseName;
      return new File([blob], name, { type });
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    const el = dropScopeRef.current;
    if (!el) return;
    return registerDropScope({
      element: el,
      onDragOver: () => showDropOverlay(),
      onDrop: (dt) => {
        hideDropOverlay();
        void (async () => {
          const files = extractFilesFromTransfer(dt);
          if (files.length > 0) {
            await onDropFiles(files);
            return;
          }
          const url = extractFirstUrlFromTransfer(dt);
          if (!url) return;
          const asFile = await urlToImageFile(url);
          if (!asFile) return;
      await onDropFiles([asFile]);
        })();
      },
    });
  }, [extractFilesFromTransfer, extractFirstUrlFromTransfer, hideDropOverlay, onDropFiles, showDropOverlay, urlToImageFile]);

  const renderThreadItem = (item: ThreadItem) => {
    if (item.kind === "spacer") {
      return <div style={{ height: 1 }} />;
    }
    if (item.kind === "assistant") {
      const thoughtExpanded = expandedThoughtByAssistantId[item.id] ?? false;
      return (
        <AssistantEntry
          id={item.id}
          content={item.content}
          thought={item.thought}
          thoughtSeconds={item.thought_seconds}
          isComplete={item.is_complete}
          variant={variant}
          thoughtExpanded={thoughtExpanded}
          onToggleThought={() =>
            setExpandedThoughtByAssistantId((prev) => ({ ...prev, [item.id]: !thoughtExpanded }))
          }
        />
      );
    }
    if (item.kind === "tool") {
      const toolExpanded = expandedToolById[item.id] ?? false;
      return (
        <WorkbenchToolRow
          item={item}
          variant={variant}
          expanded={toolExpanded}
          onToggle={() => setExpandedToolById((prev) => ({ ...prev, [item.id]: !toolExpanded }))}
        />
      );
    }
    return <ThreadItemView item={item} variant={variant} />;
  };

  return (
    <div
      className={`${wrapperClass} ctx-drop-scope`}
      ref={dropScopeRef}
    >
      {dropActive && (
        <div className="ctx-drop-overlay" aria-hidden="true">
          <div className="ctx-drop-overlay-text">Drop image to attach</div>
        </div>
      )}
      <div className={leftClass}>
        {entry?.error && (
          <div className="banner">
            <span className="error">{entry.error}</span>
          </div>
        )}
        {showDebug && variant === "workbench" && (
          <div className="wb-muted" style={{ fontFamily: "var(--mono)" }}>
            debug: events={events.length} messages={messages.length} userMessages={messages.filter((m) => m.role === "user").length} groups={wbGroups.length} headers={wbGroups.filter((g) => !!g.header).length} items={wbFlatItems.length}
          </div>
        )}
        {session && variant === "legacy" && (
          <div className="header">
            <div className="row" style={{ justifyContent: "space-between", alignItems: "center" }}>
              <Link to={`/tasks/${idToString(session.task_id)}`}>← Task</Link>
              <div className="row" style={{ alignItems: "center" }}>
                {showDebug ? (
                  <a className="muted" href={window.location.pathname}>
                    Hide debug
                  </a>
                ) : (
                  <a className="muted" href={`${window.location.pathname}?debug=1`}>
                    Debug
                  </a>
                )}
                {perfEnabled ? (
                  <a className="muted" href={window.location.pathname}>
                    Perf off
                  </a>
                ) : (
                  <a className="muted" href={`${window.location.pathname}?perf=1`}>
                    Perf
                  </a>
                )}
                {threadItems.length > 0 && (
                  <button
                    type="button"
                    onClick={() =>
                      virtuosoRef.current?.scrollToIndex({
                        index: threadItems.length - 1,
                        align: "end",
                      })
                    }
                  >
                    Jump to latest
                  </button>
                )}
              </div>
            </div>
            <div className="muted">
              {session.provider_id} / {session.model_id} ·{" "}
              {contextIndicator ? (
                <>
                  {contextIndicator.context_tokens_estimate} tokens ·{" "}
                  {Math.round(contextIndicator.remaining_fraction * 100)}% remaining
                </>
              ) : (
                <span title="Tokenizer/model window unknown">Unknown</span>
              )}
              {!streamConnected && <span title="Live stream disconnected; polling for updates."> · Polling</span>}
            </div>
          </div>
        )}

	        {(modelOptions.length > 0 || modeOptions.length > 0) && id && variant === "legacy" && (
	          <div className="card">
	            {modelCatalog.baseIds.length > 0 && (
	              <label>
	                Model
	                <select
	                  value={currentBase}
	                  onChange={async (e) => {
	                    const nextBase = e.target.value;
	                    const next = deriveFullModelIdForBase(nextBase, currentEffort);
	                    const updated = await setSessionModel(id, next);
	                    supervisor.setSession(updated);
	                  }}
	                >
	                  {modelCatalog.baseIds.map((b) => (
	                    <option key={b} value={b}>
	                      {modelCatalog.displayNameByBase[b] ?? b}
	                    </option>
	                  ))}
	                </select>
	              </label>
	            )}

	            {effortOptions.length > 0 && (
	              <label>
	                Effort
	                <select
	                  value={currentEffort ?? pickDefaultEffort(effortOptions) ?? ""}
	                  onChange={async (e) => {
	                    const nextEff = e.target.value || "";
	                    const next = deriveFullModelIdForBase(currentBase, nextEff || null);
	                    const updated = await setSessionModel(id, next);
	                    supervisor.setSession(updated);
	                  }}
	                >
	                  {effortOptions.map((eff) => (
	                    <option key={eff} value={eff}>
	                      {formatEffortLabel(eff)}
	                    </option>
	                  ))}
	                </select>
	              </label>
	            )}

            {modeOptions.length > 0 && (
              <label>
                Mode
                <select
                  value={currentModeId}
                  onChange={async (e) => {
                    await setSessionMode(id, e.target.value);
                    supervisor.refreshSession(id, { watchDiff: true });
                  }}
                >
                  {modeOptions.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
          </div>
        )}

        {interruptBanner && <div className="banner">{interruptBanner}</div>}

        {(authUi.status === "required" || authUi.status === "failed") && (
          <div className="banner">
            <div className="row" style={{ justifyContent: "space-between" }}>
              <strong>Authentication required</strong>
              <span className="muted">{authUi.provider ?? session?.provider_id}</span>
            </div>
            <div className="muted">
              {authUi.message ??
                "This provider requires authentication before it can run."}
            </div>
            {authUi.methods.length > 0 ? (
              <div className="row" style={{ flexWrap: "wrap" }}>
                {authUi.methods.length > 1 && (
                  <label>
                    Method
                    <select
                      value={authMethodId}
                      onChange={(e) => setAuthMethodId(e.target.value)}
                    >
                      {authUi.methods.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.name}
                        </option>
                      ))}
                    </select>
                  </label>
                )}
                <button
                  type="button"
                  disabled={authBusy || !id || !authMethodId}
                  onClick={async () => {
                    if (!id) return;
                    setAuthBusy(true);
                    setAuthError(null);
                    try {
                      await authenticateSession(id, authMethodId);
                      await refreshAll();
                    } catch (e: any) {
                      setAuthError(e?.message ?? String(e));
                    } finally {
                      setAuthBusy(false);
                    }
                  }}
                >
                  {authBusy ? "Authenticating…" : "Authenticate"}
                </button>
              </div>
            ) : (
              <div className="muted">
                No authentication methods were advertised by the provider.
              </div>
            )}
            {(authError || authUi.status === "failed") && (
              <div className="muted">
                {authError ??
                  "Authentication attempt failed. Check provider logs and try again."}
              </div>
            )}
          </div>
        )}

        {/* Zed-style: plan moves into the bottom activity bar; keep hidden above thread. */}

        {queue.length > 0 && (
          <div className="queue-panel card">
            <div className="row">
              <strong>Queued messages ({queue.length})</strong>
            </div>
            <ul className="sublist">
              {queue.map((m) => {
                const mid = idToString(m.id);
                return (
                  <li key={mid} className="row">
                    <span className="muted">{m.content}</span>
                    <button type="button" onClick={() => onRemoveQueued(mid)}>
                      Remove
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        )}

        {showDebug && debugEvents.length > 0 && (
          <DebugPanel events={debugEvents} />
        )}

        {(() => {
          const jumpToLatest = () => {
            if (variant === "workbench") {
              groupedVirtuosoRef.current?.scrollToIndex({ index: "LAST", align: "end" });
              return;
            }
            if (threadItems.length > 0) {
              virtuosoRef.current?.scrollToIndex({ index: threadItems.length - 1, align: "end" });
            }
          };

          if (variant === "workbench") {
            return (
              <div className="thread-stack">
                <GroupedVirtuoso
                  style={virtuosoStyle}
                  groupCounts={wbGroupCounts}
                  ref={groupedVirtuosoRef}
                  followOutput="auto"
                  atBottomStateChange={(b) => {
                    setAtBottom(b);
                    if (b) setHasNewActivity(false);
                  }}
                  components={{
                    List: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                      <div {...props} ref={ref} role="list" />
                    )),
                    Item: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                      <div {...props} ref={ref} role="listitem" />
                    )),
                  }}
                  groupContent={(groupIndex) => {
                    const header = wbGroups[groupIndex]?.header ?? null;
                    if (!header) return <div style={{ height: 0 }} />;
                    const isLong = header.content.split("\n").length > 4 || header.content.length > 280;
                    const expanded = expandedTurnHeaders[header.id] ?? !isLong;
                    return (
                      <WorkbenchTurnHeaderView
                        header={header}
                        expanded={expanded}
                        onToggle={() =>
                          setExpandedTurnHeaders((prev) => ({ ...prev, [header.id]: !expanded }))
                        }
                      />
                    );
                  }}
                  itemContent={(index, groupIndex) => {
                    const item = wbGroups[groupIndex]?.items[index];
                    if (!item) return <div style={{ height: 1 }} />;
                    return renderThreadItem(item);
                  }}
                />

                {hasNewActivity && (
                  <button
                    type="button"
                    className="new-activity-overlay"
                    aria-label="Jump to latest"
                    title="Jump to latest"
                    onClick={jumpToLatest}
                  >
                    ↓
                  </button>
                )}
              </div>
            );
          }

          return (
            <div className="thread-stack">
              <Virtuoso
                style={virtuosoStyle}
                data={threadItems}
                ref={virtuosoRef}
                followOutput="auto"
                atBottomStateChange={(b) => {
                  setAtBottom(b);
                  if (b) setHasNewActivity(false);
                }}
                components={{
                  List: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                    <div {...props} ref={ref} role="list" />
                  )),
                  Item: forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>((props, ref) => (
                    <div {...props} ref={ref} role="listitem" />
                  )),
                }}
                itemContent={(_, item) => renderThreadItem(item)}
              />

              {hasNewActivity && (
                <button
                  type="button"
                  className="new-activity-overlay"
                  aria-label="Jump to latest"
                  title="Jump to latest"
                  onClick={jumpToLatest}
                >
                  ↓
                </button>
              )}
            </div>
          );
        })()}

        {variant === "legacy" && <ActivityBar planEntries={planEntries} diffText={diff} />}

        {variant === "workbench" ? (
          <>
            <UnifiedWorkbenchComposer
              variant="activeSession"
              value={input}
              setValue={setInput}
              placeholder="Message, @ for context, / for commands"
              inputDisabled={dictationRecording}
              sessionIdForAutocomplete={id ?? null}
              slashCommands={slashCommands}
              attachments={draftAttachments}
              setAttachments={setDraftAttachments}
              onSend={sendNow}
              sendDisabled={!input.trim()}
              sendDisabledReason={!input.trim() ? "Enter a message." : null}
              onInterrupt={id ? () => interruptSession(id) : null}
              modeId={workbenchMode}
              setModeId={setWorkbenchMode}
              recording={dictationRecording}
              onToggleRecording={() => {
                if (dictationRecording) stopDictation().catch(() => {});
                else startDictation().catch(() => {});
              }}
              harnessLabel={
                HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.label ??
                (session?.provider_id ?? "Provider")
              }
              harnessLogoSrc={HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.logoSrc}
              harnessLogoInvert={HARNESS_CATALOG.find((h) => h.id === (session?.provider_id ?? ""))?.invertInDark}
              envLabel={session?.env_target === "local" ? "Local" : "Worktree"}
              availableModels={modelOptions}
              currentModelId={currentModelId}
              onSetModelId={async (next) => {
                if (!id) return;
                const updated = await setSessionModel(id, next);
                supervisor.setSession(updated);
              }}
            />
            {dictationDebugText && <div className="wb-banner">{dictationDebugText}</div>}
            {dictationError && <div className="wb-banner">{dictationError}</div>}
          </>
        ) : (
          <form onSubmit={onSend} className="composer">
            <div className="row">
              <button
                type="button"
                onClick={() => {
                  insertIntoComposer("@");
                }}
              >
                @ File
              </button>
              <button type="button" onClick={() => insertIntoComposer("/")} title="Slash commands">
                /
              </button>
              <label className="file-btn">
                Image
                <input
                  type="file"
                  accept="image/*"
                  multiple
                  onChange={async (e) => {
                    const files = Array.from(e.target.files ?? []);
                    const next = await imageFilesToBlobRefAttachments(files);
                    setDraftAttachments((prev) => [...prev, ...next]);
                    e.target.value = "";
                  }}
                />
              </label>
            </div>
            {draftAttachments.length > 0 && (
              <div className="card">
                <div className="muted">Attachments</div>
                <div className="row" style={{ flexWrap: "wrap" }}>
                  {draftAttachments.map((a, idx) => {
                    if (a.kind !== "image" && a.kind !== "image_ref") return null;
                    const src =
                      a.kind === "image_ref"
                        ? blobUrl(a.blob_id)
                        : `data:${a.mime_type};base64,${a.data_base64}`;
                    return (
                      <div key={idx} className="thumb">
                        <img src={src} alt={a.name ?? `image-${idx}`} />
                        <button
                          type="button"
                          onClick={() =>
                            setDraftAttachments((prev) => prev.filter((_, i) => i !== idx))
                          }
                        >
                          Remove
                        </button>
                      </div>
                    );
                  })}
                </div>
              </div>
            )}
            <textarea
              ref={textareaRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              placeholder="Send a message… (/ commands, @ file refs)"
              onKeyDown={(e) => {
                if (composerAutocomplete.onKeyDown(e)) return;
                if (shouldSendOnEnter(e)) {
                  e.preventDefault();
                  sendNow();
                }
              }}
              onKeyUp={() => composerAutocomplete.syncFromDom()}
              onClick={() => composerAutocomplete.syncFromDom()}
              onSelect={() => composerAutocomplete.syncFromDom()}
              onPaste={async (e) => {
                const files = Array.from(e.clipboardData?.files ?? []);
                const images = files.filter((f) => (f.type || "").startsWith("image/"));
                if (images.length === 0) return;
                e.preventDefault();
                const next: MessageAttachment[] = [];
                for (const f of images) {
                  const uploaded = await uploadBlob(f);
                  next.push({
                    kind: "image_ref",
                    blob_id: uploaded.blob_id,
                    mime_type: uploaded.mime_type,
                    name: uploaded.name ?? f.name,
                  });
                }
                setDraftAttachments((prev) => [...prev, ...next]);
              }}
            />
            <ComposerAutocompleteMenu
              open={composerAutocomplete.open}
              loading={composerAutocomplete.loading}
              items={composerAutocomplete.items}
              activeIndex={composerAutocomplete.activeIndex}
              onPick={composerAutocomplete.pick}
              onHoverIndex={(i) => composerAutocomplete.setActiveIndex(i)}
              anchorRect={composerAutocomplete.anchorRect}
              anchorInputRect={composerAutocomplete.anchorInputRect}
              inlineFallback={composerAutocomplete.inlineFallback}
            />
            <div className="row">
              <button type="submit">Send</button>
              <button type="button" onClick={() => id && cancelSession(id)}>
                Cancel
              </button>
              <button type="button" onClick={() => id && interruptSession(id)}>
                Interrupt
              </button>
            </div>
          </form>
        )}

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>

      {showDiffPane && (
        <div className="right">
          <DiffReviewPane
            diff={diff}
            trackId={session ? idToString(session.track_id) : ""}
            sessionId={id || undefined}
            onDiffUpdated={(d) => id && supervisor.setDiff(id, d)}
            onFileSaved={() => id && supervisor.refreshSession(id, { watchDiff: true })}
          />
        </div>
      )}
    </div>
  );
}

function ThreadItemView({ item, variant }: { item: ThreadItem; variant: SessionViewVariant }) {
  switch (item.kind) {
    case "message":
      return (
        <CollapsibleMessage
          id={item.id}
          role={item.role}
          content={item.content}
          attachments={item.attachments}
          delivery={item.delivery}
        />
      );
    case "assistant":
    case "tool":
      // Workbench uses specialized renderers; legacy never reaches here.
      return variant === "workbench" ? null : null;
  }
}

function WorkbenchTurnHeaderView({
  header,
  expanded,
  onToggle,
}: {
  header: WorkbenchTurnHeader;
  expanded: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className={`wb-turn-header ${expanded ? "wb-turn-header-expanded" : "wb-turn-header-collapsed"}`}
      onClick={onToggle}
      aria-expanded={expanded}
    >
      <div className="wb-turn-header-bubble">
        <div className="wb-turn-header-content">
          <Markdown content={header.content} />
        </div>
        {header.attachments.length > 0 && (
          <div className="wb-turn-header-attachments" aria-label="Attachments">
            {header.attachments.map((a, idx) => {
              if (a.kind !== "image" && a.kind !== "image_ref") return null;
              const src = imageAttachmentSrc(a);
              const name = attachmentDisplayName(a.name);
              return <img key={idx} className="wb-turn-header-attachment-img" src={src} alt={name} title={name} />;
            })}
          </div>
        )}
      </div>
    </button>
  );
}

function WorkbenchComposer({
  session,
  modelOptions,
  effortOptions,
  currentModelId,
  autocomplete,
  onSetModel,
  onSetEffort,
  textareaRef,
  input,
  setInput,
  onSend,
  onInterrupt,
  onInsertAtFile,
  onInsertSlash,
  attachments,
  setAttachments,
}: {
  session: Session | null;
  modelOptions: Array<{ id: string; name: string }>;
  effortOptions: string[];
  currentModelId: string;
  autocomplete: ReturnType<typeof useComposerAutocomplete>;
  onSetModel: (modelId: string) => Promise<void>;
  onSetEffort: (effort: string) => Promise<void>;
  textareaRef: { current: HTMLTextAreaElement | null };
  input: string;
  setInput: (next: string) => void;
  onSend: () => Promise<void>;
  onInterrupt: () => void;
  onInsertAtFile: () => void;
  onInsertSlash: () => void;
  attachments: MessageAttachment[];
  setAttachments: React.Dispatch<React.SetStateAction<MessageAttachment[]>>;
}) {
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "0px";
    const next = Math.min(220, Math.max(28, el.scrollHeight));
    el.style.height = `${next}px`;
  }, [input, textareaRef]);

  const providerLabel = useMemo(() => {
    const p = String(session?.provider_id ?? "agent");
    if (p === "codex") return "Codex";
    if (p === "claude") return "Claude";
    if (p === "gemini") return "Gemini";
    if (p === "fake") return "Fake";
    return p.slice(0, 1).toUpperCase() + p.slice(1);
  }, [session?.provider_id]);

  const currentEffort = String(currentModelId).split("/")[1] ?? "";

  return (
    <div className="wb-composer">
      {attachments.length > 0 && (
        <div className="wb-composer-attachments">
          {attachments.map((a, idx) => (
            <button
              key={idx}
              type="button"
              className="wb-attach-chip"
              onClick={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}
              title="Remove attachment"
            >
              {a.kind === "image" || a.kind === "image_ref" ? (a.name ?? "image") : "attachment"} ×
            </button>
          ))}
        </div>
      )}

      <textarea
        ref={textareaRef}
        className="wb-composer-input"
        value={input}
        onChange={(e) => setInput(e.target.value)}
        placeholder="Ask follow-ups in the worktree"
        onKeyDown={(e) => {
          if (autocomplete.onKeyDown(e)) return;
          if (shouldSendOnEnter(e)) {
            e.preventDefault();
            onSend();
          }
        }}
        onKeyUp={() => autocomplete.syncFromDom()}
        onClick={() => autocomplete.syncFromDom()}
        onSelect={() => autocomplete.syncFromDom()}
        onPaste={async (e) => {
          const files = Array.from(e.clipboardData?.files ?? []);
          const images = files.filter((f) => (f.type || "").startsWith("image/"));
          if (images.length === 0) return;
          e.preventDefault();
          const next: MessageAttachment[] = [];
          for (const f of images) {
            const uploaded = await uploadBlob(f);
            next.push({
              kind: "image_ref",
              blob_id: uploaded.blob_id,
              mime_type: uploaded.mime_type,
              name: uploaded.name ?? f.name,
            });
          }
          setAttachments((prev) => [...prev, ...next]);
        }}
      />

      <ComposerAutocompleteMenu
        open={autocomplete.open}
        loading={autocomplete.loading}
        items={autocomplete.items}
        activeIndex={autocomplete.activeIndex}
        onPick={autocomplete.pick}
        onHoverIndex={(i) => autocomplete.setActiveIndex(i)}
        anchorRect={autocomplete.anchorRect}
        anchorInputRect={autocomplete.anchorInputRect}
        inlineFallback={autocomplete.inlineFallback}
      />

      <div className="wb-composer-footer">
        <div className="wb-composer-left">
          <div className="wb-composer-pill" title="Harness">
            ∞ {providerLabel}
          </div>

          {modelOptions.length > 0 && (
            <div className="wb-composer-pill" title="Model">
              <select
                className="wb-composer-select"
                value={currentModelId}
                onChange={(e) => onSetModel(e.target.value)}
              >
                {modelOptions.map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                  </option>
                ))}
              </select>
            </div>
          )}

	          {effortOptions.length > 0 && (
	            <div className="wb-composer-pill" title="Effort">
	              <select
	                className="wb-composer-select"
	                value={currentEffort}
	                onChange={(e) => onSetEffort(e.target.value)}
	              >
	                {effortOptions.map((eff) => (
	                  <option key={eff} value={eff}>
	                    {formatEffortLabel(eff)}
	                  </option>
	                ))}
	              </select>
	            </div>
	          )}
        </div>

        <div className="wb-composer-right">
          <button type="button" className="wb-composer-icon" onClick={onInterrupt} title="Interrupt">
            ■
          </button>
          <button type="button" className="wb-composer-icon" onClick={onInsertAtFile} title="Insert @file">
            @
          </button>
          <button type="button" className="wb-composer-icon" onClick={onInsertSlash} title="Slash commands">
            /
          </button>
          <button
            type="button"
            className="wb-composer-icon"
            onClick={() => fileInputRef.current?.click()}
            title="Attach image"
          >
            ☐
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            multiple
            style={{ display: "none" }}
            onChange={async (e) => {
              const files = Array.from(e.target.files ?? []);
              const next = await imageFilesToBlobRefAttachments(files);
              setAttachments((prev) => [...prev, ...next]);
              e.target.value = "";
            }}
          />
        </div>
      </div>
    </div>
  );
}

function CollapsibleMessage({
  id,
  role,
  content,
  attachments,
  delivery,
}: {
  id: string;
  role: "user" | "assistant";
  content: string;
  attachments: MessageAttachment[];
  delivery?: "immediate" | "queued";
}) {
  const lines = (content || "").split("\n");
  const isLong = lines.length > 20 || content.length > 1500;
  const [expanded, setExpanded] = useState(!isLong);
  const shown = expanded ? content : lines.slice(0, 20).join("\n");

  return (
    <div className={`msg ${role}`}>
      <div className="role">{role}</div>
      <div id={`msg-${id}`}>
        <Markdown content={shown} />
      </div>
      {attachments?.length > 0 && (
        <div className="attachments">
          {attachments.map((a, idx) => {
            if (a.kind !== "image" && a.kind !== "image_ref") return null;
            const src =
              a.kind === "image_ref"
                ? blobUrl(a.blob_id)
                : `data:${a.mime_type};base64,${a.data_base64}`;
            return (
              <img
                key={idx}
                className="attachment-img"
                src={src}
                alt={a.name ?? `image-${idx}`}
              />
            );
          })}
        </div>
      )}
      {delivery === "queued" && <span className="badge">Queued</span>}
      {isLong && (
        <button
          type="button"
          className="link"
          aria-expanded={expanded}
          aria-controls={`msg-${id}`}
          onClick={() => setExpanded((e) => !e)}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function AssistantEntry({
  id,
  content,
  thought,
  thoughtSeconds,
  isComplete,
  variant,
  thoughtExpanded,
  onToggleThought,
}: {
  id: string;
  content: string;
  thought: string;
  thoughtSeconds?: number;
  isComplete: boolean;
  variant: SessionViewVariant;
  thoughtExpanded: boolean;
  onToggleThought: () => void;
}) {
  const [showThought, setShowThought] = useState(false);
  const show = variant === "workbench" ? thoughtExpanded : showThought;
  const toggle = variant === "workbench" ? onToggleThought : () => setShowThought((s) => !s);

  if (variant === "workbench") {
    return (
      <div className="wb-assistant-entry">
        {thought.trim() && (
          <div className="wb-thought">
            <button
              type="button"
              className="wb-event-row wb-thought-row"
              onClick={toggle}
              aria-expanded={show}
              aria-controls={`thought-${id}`}
            >
              <span className="wb-event-text">Thought for {thoughtSeconds ?? 1}s</span>
            </button>
            {show && (
              <pre id={`thought-${id}`} className="wb-thought-body">
                {thought}
              </pre>
            )}
          </div>
        )}
        <div className="wb-assistant-body">
          <Markdown content={content} />
        </div>
      </div>
    );
  }

  return (
    <div className="msg assistant">
      <div className="row">
        <div className="role">assistant</div>
        <span className={`pill ${isComplete ? "ok" : "run"}`}>
          {isComplete ? "complete" : "streaming"}
        </span>
      </div>
      <div id={`msg-${id}`}>
        <Markdown content={content} />
      </div>
      {thought.trim() && (
        <div className="thinking">
          <button
            type="button"
            className="thinking-header"
            aria-expanded={show}
            aria-controls={`thought-${id}`}
            onClick={toggle}
          >
            <span className="thinking-title">Thinking</span>
            <span className="thinking-chev">{show ? "▴" : "▾"}</span>
          </button>
          {show && (
            <pre id={`thought-${id}`} className="thought">
              {thought}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

function WorkbenchToolRow({
  item,
  variant,
  expanded,
  onToggle,
}: {
  item: Extract<ThreadItem, { kind: "tool" }>;
  variant: SessionViewVariant;
  expanded: boolean;
  onToggle: () => void;
}) {
  if (variant !== "workbench") return <ToolCard item={item} />;

  const kind = String(item.tool_kind ?? "").toLowerCase();
  const pathFromLoc = item.locations?.[0]?.path;
  const title = String(item.title ?? "").trim();
  const summary = toolSummaryLine(kind, item.input);

  const normalizeWorktreePath = (p?: string) => {
    const s = String(p ?? "").trim();
    if (!s) return "";
    // Strip the Context worktree prefix.
    return s.replace(
      /\/home\/[^/]+\/\.context\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
  };

  const shortPath = (p?: string) => {
    const s0 = normalizeWorktreePath(p);
    const s = String(s0 ?? "").trim();
    if (!s) return "";
    const parts = s.split(/[\\/]/).filter(Boolean);
    if (parts.length <= 2) return s;
    return `${parts[parts.length - 2]}/${parts[parts.length - 1]}`;
  };

  const shortCommand = (raw: string) => {
    let cmd = String(raw ?? "").trim();
    if (!cmd) return "";
    cmd = cmd.replace(/^\/bin\/bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^bash\s+-lc\s+/, "");
    cmd = cmd.replace(/^set\s+-euo\s+pipefail\s*;?\s*/i, "");
    cmd = cmd.replace(/^\s*&&\s*/, "");
    cmd = cmd.replace(/^"(.+)"$/, "$1");
    cmd = cmd.replace(
      /\/home\/[^/]+\/\.context\/worktrees\/[0-9a-f-]+\/[0-9a-f-]+\//g,
      "",
    );
    const mSupercat = cmd.match(/(?:^|\s)(\.?\/?scripts\/supercat\.sh)\s+([^\s&;]+)/);
    if (mSupercat) return `./scripts/supercat.sh ${shortPath(mSupercat[2])}`;
    const mReadMany =
      cmd.match(/rg\s+--files\s+([^\s|&;]+)\s*\|\s*sort\b[\s\S]*xargs[\s\S]*\bcat\b/) ??
      cmd.match(/find\s+([^\s|&;]+)\s+.*xargs[\s\S]*\bcat\b/);
    if (mReadMany) return `Read ${shortPath(mReadMany[1])}`;
    const mCat = cmd.match(/(?:^|[;&|]\s*)cat\s+([^\s|&;]+)(?:\s|$)/);
    if (mCat) return `Read ${shortPath(mCat[1])}`;
    const mLs = cmd.match(/(?:^|[;&|]\s*)(?:ls|ls\s+-la|ls\s+-l)\s+([^\s|&;]+)(?:\s|$)/);
    if (mLs) return `Explored ${shortPath(mLs[1])}`;
    const mRg = cmd.match(/(?:^|[;&|]\s*)rg\s+([^\s]+)\s+([^\s|&;]+)(?:\s|$)/);
    if (mRg) return `Searched ${truncateMiddle(mRg[1], 60)}`;
    const first = cmd.split(/\s+/).slice(0, 4).join(" ");
    return truncateMiddle(first, 80);
  };

  const preferTitle =
    title.length > 0 &&
    title.length <= 90 &&
    /^(Read|Wrote|Edited|List|Listed|Explore|Explored|Planning|Plan|Thought|Run)\b/.test(title);

  const label = (() => {
    const parsed = Array.isArray((item.input as any)?.parsed_cmd) ? ((item.input as any).parsed_cmd as any[]) : [];
    if (parsed.length > 0) {
      const c0 = parsed[0] ?? {};
      if (c0.type === "list_files" && c0.path) return `Explored ${shortPath(c0.path)}`;
      if (c0.type === "read_file" && c0.path) return `Read ${shortPath(c0.path)}`;
    }

    if (preferTitle) {
      if (/^Run\b/.test(title)) {
        const cmd = Array.isArray(item.input?.command) ? item.input.command.join(" ") : item.input?.command;
        const short = shortCommand(cmd ?? title.replace(/^Run\s+/, ""));
        if (!short) return title;
        if (/^(Read|Explored|Searched|Wrote|Edited)\b/.test(short)) return short;
        return `Run ${short}`;
      }
      return title;
    }

    if (/^Run\b/.test(title)) {
      const cmd = Array.isArray(item.input?.command) ? item.input.command.join(" ") : item.input?.command;
      const short = shortCommand(cmd ?? title.replace(/^Run\s+/, ""));
      if (!short) return truncateMiddle(title, 90);
      if (/^(Read|Explored|Searched|Wrote|Edited)\b/.test(short)) return short;
      return `Run ${short}`;
    }

    if (kind === "search") {
      if (/^List\b/.test(title)) return `Explored ${shortPath(title.replace(/^List\s+/, ""))}`;
      const q = String(item.input?.query ?? item.input?.pattern ?? item.input?.text ?? summary ?? "").trim();
      if (/^(List|Listed|Explore|Explored)\b/.test(q)) return truncateMiddle(q, 90);
      return q ? `Searched ${truncateMiddle(q, 90)}` : "Searched";
    }
    if (kind === "execute") {
      const cmd = Array.isArray(item.input?.command) ? item.input.command.join(" ") : item.input?.command;
      const short = shortCommand(cmd ?? "");
      if (!short) return "Run";
      if (/^(Read|Explored|Searched|Wrote|Edited)\b/.test(short)) return short;
      return `Run ${short}`;
    }
    if (kind === "read_file" || kind === "read") {
      const p = pathFromLoc ?? summary;
      return p ? `Read ${shortPath(p)}` : "Read";
    }
    if (kind === "write" || kind === "edit") {
      const p = summary || pathFromLoc;
      return p ? `${kind === "write" ? "Wrote" : "Edited"} ${shortPath(p)}` : kind === "write" ? "Wrote" : "Edited";
    }
    if (kind === "error") return `Error`;
    return normalizeWorktreePath(title) || humanToolKind(item.tool_kind);
  })();

  const hasDetails = !!(item.input || item.output_text?.trim());

  return (
    <div className="wb-tool-row">
      <button
        type="button"
        className={`wb-event-row ${expanded ? "wb-event-row-expanded" : ""}`}
        onClick={hasDetails ? onToggle : undefined}
        aria-expanded={hasDetails ? expanded : undefined}
        title={label}
      >
        <span className="wb-event-text">{label}</span>
      </button>
      {hasDetails && expanded && (
        <div className="wb-tool-details">
          {item.input && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Input</div>
              <pre className="wb-tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {!!item.output_text?.trim() && (
            <div className="wb-tool-section">
              <div className="wb-tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="wb-tool-markdown">
                  <Markdown content={item.output_text} />
                </div>
              ) : (
                <pre className="wb-tool-pre">{item.output_text}</pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function ToolCard({ item }: { item: Extract<ThreadItem, { kind: "tool" }> }) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = item.status === "in_progress" || item.status === "pending";
  const isFailed = item.status === "failed";
  const hasOutput = item.output_text.trim().length > 0;
  const shouldDefaultOpen = item.tool_kind === "execute" && isRunning;
  const isOpen = expanded || shouldDefaultOpen;
  const summary = useMemo(() => toolSummaryLine(item.tool_kind, item.input), [item.tool_kind, item.input]);

  return (
    <div className={`tool-card ${isOpen ? "expanded" : ""}`}>
      <button
        type="button"
        className="tool-header"
        onClick={() => setExpanded((e) => !e)}
        aria-expanded={isOpen}
        aria-controls={`tool-${item.id}`}
      >
        <div className="tool-header-left">
          <span className={`tool-icon kind-${item.tool_kind}`}>{toolKindIcon(item.tool_kind)}</span>
          <div className="tool-title-wrap">
            <div className="tool-title">{item.title}</div>
            <div className="tool-subtitle">
              <span className={`pill ${isFailed ? "err" : isRunning ? "run" : "ok"}`}>
                {humanToolStatus(item.status)}
              </span>
              {item.locations?.length === 1 && item.locations[0]?.path && (
                <span className="muted tool-path">{item.locations[0].path}</span>
              )}
              {summary && <span className="muted tool-summary">{summary}</span>}
            </div>
          </div>
        </div>
        <div className="tool-header-right">
          <span className="muted">{new Date(item.updated_at).toLocaleTimeString()}</span>
          <span className="thinking-chev">{isOpen ? "▴" : "▾"}</span>
        </div>
      </button>

      {isOpen && (
        <div id={`tool-${item.id}`} className="tool-body">
          {item.input && (
            <div className="tool-section">
              <div className="tool-section-title">Input</div>
              <pre className="tool-pre">{formatToolInput(item.tool_kind, item.input)}</pre>
            </div>
          )}
          {hasOutput && (
            <div className="tool-section">
              <div className="tool-section-title">Output</div>
              {looksLikeMarkdown(item.output_text) ? (
                <div className="tool-markdown">
                  <Markdown content={item.output_text} />
                </div>
              ) : (
                <pre className="tool-pre tool-output">{item.output_text}</pre>
              )}
            </div>
          )}
          <details className="tool-raw">
            <summary className="link">
              Raw event {item.updates_seen > 1 ? `(updated ${item.updates_seen}×)` : ""}
            </summary>
            <pre className="json">{JSON.stringify(item.raw, null, 2)}</pre>
          </details>
        </div>
      )}
    </div>
  );
}

function DebugPanel({ events }: { events: SessionEvent[] }) {
  const [open, setOpen] = useState(false);
  const kinds = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of events) {
      counts[e.event_type] = (counts[e.event_type] ?? 0) + 1;
    }
    return counts;
  }, [events.length]);

  return (
    <div className="debug card">
      <button type="button" className="debug-header" onClick={() => setOpen((v) => !v)}>
        <strong>Debug</strong>
        <span className="muted">
          {Object.entries(kinds)
            .map(([k, v]) => `${k}:${v}`)
            .join(" · ")}
        </span>
        <span className="thinking-chev">{open ? "▴" : "▾"}</span>
      </button>
      {open && (
        <div className="debug-body">
          {events.map((e) => {
            const key = idToString(e.id) || `${e.created_at}-${e.event_type}`;
            return (
              <details key={key} className="debug-event">
                <summary>
                  <span className="muted">{new Date(e.created_at).toLocaleTimeString()}</span>{" "}
                  <strong>{e.event_type}</strong>
                </summary>
                <pre className="json">{JSON.stringify(e.payload_json, null, 2)}</pre>
              </details>
            );
          })}
        </div>
      )}
    </div>
  );
}

function PlanPanel({ entries }: { entries: any[] }) {
  const [expanded, setExpanded] = useState(false);
  const counts = entries.reduce(
    (acc, e) => {
      const status = e?.status ?? "pending";
      if (status === "completed") acc.completed += 1;
      else if (status === "in_progress") acc.in_progress += 1;
      else acc.pending += 1;
      return acc;
    },
    { pending: 0, in_progress: 0, completed: 0 },
  );

  return (
    <div className="plan-bar card">
      <div className="row">
        <strong>Plan</strong>
        <span className="muted">
          {counts.in_progress} in progress · {counts.pending} pending ·{" "}
          {counts.completed} done
        </span>
      </div>
      <button type="button" onClick={() => setExpanded((e) => !e)}>
        {expanded ? "Hide plan" : "Show plan"}
      </button>
      {expanded && (
        <ul className="sublist">
          {entries.map((e, idx) => (
            <li key={idx} className={`plan-item ${e.status ?? ""}`}>
              <span className="muted">{e.status ?? "pending"}</span> {e.content}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function ActivityBar({ planEntries, diffText }: { planEntries: any[]; diffText: string }) {
  const [planOpen, setPlanOpen] = useState(false);
  const [editsOpen, setEditsOpen] = useState(false);

  const planStats = useMemo(() => {
    const counts = planEntries.reduce(
      (acc, e) => {
        const status = e?.status ?? "pending";
        if (status === "completed") acc.completed += 1;
        else if (status === "in_progress") acc.in_progress += 1;
        else acc.pending += 1;
        return acc;
      },
      { pending: 0, in_progress: 0, completed: 0 },
    );
    const current = planEntries.find((e) => e?.status === "in_progress") ?? null;
    return { ...counts, current };
  }, [planEntries]);

  const editedFiles = useMemo(() => extractEditedFiles(diffText), [diffText]);

  if (planEntries.length === 0 && editedFiles.length === 0) return null;

  return (
    <div className="activity-bar">
      {planEntries.length > 0 && (
        <div className="activity-section">
          <button type="button" className="activity-summary" onClick={() => setPlanOpen((v) => !v)}>
            <span className="activity-title">Plan</span>
            {planStats.current && !planOpen ? (
              <span className="muted activity-current">
                Current: {String(planStats.current.content ?? "").trim() || "in progress"}
              </span>
            ) : (
              <span className="muted">
                {planStats.in_progress} in progress · {planStats.pending} pending · {planStats.completed} done
              </span>
            )}
            {planStats.pending > 0 && !planOpen && (
              <span className="muted activity-right">{planStats.pending} left</span>
            )}
            <span className="thinking-chev">{planOpen ? "▴" : "▾"}</span>
          </button>
          {planOpen && (
            <ul className="activity-list">
              {planEntries.map((e, idx) => (
                <li key={idx} className={`plan-item ${e.status ?? ""}`}>
                  <span className={`plan-dot ${e.status ?? ""}`} />
                  <span className="muted">{e.status ?? "pending"}</span>{" "}
                  <span className="activity-item-text">{e.content}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {editedFiles.length > 0 && (
        <div className="activity-section">
          <button type="button" className="activity-summary" onClick={() => setEditsOpen((v) => !v)}>
            <span className="activity-title">Edits</span>
            <span className="muted">{editedFiles.length} file{editedFiles.length === 1 ? "" : "s"}</span>
            <span className="thinking-chev">{editsOpen ? "▴" : "▾"}</span>
          </button>
          {editsOpen && (
            <ul className="activity-list">
              {editedFiles.map((f) => (
                <li key={f} className="activity-item-text">
                  {f}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}

function extractEditedFiles(diffText: string): string[] {
  const text = String(diffText ?? "");
  const files = new Set<string>();
  for (const line of text.split("\n")) {
    const m = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
    if (m) {
      files.add(m[2]);
      continue;
    }
    const m2 = /^\+\+\+ b\/(.+)$/.exec(line);
    if (m2) files.add(m2[1]);
  }
  return [...files].slice(0, 200);
}

type MdastNode = {
  type?: string;
  children?: MdastNode[];
  [key: string]: unknown;
};

function remarkNormalizeCursorMarkdown() {
  return (tree: MdastNode) => {
    const walk = (node: MdastNode) => {
      if (node?.type === "listItem" && Array.isArray(node.children)) {
        const maybeInlineFromCode = (codeNode: MdastNode): string | null => {
          if (codeNode?.type !== "code") return null;
          const lang = typeof codeNode.lang === "string" ? codeNode.lang : undefined;
          const raw = typeof codeNode.value === "string" ? codeNode.value : "";
          const trimmed = raw.replace(/[\r\n]+$/, "");
          if ((!lang || lang === "code") && trimmed.length > 0 && !trimmed.includes("\n")) return trimmed;
          return null;
        };

        // Cursor sometimes emits list items that are a single one-line fenced code block.
        if (node.children.length === 1) {
          const trimmed = maybeInlineFromCode(node.children[0] as MdastNode);
          if (trimmed) {
            node.children = [
              {
                type: "paragraph",
                children: [{ type: "inlineCode", value: trimmed }],
              },
            ];
          }
        }

        // Some models emit list items as:
        //   - code
        //     ```code
        //     .github/
        //     ```
        // Normalize that to a single inlineCode row.
        if (node.children.length === 2) {
          const [p, codeNode] = node.children as MdastNode[];
          if (p?.type === "paragraph" && Array.isArray((p as any).children)) {
            const kids = (p as any).children as MdastNode[];
            if (
              kids.length === 1 &&
              kids[0]?.type === "text" &&
              String((kids[0] as any).value ?? "").trim().toLowerCase() === "code"
            ) {
              const trimmed = maybeInlineFromCode(codeNode);
              if (trimmed) {
                node.children = [
                  {
                    type: "paragraph",
                    children: [{ type: "inlineCode", value: trimmed }],
                  },
                ];
              }
            }
          }
        }
      }

      if (!Array.isArray(node.children)) return;
      for (const child of node.children) walk(child);
    };

    walk(tree);
  };
}

function Markdown({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm, remarkNormalizeCursorMarkdown]}
      components={{
        pre({ children }) {
          return <>{children}</>;
        },
        code({ inline, className, children }) {
          const match = /language-([A-Za-z0-9_-]+)/.exec(className || "");
          const rawLang = match?.[1];
          const lang = rawLang && rawLang !== "code" ? rawLang : undefined;
          const codeString = String(children ?? "").replace(/[\r\n]+$/, "");
          if (inline) {
            return <code className={className}>{children}</code>;
          }

          if (!lang && !codeString.includes("\n") && codeString.length <= 120) {
            return <code className={className}>{codeString}</code>;
          }

          return (
            <div className="codeblock">
              <button
                type="button"
                className="codeblock-copy"
                aria-label="Copy code"
                title="Copy"
                onClick={() => navigator.clipboard.writeText(codeString)}
              >
                ⧉
              </button>
              <SyntaxHighlighter style={oneDark} language={lang} PreTag="div">
                {codeString}
              </SyntaxHighlighter>
            </div>
          );
        },
      }}
    >
      {content}
    </ReactMarkdown>
  );
}

export function buildWorkbenchThreadViewModel(events: SessionEvent[], messages: Message[]): WorkbenchThreadView {
  type ToolItem = Extract<ThreadItem, { kind: "tool" }>;
  type TurnGroup = {
    key: string;
    header: WorkbenchTurnHeader | null;
    first_at: string;
    toolItems: ToolItem[];
    toolById: Map<string, ToolItem>;
    assistant: Extract<ThreadItem, { kind: "assistant" }> | null;
    thought_first_at: string | null;
    thought_last_at: string | null;
    assistant_first_at: string | null;
    assistant_complete_at: string | null;
  };

  const debugEvents: SessionEvent[] = [];

  const userMessages = messages
    .filter((m) => m.role === "user")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

  const assistantMessages = messages
    .filter((m) => m.role === "assistant")
    .slice()
    .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

  const ensureTool = (g: TurnGroup, toolCallId: string, createdAt: string) => {
    const existing = g.toolById.get(toolCallId);
    if (existing) return existing;
    const t: ToolItem = {
      kind: "tool",
      id: `tool-${toolCallId}`,
      tool_call_id: toolCallId,
      created_at: createdAt,
      updated_at: createdAt,
      tool_kind: "tool",
      title: "Tool",
      status: "pending",
      locations: [],
      input: null,
      output_text: "",
      raw: null,
      updates_seen: 0,
    };
    g.toolById.set(toolCallId, t);
    g.toolItems.push(t);
    return t;
  };

  // If messages haven't been refreshed yet (common in the Workbench "Start" flow), fall back to
  // grouping by `user_message` events so streamed assistant/tool updates still render.
  if (userMessages.length === 0) {
    const userEvents = events
      .filter((e) => e.event_type === "user_message")
      .slice()
      .sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));

    const eventsInRangeExclusive = (startIso: string, endIso: string | null) => {
      const start = Date.parse(startIso);
      const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
      return events.filter((e) => {
        const t = Date.parse(String(e.created_at));
        if (!Number.isFinite(t) || !Number.isFinite(start)) return false;
        return t >= start && t < end;
      });
    };

    const groups: WorkbenchThreadView["groups"] = [];

    if (userEvents.length === 0) {
      // As a last resort, show any tool activity even without a user turn anchor.
      const g: TurnGroup = {
        key: "no-user-messages",
        header: null,
        first_at: events[0]?.created_at ?? new Date().toISOString(),
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        assistant_first_at: null,
        assistant_complete_at: null,
      };
      for (const ev of events) {
        const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) continue;
        ensureTool(g, toolCallId, ev.created_at);
      }
      const items: ThreadItem[] = [...g.toolItems];
      if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      groups.push({ key: g.key, header: g.header, items });
      return { groups, debugEvents };
    }

    for (let i = 0; i < userEvents.length; i++) {
      const u = userEvents[i];
      const nextUser = userEvents[i + 1] ?? null;
      const mid =
        String(u.payload_json?.message_id ?? "").trim() ||
        (idToString(u.id) || `msg-${u.created_at}`);

      const header: WorkbenchTurnHeader = {
        id: mid,
        content: String(u.payload_json?.content ?? ""),
        attachments: Array.isArray(u.payload_json?.attachments)
          ? (u.payload_json.attachments as MessageAttachment[])
          : [],
        created_at: u.created_at,
      };

      const g: TurnGroup = {
        key: `m-${mid}`,
        header,
        first_at: u.created_at,
        toolItems: [],
        toolById: new Map(),
        assistant: null,
        thought_first_at: null,
        thought_last_at: null,
        assistant_first_at: null,
        assistant_complete_at: null,
      };

      const evs = eventsInRangeExclusive(u.created_at, nextUser?.created_at ?? null);

      for (const ev of evs) {
        const eventId = idToString(ev.id) || `${ev.created_at}`;
        if (ev.created_at < g.first_at) g.first_at = ev.created_at;

        switch (ev.event_type) {
          case "error": {
            const message = String(ev.payload_json?.message ?? "Error");
            const provider = String(ev.payload_json?.provider ?? "").trim();
            const output = provider ? `${message}\nprovider: ${provider}` : message;
            const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
            tool.tool_kind = "error";
            tool.title = "Error";
            tool.status = "failed";
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.input = ev.payload_json ?? null;
            tool.output_text = output;
            tool.raw = ev;
            break;
          }
          case "assistant_chunk": {
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
            g.assistant.content += fragment;
            break;
          }
          case "assistant_complete": {
            const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.assistant_complete_at = ev.created_at;
            if (full) g.assistant.content = full;
            g.assistant.is_complete = true;
            break;
          }
          case "thought_chunk": {
            const fragment = String(ev.payload_json?.content_fragment ?? "");
            if (!fragment) break;
            if (!g.assistant) {
              g.assistant = {
                kind: "assistant",
                id: `assistant-${g.key}`,
                created_at: ev.created_at,
                content: "",
                thought: "",
                is_complete: false,
              };
            }
            g.thought_first_at = g.thought_first_at ?? ev.created_at;
            g.thought_last_at = ev.created_at;
            g.assistant.thought += fragment;
            break;
          }
          case "tool_call":
          case "tool_call_update":
          case "tool_result": {
            const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
            const toolCallId =
              String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
            if (!toolCallId) {
              debugEvents.push(ev);
              break;
            }
            const tool = ensureTool(g, toolCallId, ev.created_at);
            tool.updated_at = ev.created_at;
            tool.updates_seen += 1;
            tool.raw = ev.payload_json;

            const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
            if (nextKind) tool.tool_kind = nextKind;

            const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
            if (nextTitle) tool.title = nextTitle;
            else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

            const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
            if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
            else if (ev.event_type === "tool_result") tool.status = "completed";

            const locs = Array.isArray(update?.locations) ? update.locations : [];
            tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

            const rawInput =
              update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
            if (rawInput != null) tool.input = rawInput;

            const output =
              update?.outputText ??
              update?.output_text ??
              update?.toolCall?.outputText ??
              update?.toolCall?.output_text ??
              update?.result ??
              null;
            if (typeof output === "string") tool.output_text = output;

            break;
          }
          default: {
            break;
          }
        }
      }

      const items: ThreadItem[] = [];
      items.push(...g.toolItems);
      if (g.assistant) {
        items.push({
          ...g.assistant,
          thought_seconds: (() => {
            if (!g.thought_first_at) return undefined;
            const start = Date.parse(g.thought_first_at);
            const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
            if (!endRaw) return undefined;
            const end = Date.parse(endRaw);
            if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
            return Math.max(1, Math.round((end - start) / 1000));
          })(),
        });
      }
      if (items.length === 0) {
        items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
      }

      groups.push({ key: g.key, header: g.header, items });
    }

    return { groups, debugEvents };
  }

  const eventsInRange = (startIso: string, endIso: string | null) => {
    const start = Date.parse(startIso);
    const end = endIso ? Date.parse(endIso) : Number.POSITIVE_INFINITY;
    return events.filter((e) => {
      const t = Date.parse(String(e.created_at));
      return Number.isFinite(t) && t >= start && t <= end;
    });
  };

  const groups: WorkbenchThreadView["groups"] = [];

  for (let i = 0; i < userMessages.length; i++) {
    const u = userMessages[i];
    const nextUser = userMessages[i + 1] ?? null;

    const mid = idToString(u.id) || `msg-${u.created_at}`;
    const g: TurnGroup = {
      key: `m-${mid}`,
      header: {
        id: mid,
        content: u.content ?? "",
        attachments: Array.isArray((u as any).attachments) ? ((u as any).attachments as MessageAttachment[]) : [],
        created_at: u.created_at,
      },
      first_at: u.created_at,
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      assistant_first_at: null,
      assistant_complete_at: null,
    };

    const assistant = assistantMessages.find((a) => {
      const ta = Date.parse(String(a.created_at));
      const tu = Date.parse(String(u.created_at));
      if (!Number.isFinite(ta) || !Number.isFinite(tu) || ta <= tu) return false;
      if (!nextUser) return true;
      const tn = Date.parse(String(nextUser.created_at));
      return !Number.isFinite(tn) || ta < tn;
    });

    const endAt = assistant?.created_at ?? nextUser?.created_at ?? null;
    const evs = eventsInRange(u.created_at, endAt);

    for (const ev of evs) {
      const eventId = idToString(ev.id) || `${ev.created_at}`;
      if (ev.created_at < g.first_at) g.first_at = ev.created_at;

      switch (ev.event_type) {
        case "error": {
          const message = String(ev.payload_json?.message ?? "Error");
          const provider = String(ev.payload_json?.provider ?? "").trim();
          const output = provider ? `${message}\nprovider: ${provider}` : message;
          const tool = ensureTool(g, `error-${eventId}`, ev.created_at);
          tool.tool_kind = "error";
          tool.title = "Error";
          tool.status = "failed";
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.input = ev.payload_json ?? null;
          tool.output_text = output;
          tool.raw = ev;
          break;
        }
        case "assistant_chunk": {
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (!fragment) break;
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
          g.assistant.content += fragment;
          break;
        }
        case "assistant_complete": {
          const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.assistant_complete_at = ev.created_at;
          if (full) g.assistant.content = full;
          g.assistant.is_complete = true;
          break;
        }
        case "thought_chunk": {
          const fragment = String(ev.payload_json?.content_fragment ?? "");
          if (!fragment) break;
          if (!g.assistant) {
            g.assistant = {
              kind: "assistant",
              id: `assistant-${g.key}`,
              created_at: ev.created_at,
              content: "",
              thought: "",
              is_complete: false,
            };
          }
          g.thought_first_at = g.thought_first_at ?? ev.created_at;
          g.thought_last_at = ev.created_at;
          g.assistant.thought += fragment;
          break;
        }
        case "tool_call":
        case "tool_call_update":
        case "tool_result": {
          const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
          const toolCallId =
            String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
          if (!toolCallId) {
            debugEvents.push(ev);
            break;
          }
          const tool = ensureTool(g, toolCallId, ev.created_at);
          tool.updated_at = ev.created_at;
          tool.updates_seen += 1;
          tool.raw = ev.payload_json;

          const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
          if (nextKind) tool.tool_kind = nextKind;

          const nextTitle = String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
          if (nextTitle) tool.title = nextTitle;
          else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

          const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
          if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
          else if (ev.event_type === "tool_result") tool.status = "completed";

          const locs = Array.isArray(update?.locations) ? update.locations : [];
          tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

          const rawInput =
            update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
          if (rawInput != null) tool.input = rawInput;

          const output =
            update?.outputText ??
            update?.output_text ??
            update?.toolCall?.outputText ??
            update?.toolCall?.output_text ??
            update?.result ??
            null;
          if (typeof output === "string") tool.output_text = output;

          break;
        }
        default: {
          break;
        }
      }
    }

    if (!g.assistant && assistant) {
      g.assistant = {
        kind: "assistant",
        id: `assistant-${g.key}`,
        created_at: assistant.created_at,
        content: assistant.content ?? "",
        thought: "",
        is_complete: true,
      };
      g.assistant_complete_at = assistant.created_at;
    } else if (g.assistant && assistant && !g.assistant.content) {
      g.assistant.content = assistant.content ?? "";
      g.assistant.is_complete = true;
      g.assistant_complete_at = assistant.created_at;
    }

    const items: ThreadItem[] = [];
    items.push(...g.toolItems);
    if (g.assistant) {
      items.push({
        ...g.assistant,
        thought_seconds: (() => {
          if (!g.thought_first_at) return undefined;
          const start = Date.parse(g.thought_first_at);
          const endRaw = g.assistant_first_at ?? g.assistant_complete_at ?? g.thought_last_at;
          if (!endRaw) return undefined;
          const end = Date.parse(endRaw);
          if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 1;
          return Math.max(1, Math.round((end - start) / 1000));
        })(),
      });
    }
    if (items.length === 0) {
      items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    }

    groups.push({ key: g.key, header: g.header, items });
  }

  // If there are no user messages (should be rare), fall back to an event-only group.
  if (groups.length === 0) {
    const g: TurnGroup = {
      key: "no-user-messages",
      header: null,
      first_at: events[0]?.created_at ?? new Date().toISOString(),
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      assistant_first_at: null,
      assistant_complete_at: null,
    };
    for (const ev of events) {
      const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
      const toolCallId =
        String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
      if (!toolCallId) continue;
      ensureTool(g, toolCallId, ev.created_at);
    }
    const items: ThreadItem[] = [...g.toolItems];
    if (items.length === 0) items.push({ kind: "spacer", id: `spacer-${g.key}`, created_at: g.first_at });
    groups.push({ key: g.key, header: g.header, items });
  }

  return { groups, debugEvents };
}

function buildThreadViewModel(events: SessionEvent[]): {
  items: ThreadItem[];
  debugEvents: SessionEvent[];
} {
  const items: ThreadItem[] = [];
  const debugEvents: SessionEvent[] = [];

  const assistantByTurn = new Map<string, Extract<ThreadItem, { kind: "assistant" }>>();
  const toolById = new Map<string, Extract<ThreadItem, { kind: "tool" }>>();

  const upsertAssistant = (turnId: string, createdAt: string) => {
    const existing = assistantByTurn.get(turnId);
    if (existing) return existing;
    const item: Extract<ThreadItem, { kind: "assistant" }> = {
      kind: "assistant",
      id: `assistant-${turnId}`,
      created_at: createdAt,
      content: "",
      thought: "",
      is_complete: false,
    };
    assistantByTurn.set(turnId, item);
    items.push(item);
    return item;
  };

  const upsertTool = (toolCallId: string, createdAt: string) => {
    const existing = toolById.get(toolCallId);
    if (existing) return existing;
    const item: Extract<ThreadItem, { kind: "tool" }> = {
      kind: "tool",
      id: `tool-${toolCallId}`,
      tool_call_id: toolCallId,
      created_at: createdAt,
      updated_at: createdAt,
      tool_kind: "tool",
      title: "Tool",
      status: "pending",
      locations: [],
      input: null,
      output_text: "",
      raw: null,
      updates_seen: 0,
    };
    toolById.set(toolCallId, item);
    items.push(item);
    return item;
  };

  for (const ev of events) {
    const id = idToString(ev.id) || `${ev.created_at}`;
    const turnId = idToString((ev as any).turn_id) || "no-turn";

    switch (ev.event_type) {
      case "user_message": {
        items.push({
          kind: "message",
          id,
          role: "user",
          content: ev.payload_json?.content ?? "",
          attachments: Array.isArray(ev.payload_json?.attachments)
            ? (ev.payload_json.attachments as MessageAttachment[])
            : [],
          created_at: ev.created_at,
        });
        break;
      }
      case "error": {
        const message = String(ev.payload_json?.message ?? "Error");
        const provider = String(ev.payload_json?.provider ?? "").trim();
        const output = provider ? `${message}\nprovider: ${provider}` : message;
        items.push({
          kind: "tool",
          id: `error-${id}`,
          tool_call_id: `error-${id}`,
          created_at: ev.created_at,
          updated_at: ev.created_at,
          tool_kind: "error",
          title: "Error",
          status: "failed",
          locations: [],
          input: ev.payload_json ?? null,
          output_text: output,
          raw: ev,
          updates_seen: 1,
        });
        break;
      }
      case "assistant_chunk": {
        const fragment = String(ev.payload_json?.content_fragment ?? "");
        if (!fragment) break;
        const item = upsertAssistant(turnId, ev.created_at);
        item.content += fragment;
        break;
      }
      case "assistant_complete": {
        const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
        const item = upsertAssistant(turnId, ev.created_at);
        // Place completed assistant responses after any preceding tool activity.
        item.created_at = ev.created_at;
        if (full) item.content = full;
        item.is_complete = true;
        break;
      }
      case "thought_chunk": {
        const fragment = String(ev.payload_json?.content_fragment ?? "");
        if (!fragment) break;
        const item = upsertAssistant(turnId, ev.created_at);
        item.thought += fragment;
        break;
      }
      case "tool_call":
      case "tool_call_update":
      case "tool_result": {
        const update = ev.payload_json?.acp_update ?? ev.payload_json ?? {};
        const toolCallId =
          String(ev.payload_json?.tool_call_id ?? update?.toolCallId ?? update?.rawInput?.call_id ?? "").trim();
        if (!toolCallId) {
          debugEvents.push(ev);
          break;
        }
        const tool = upsertTool(toolCallId, ev.created_at);
        tool.updated_at = ev.created_at;
        tool.updates_seen += 1;
        tool.raw = ev.payload_json;

        const nextKind = String(update?.kind ?? update?.toolCall?.kind ?? "").trim();
        if (nextKind) tool.tool_kind = nextKind;

        const nextTitle =
          String(update?.title ?? update?.toolCall?.title ?? update?.toolCall?.name ?? "").trim();
        if (nextTitle) tool.title = nextTitle;
        else if (tool.tool_kind && tool.title === "Tool") tool.title = humanToolKind(tool.tool_kind);

        const nextStatus = String(update?.status ?? update?.toolCall?.status ?? "").trim();
        if (nextStatus) tool.status = normalizeToolStatus(nextStatus, ev.event_type);
        else if (ev.event_type === "tool_result") tool.status = "completed";

        const locs = Array.isArray(update?.locations) ? update.locations : [];
        tool.locations = locs.map((l: any) => ({ path: l?.path, range: l?.range }));

        const input = update?.rawInput ?? update?.toolCall?.input ?? update?.args ?? null;
        if (input) tool.input = input;

        const nextOutput = extractToolOutputText(update);
        if (nextOutput) {
          tool.output_text = mergeStreamingText(tool.output_text, nextOutput);
        }
        break;
      }
      case "plan":
        break;
      case "init":
      case "notice":
      case "auth_required":
      case "done":
      case "interrupt_requested":
      case "turn_interrupted":
      case "input_queued":
        debugEvents.push(ev);
        break;
      default:
        break;
    }
  }

  const itemRank = (it: ThreadItem): number => {
    if (it.kind === "message") return 0;
    if (it.kind === "tool") return 1;
    return 2; // assistant
  };

  items.sort((a, b) => {
    const ta = a.created_at;
    const tb = b.created_at;
    const tcmp = String(ta).localeCompare(String(tb));
    if (tcmp !== 0) return tcmp;
    const rcmp = itemRank(a) - itemRank(b);
    if (rcmp !== 0) return rcmp;
    return String(a.id).localeCompare(String(b.id));
  });

  return { items, debugEvents };
}

function extractToolOutputText(update: any): string {
  const raw = update?.rawOutput?.aggregated_output ?? update?.rawOutput?.output ?? null;
  if (typeof raw === "string" && raw.trim()) return raw;

  const blocks = Array.isArray(update?.content) ? update.content : [];
  const parts: string[] = [];
  for (const b of blocks) {
    const c = b?.content ?? b;
    const t = c?.text;
    if (typeof t === "string") parts.push(t);
  }
  return parts.join("").trim();
}

function mergeStreamingText(prev: string, next: string): string {
  const p = prev ?? "";
  const n = next ?? "";
  if (!p) return n;
  if (!n) return p;
  if (n.startsWith(p)) return n;
  if (p.startsWith(n)) return p;
  return n.length >= p.length ? n : p;
}

function normalizeToolStatus(status: string, eventType: string): string {
  const s = status.toLowerCase();
  if (s === "inprogress") return "in_progress";
  if (s === "in_progress") return "in_progress";
  if (s === "running") return "in_progress";
  if (s === "pending" || s === "queued") return "pending";
  if (s === "completed" || s === "complete" || s === "ok" || s === "succeeded") return "completed";
  if (s === "failed" || s === "error") return "failed";
  if (eventType === "tool_result") return "completed";
  return s || "pending";
}

function humanToolStatus(status: string): string {
  switch (status) {
    case "pending":
      return "Pending";
    case "in_progress":
      return "Running";
    case "completed":
      return "Done";
    case "failed":
      return "Failed";
    default:
      return status || "Unknown";
  }
}

function humanToolKind(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "Run Command";
  if (k === "search") return "Search";
  if (k === "read") return "Read File";
  if (k === "edit" || k === "write") return "Edit File";
  if (k === "fetch") return "Fetch";
  if (k === "think") return "Think";
  if (k === "error") return "Error";
  return kind || "Tool";
}

function toolKindIcon(kind: string): string {
  const k = (kind || "").toLowerCase();
  if (k === "execute") return "⌘";
  if (k === "search") return "⌕";
  if (k === "read") return "⟲";
  if (k === "edit" || k === "write") return "✎";
  if (k === "fetch") return "⇣";
  if (k === "think") return "…";
  if (k === "error") return "!";
  return "▦";
}

function formatToolInput(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    const cwd = input?.cwd;
    const out: string[] = [];
    if (cwd) out.push(`cwd: ${cwd}`);
    if (cmd) out.push(`cmd: ${cmd}`);
    return out.join("\n") || JSON.stringify(input, null, 2);
  }
  if (typeof input === "string") return input;
  return JSON.stringify(input, null, 2);
}

function toolSummaryLine(toolKind: string, input: any): string {
  const k = (toolKind || "").toLowerCase();
  if (k === "execute") {
    const cmd = Array.isArray(input?.command) ? input.command.join(" ") : input?.command;
    return cmd ? truncateMiddle(String(cmd), 120) : "";
  }
  if (k === "search") {
    const q = input?.query ?? input?.pattern ?? input?.text;
    return q ? truncateMiddle(String(q), 120) : "";
  }
  if (k === "read" || k === "edit" || k === "write") {
    const path = input?.path ?? input?.file ?? input?.filename;
    return path ? truncateMiddle(String(path), 120) : "";
  }
  return "";
}

function truncateMiddle(text: string, maxLen: number): string {
  const s = String(text ?? "");
  if (s.length <= maxLen) return s;
  const head = Math.max(10, Math.floor(maxLen * 0.6));
  const tail = Math.max(10, maxLen - head - 3);
  return `${s.slice(0, head)}...${s.slice(-tail)}`;
}

function looksLikeMarkdown(text: string): boolean {
  const t = String(text ?? "");
  if (t.includes("```")) return true;
  if (/^#{1,6}\s/m.test(t)) return true;
  if (/^\s*[-*]\s+/m.test(t)) return true;
  if (/\[[^\]]+\]\([^)]+\)/.test(t)) return true;
  return false;
}

function mergeEvents(prev: SessionEvent[], incoming: SessionEvent[]): SessionEvent[] {
  const map = new Map<string, SessionEvent>();
  for (const ev of prev) {
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
  }
  for (const ev of incoming) {
    const key = idToString(ev.id) || `${ev.created_at}-${ev.event_type}`;
    map.set(key, ev);
  }
  return [...map.values()].sort((a, b) => String(a.created_at).localeCompare(String(b.created_at)));
}

type AuthMethodOption = { id: string; name: string };

type AuthUi = {
  status: "unknown" | "required" | "failed" | "authenticated";
  provider?: string;
  message?: string;
  methods: AuthMethodOption[];
};

function deriveAuthUi(events: SessionEvent[]): AuthUi {
  const fromMethodsValue = (value: any): AuthMethodOption[] => {
    const list = Array.isArray(value) ? value : [];
    return list
      .map((m: any) => ({
        id: m?.methodId ?? m?.method_id ?? m?.id,
        name: m?.name ?? m?.label ?? (m?.methodId ?? m?.method_id ?? m?.id),
      }))
      .filter((m: any) => typeof m.id === "string" && m.id.length > 0)
      .map((m: any) => ({ id: String(m.id), name: String(m.name ?? m.id) }));
  };

  let status: AuthUi["status"] = "unknown";
  let provider: string | undefined;
  let message: string | undefined;
  let methods: AuthMethodOption[] = [];

  const lastInit = [...events].reverse().find((e) => e.event_type === "init");
  const initMethods =
    lastInit?.payload_json?.auth_methods ??
    lastInit?.payload_json?.authMethods ??
    lastInit?.payload_json?.auth_methods;
  const initMethodOptions = fromMethodsValue(initMethods);

  for (const ev of events) {
    if (ev.event_type === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
      continue;
    }

    if (ev.event_type !== "notice") continue;
    const kind = ev.payload_json?.kind;
    if (kind === "auth_required") {
      status = "required";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
      methods = fromMethodsValue(ev.payload_json?.auth_methods ?? ev.payload_json?.authMethods);
    }
    if (kind === "auth_failed") {
      status = "failed";
      provider = ev.payload_json?.provider;
      message = ev.payload_json?.message;
    }
    if (kind === "auth_finished") {
      status = "authenticated";
      provider = ev.payload_json?.provider;
      message = undefined;
      methods = [];
    }
  }

  if ((status === "required" || status === "failed") && methods.length === 0) {
    methods = initMethodOptions;
  }

  return { status, provider, message, methods };
}
