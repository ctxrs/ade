import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type React from "react";
import { createPortal } from "react-dom";
import { ArrowUp, ChevronDown, Ellipsis, Image, Info, Mic, Square } from "lucide-react";
import {
  authenticateProviderForWorkspace,
  blobUrl,
  type MessageAttachment,
  type ProviderOptions,
  type ProviderStatus,
  verifyProviderForWorkspace,
} from "../api/client";
import { shouldSendOnEnter } from "../utils/keyboard";
import { buildModelCatalog, composeModelId, formatEffortLabel, parseModelId } from "../utils/modelEffort";
import { ComposerAutocompleteMenu } from "./ComposerAutocompleteMenu";
import { useComposerAutocomplete, type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import type { HarnessCatalogEntry } from "../utils/harnessCatalog";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";
import type { SessionViewVerbosity } from "../state/uiStateStore";

export type WorkbenchModeId = "default" | "research" | "plan" | "review";
export type ContextWindowInfo = {
  windowTokens?: number;
  usedTokens?: number;
  remainingTokens?: number;
  remainingFraction?: number;
};

const MENU_DESCRIPTIONS = {
  harness: `Agent harnesses are the low-level wrappers around models that provide the basic plumbing to allow the model to interact with the workspace. This normally includes features like filesystem access, shell access, configurations to set up MCP servers, and more. Despite similarities between them, different harnesses will have varying tools, capabilities, and performance - even if used with the same underlying models. From here, you can install agent harnesses you haven't used before and pick which harness will start the task.`,
  model: `You can switch between different models here. Model selection offers a tradeoff between cost, latency, and intelligence. If you want a different model for a new task, choose it here and it will apply to the next session you start.`,
  effort: `Some models have a "thinking effort" or "reasoning effort" setting, while others do not. The effort level simply corresponds to how many tokens a model spends on thinking while solving a problem. Models that offer high or extra high can sometimes be very powerful, at the expense of latency and cost. However, you can also experience an unintended negative consequence from extra high thinking: if the model is emitting lots of thinking tokens that don't add much value, this will cause the context window to fill up faster (not just from thinking tokens alone, but also from more excessive tool calls like reading files). Performance on coding tasks declines as context increases beyond the minimum context needed to solve the problem, so effort level is a key lever in tuning your agent for optimal performance.`,
  mode: `Modes are basically just prompts, sometimes combined with access limitations. For example, the review mode is nothing more than prompting the agent to tell it to review the code and putting it in a read-only access level. That sounds fairly simple, but there is a hidden benefit: developers who build agent harnesses and models in conjunction will often train their custom model to use their bespoke harness, including its different modes. So in a way, this prompt can be more than just a regular prompt. It is a special prompt than has been trained on via reinforcement learning to achieve certain outcomes. For example, OpenAI trained their codex model to use their codex harness in review mode, so as to output only high value review comments with priority details. If you give the exact same prompt to a model that has not undergone the same RL, it will emit much less useful review comments. We recommend using RPIR (Research, Plan, Implement, Review) pattern for most changes except for small and easy ones.`,
  verbosity: `Verbosity controls how much activity is shown during a turn. Terse hides tools and thoughts, default shows summaries and thoughts, and verbose will eventually expand full tool details.`,
} as const;

function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

function MenuInfoTooltip({ title, description, tooltipId }: { title: string; description: string; tooltipId: string }) {
  const buttonRef = useRef<HTMLButtonElement | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  const [open, setOpen] = useState(false);
  const [ready, setReady] = useState(false);
  const [style, setStyle] = useState<React.CSSProperties | null>(null);

  const cancelClose = useCallback(() => {
    if (closeTimerRef.current == null) return;
    window.clearTimeout(closeTimerRef.current);
    closeTimerRef.current = null;
  }, []);

  const requestClose = useCallback(() => {
    cancelClose();
    closeTimerRef.current = window.setTimeout(() => {
      setOpen(false);
    }, 180);
  }, [cancelClose]);

  useLayoutEffect(() => {
    if (!open) {
      setReady(false);
      setStyle(null);
      return;
    }

    setReady(false);
    setStyle({ position: "fixed", left: 0, top: 0, visibility: "hidden" });

    const update = () => {
      const btn = buttonRef.current;
      const tip = tooltipRef.current;
      if (!btn || !tip) return;

      const anchor = btn.getBoundingClientRect();
      const tipRect = tip.getBoundingClientRect();
      const viewportW = window.innerWidth;
      const viewportH = window.innerHeight;
      const margin = 10;
      const gap = 8;

      const maxWidth = Math.max(220, Math.min(440, viewportW - margin * 2));
      const effectiveW = Math.min(tipRect.width, maxWidth);

      const downTop = anchor.bottom + gap;
      const availableDown = viewportH - margin - downTop;
      const availableUp = anchor.top - margin - gap;
      const pickUp = (h: number) => availableDown < h && availableUp > availableDown;
      let shouldOpenUp = pickUp(tipRect.height);
      let maxHeight = Math.min(320, Math.max(0, shouldOpenUp ? availableUp : availableDown));
      let effectiveH = Math.min(tipRect.height, maxHeight);

      const revisedShouldOpenUp = pickUp(effectiveH);
      if (revisedShouldOpenUp !== shouldOpenUp) {
        shouldOpenUp = revisedShouldOpenUp;
        maxHeight = Math.min(320, Math.max(0, shouldOpenUp ? availableUp : availableDown));
        effectiveH = Math.min(tipRect.height, maxHeight);
      }

      const upTop = anchor.top - gap - effectiveH;
      const rawTop = shouldOpenUp ? upTop : downTop;
      const top = clamp(rawTop, margin, viewportH - margin - effectiveH);

      const preferredLeft = anchor.right - effectiveW;
      const left = clamp(preferredLeft, margin, viewportW - margin - effectiveW);

      setStyle({
        position: "fixed",
        left,
        top,
        maxWidth,
        maxHeight,
        overflow: "auto",
        visibility: "visible",
      });
      setReady(true);
    };

    const raf = window.requestAnimationFrame(update);
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [open]);

  useEffect(() => {
    return () => {
      if (closeTimerRef.current != null) window.clearTimeout(closeTimerRef.current);
    };
  }, []);

  const tooltip =
    open && typeof document !== "undefined"
      ? createPortal(
          <div
            ref={tooltipRef}
            id={tooltipId}
            className="wb-menu-tooltip"
            role="tooltip"
            data-open={ready ? "true" : "false"}
            style={style ?? undefined}
            onMouseEnter={() => {
              cancelClose();
              setOpen(true);
            }}
            onMouseLeave={requestClose}
          >
            {description}
          </div>,
          document.body,
        )
      : null;

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="wb-menu-info-btn"
        aria-label={`About ${title}`}
        aria-describedby={open ? tooltipId : undefined}
        onMouseEnter={() => {
          cancelClose();
          setOpen(true);
        }}
        onMouseLeave={requestClose}
        onFocus={() => {
          cancelClose();
          setOpen(true);
        }}
        onBlur={requestClose}
      >
        <Info size={14} />
      </button>
      {tooltip}
    </>
  );
}

function MenuTitleRow({
  title,
  description,
  tooltipId,
}: {
  title: string;
  description: string;
  tooltipId: string;
}) {
  return (
    <div className="wb-menu-title-row">
      <div className="wb-menu-title">{title}</div>
      <div className="wb-menu-info">
        <MenuInfoTooltip title={title} description={description} tooltipId={tooltipId} />
      </div>
    </div>
  );
}

function imageAttachmentSrc(a: MessageAttachment): string {
  return a.kind === "image_ref" ? blobUrl(a.blob_id) : `data:${a.mime_type};base64,${a.data_base64}`;
}

function attachmentDisplayName(name?: string | null) {
  const n = String(name ?? "").trim();
  if (!n) return "image";
  return n.split(/[\\/]/).pop() || "image";
}

export type DraftTrack = {
  key: string;
  label: string;
  providerId: string;
  modelId: string;
};

type OpenMenuId = "harness" | "model" | "effort" | "mode" | "verbosity";

type SharedProps = {
  variant: "newSession" | "activeSession";

  value: string;
  setValue: (next: string) => void;
  placeholder: string;
  inputDisabled?: boolean;

  sessionIdForAutocomplete: string | null;
  workspaceIdForAutocomplete?: string | null;
  slashCommands: SlashCommandDescriptor[];

  attachments: MessageAttachment[];
  setAttachments: React.Dispatch<React.SetStateAction<MessageAttachment[]>>;

  onSend: () => void;
  sendDisabledReason?: string | null;
  sendDisabled?: boolean;

  onInterrupt?: (() => void) | null;
  isWorking?: boolean;
  verbosity?: SessionViewVerbosity;
  onSetVerbosity?: (next: SessionViewVerbosity) => void;

  modeId: WorkbenchModeId;
  setModeId: (next: WorkbenchModeId) => void;

  recording?: boolean;
  recordDisabledReason?: string | null;
  onToggleRecording?: (() => void) | null;
};

type NewSessionProps = SharedProps & {
  variant: "newSession";
  harnessCatalog: HarnessCatalogEntry[];
  providersById: Record<string, ProviderStatus>;
  providerInstallsById: Record<string, { installId: string; state: "running" | "succeeded" | "failed"; pct: number | null } | undefined>;
  onInstallProvider: (providerId: string) => void;
  onInstallAllProviders: () => void;
  installAllBusy?: boolean;
  providerOptions: Record<string, ProviderOptions | undefined>;
  ensureProviderOptions: (providerId: string, opts?: { force?: boolean }) => Promise<ProviderOptions | undefined>;

  draftTracks: DraftTrack[];
  setDraftTracks: React.Dispatch<React.SetStateAction<DraftTrack[]>>;
  defaultProviderId: string;
};

type ActiveSessionProps = SharedProps & {
  variant: "activeSession";
  harnessLabel: string;
  harnessLogoSrc?: string;
  harnessLogoInvert?: boolean;

  availableModels: Array<{ id: string; name?: string }>;
  currentModelId: string;
  onSetModelId: (next: string) => void;

  contextWindow?: ContextWindowInfo | null;
};

export type WorkbenchComposerProps = NewSessionProps | ActiveSessionProps;

function labelForMode(mode: WorkbenchModeId): string {
  if (mode === "default") return "Default";
  if (mode === "research") return "Research";
  if (mode === "plan") return "Plan";
  return "Review";
}

function labelForVerbosity(level: SessionViewVerbosity): string {
  if (level === "terse") return "Terse";
  if (level === "verbose") return "Verbose";
  return "Default";
}

function formatTokenCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) {
    const scaled = value / 1_000_000;
    const fixed = scaled >= 10 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}m`;
  }
  if (value >= 1_000) {
    const scaled = value / 1_000;
    const fixed = scaled >= 100 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}k`;
  }
  return `${Math.round(value)}`;
}

function formatUsedTokenCount(value: number): string {
  if (!Number.isFinite(value)) return "0";
  if (value >= 1_000_000) {
    const scaled = value / 1_000_000;
    const fixed = scaled >= 10 ? scaled.toFixed(0) : scaled.toFixed(1);
    return `${fixed.replace(/\.0$/, "")}m`;
  }
  if (value >= 1_000) {
    const rounded = Math.round(value / 1_000);
    return `${rounded}k`;
  }
  return `${Math.round(value)}`;
}

function buildModelsFromProviderOptions(opts?: ProviderOptions): Array<{ id: string; name?: string }> {
  const raw = opts?.models;
  if (!raw) return [];
  const list = (raw as any)?.availableModels ?? (raw as any)?.available_models ?? (raw as any)?.models ?? raw;
  if (!Array.isArray(list)) return [];
  return list
    .map((m: any) => ({
      id: String(m?.modelId ?? m?.model_id ?? m?.id ?? m?.name ?? "").trim(),
      name: typeof m?.name === "string" ? m.name : undefined,
    }))
    .filter((m) => m.id.length > 0);
}

const FALLBACK_MODELS_BY_PROVIDER: Record<string, Array<{ id: string; name?: string }>> = {
  gemini: [
    { id: "gemini-2.0-flash", name: "Gemini 2.0 Flash" },
    { id: "gemini-2.0-flash-lite", name: "Gemini 2.0 Flash Lite" },
    { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro" },
    { id: "gemini-1.5-pro", name: "Gemini 1.5 Pro" },
    { id: "gemini-1.5-flash", name: "Gemini 1.5 Flash" },
  ],
};


function buildModelsForProvider(providerId: string, opts?: ProviderOptions): Array<{ id: string; name?: string }> {
  const models = buildModelsFromProviderOptions(opts);
  if (models.length > 0) return models;
  return FALLBACK_MODELS_BY_PROVIDER[providerId] ?? [];
}

function pickDefaultEffort(efforts: string[]): string | null {
  if (efforts.includes("medium")) return "medium";
  return efforts[0] ?? null;
}

function deriveFullModelIdForBase(
  catalog: ReturnType<typeof buildModelCatalog>,
  base: string,
  preferredEffort: string | null,
): string {
  const efforts = catalog.effortsByBase[base] ?? [];
  if (efforts.length === 0) return base;
  const eff = preferredEffort && efforts.includes(preferredEffort) ? preferredEffort : pickDefaultEffort(efforts);
  if (!eff) return base;
  const mapped = catalog.fullIdByBaseEffort[base]?.[eff];
  return mapped ?? composeModelId(base, eff);
}

function modelIdFromProviderOptions(opts?: ProviderOptions): string | null {
  const raw = opts?.models as any;
  if (!raw || typeof raw !== "object") return null;
  const current = raw.currentModelId ?? raw.current_model_id;
  if (typeof current === "string" && current.trim().length > 0) return current.trim();
  const list = raw.availableModels ?? raw.available_models ?? raw.models ?? [];
  if (!Array.isArray(list) || list.length === 0) return null;
  const first = list[0] as any;
  const id = first?.modelId ?? first?.model_id ?? first?.id ?? first?.name;
  return typeof id === "string" && id.trim().length > 0 ? id.trim() : null;
}

export function WorkbenchComposer(props: WorkbenchComposerProps) {
  const {
    variant,
    value,
    setValue,
    placeholder,
    inputDisabled,
    attachments,
    setAttachments,
    onSend,
    sendDisabled,
    sendDisabledReason,
    onInterrupt,
    isWorking,
    modeId,
    setModeId,
    sessionIdForAutocomplete,
    workspaceIdForAutocomplete,
    slashCommands,
    recording,
    recordDisabledReason,
    onToggleRecording,
  } = props;

  const newSession = variant === "newSession" ? (props as NewSessionProps) : null;
  const verbosity = props.verbosity ?? "default";
  const canAdjustVerbosity = variant === "newSession" && typeof props.onSetVerbosity === "function";
  const contextWindow =
    variant === "activeSession" ? (props as ActiveSessionProps).contextWindow ?? null : null;
  const hasDraft = value.trim().length > 0 || attachments.length > 0;
  const showStop = !!onInterrupt && !!isWorking && !hasDraft;
  const sendActionDisabled = !showStop && (!!sendDisabled || !!sendDisabledReason);
  const sendActionTitle = showStop ? "Stop" : sendDisabledReason ?? "Send";
  const sendActionLabel = showStop ? "Stop" : "Send";
  const contextWindowDisplay = useMemo(() => {
    if (!contextWindow?.windowTokens) return null;
    let usedTokens = contextWindow.usedTokens;
    if (usedTokens == null && contextWindow.remainingTokens != null) {
      usedTokens = contextWindow.windowTokens - contextWindow.remainingTokens;
    }
    if (usedTokens == null && contextWindow.remainingFraction != null) {
      usedTokens = Math.round(contextWindow.windowTokens * (1 - contextWindow.remainingFraction));
    }
    if (usedTokens == null) return null;

    const windowTokens = Math.max(1, Math.round(contextWindow.windowTokens));
    const clampedUsed = Math.max(0, Math.min(windowTokens, Math.round(usedTokens)));
    const fraction = clampedUsed / windowTokens;
    const percent = Math.max(0, Math.min(100, Math.round(fraction * 100)));
    const usedLabel = formatUsedTokenCount(clampedUsed);
    const windowLabel = formatTokenCount(windowTokens);
    const summary = `${percent}% · ${usedLabel}/${windowLabel}`;
    const title = `Context Window: ${summary}`;

    return {
      percent,
      usedLabel,
      windowLabel,
      title,
      summary,
    };
  }, [contextWindow]);

  const [openMenu, setOpenMenu] = useState<OpenMenuId | null>(null);
  const [menuStyle, setMenuStyle] = useState<React.CSSProperties | null>(null);

  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const harnessTriggerRef = useRef<HTMLButtonElement | null>(null);
  const modelTriggerRef = useRef<HTMLButtonElement | null>(null);
  const effortTriggerRef = useRef<HTMLButtonElement | null>(null);
  const modeTriggerRef = useRef<HTMLButtonElement | null>(null);
  const verbosityTriggerRef = useRef<HTMLButtonElement | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const autocomplete = useComposerAutocomplete({
    sessionId: sessionIdForAutocomplete,
    workspaceId: workspaceIdForAutocomplete ?? null,
    value,
    setValue,
    textareaRef,
    slashCommands,
  });

  const resizeTextarea = useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;

    const minHeightPx = variant === "newSession" ? 88 : 28;
    const maxHeightPx = variant === "newSession" ? 380 : 220;

    el.style.height = "0px";
    const next = Math.min(maxHeightPx, Math.max(minHeightPx, el.scrollHeight));
    el.style.height = `${next}px`;

    if (recording) el.scrollTop = el.scrollHeight;
  }, [recording, variant]);

  useLayoutEffect(() => {
    resizeTextarea();
  }, [resizeTextarea, value]);

  useEffect(() => {
    if (!openMenu) return;
    const onPointerDown = (e: PointerEvent) => {
      const root = rootRef.current;
      if (!root) return;
      const target = e.target as Node;
      if (!root.contains(target)) {
        setOpenMenu(null);
        return;
      }
      const el = e.target as Element | null;
      if (el && (el.closest(".wb-menu") || el.closest(".wb-menu-trigger"))) return;
      setOpenMenu(null);
    };
    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [openMenu]);

  const getTriggerForMenu = useCallback(
    (id: OpenMenuId): HTMLButtonElement | null => {
      if (id === "harness") return harnessTriggerRef.current;
      if (id === "model") return modelTriggerRef.current;
      if (id === "effort") return effortTriggerRef.current;
      if (id === "mode") return modeTriggerRef.current;
      if (id === "verbosity") return verbosityTriggerRef.current;
      return null;
    },
    [],
  );

  const recomputeMenuPosition = useCallback(() => {
    if (!openMenu) return;
    const menuEl = menuRef.current;
    const triggerEl = getTriggerForMenu(openMenu);
    if (!menuEl || !triggerEl) return;

    const margin = 10;
    const viewportW = window.innerWidth;
    const viewportH = window.innerHeight;

    const triggerRect = triggerEl.getBoundingClientRect();
    const menuRect = menuEl.getBoundingClientRect();
    const menuW = Math.max(160, menuRect.width);
    const menuH = Math.max(40, menuRect.height);

    let left = triggerRect.left;
    let top = triggerRect.bottom + 8;
    let maxHeight: number | null = null;
    let overflowY: React.CSSProperties["overflowY"] = "visible";

    if (openMenu === "harness") {
      left = triggerRect.right + 10;
      top = triggerRect.top + triggerRect.height / 2 - menuH / 2;

      if (left + menuW > viewportW - margin) {
        left = Math.max(margin, viewportW - margin - menuW);
      }
      top = Math.max(margin, Math.min(top, viewportH - margin - menuH));

      const maxH = viewportH - margin * 2;
      if (menuH > maxH) {
        top = margin;
        maxHeight = maxH;
        overflowY = "hidden";
      }
    } else {
      const downTop = triggerRect.bottom + 8;
      const upTop = triggerRect.top - 8 - menuH;
      const availableDown = viewportH - margin - downTop;
      const availableUp = triggerRect.top - margin - 8;

      const shouldOpenUp = availableDown < menuH && availableUp > availableDown;
      if (shouldOpenUp) {
        const maxH = Math.max(120, availableUp);
        const usedH = Math.min(menuH, maxH);
        top = triggerRect.top - 8 - usedH;
        maxHeight = menuH > maxH ? maxH : null;
        overflowY = menuH > maxH ? "auto" : "visible";
      } else {
        top = downTop;
        const maxH = Math.max(120, availableDown);
        maxHeight = menuH > maxH ? maxH : null;
        overflowY = menuH > maxH ? "auto" : "visible";
      }

      if (left + menuW > viewportW - margin) left = viewportW - margin - menuW;
      if (left < margin) left = margin;
      if (top < margin) top = margin;
    }

    setMenuStyle({
      position: "fixed",
      left,
      top,
      maxHeight: maxHeight ?? undefined,
      overflowY,
      visibility: "visible",
    });
  }, [getTriggerForMenu, openMenu]);

  useLayoutEffect(() => {
    if (!openMenu) {
      setMenuStyle(null);
      return;
    }
    setMenuStyle({
      position: "fixed",
      left: 0,
      top: 0,
      maxHeight: undefined,
      overflowY: "visible",
      visibility: "hidden",
    });

    const raf = window.requestAnimationFrame(() => {
      recomputeMenuPosition();
    });

    window.addEventListener("resize", recomputeMenuPosition);
    const onAnyScroll = (e: Event) => {
      const target = e.target as Element | null;
      if (target && typeof (target as any).closest === "function" && target.closest(".wb-menu")) return;
      recomputeMenuPosition();
    };
    window.addEventListener("scroll", onAnyScroll, true);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", recomputeMenuPosition);
      window.removeEventListener("scroll", onAnyScroll, true);
    };
  }, [openMenu, recomputeMenuPosition]);

  const modeMenu = (
    <div className="wb-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
      <div className="wb-menu-top">
        <MenuTitleRow title="Mode" description={MENU_DESCRIPTIONS.mode} tooltipId="wb-menu-tooltip-mode" />
      </div>
      {(["default", "research", "plan", "review"] as WorkbenchModeId[]).map((m) => (
        <button
          key={m}
          type="button"
          className={`wb-menu-item ${modeId === m ? "wb-menu-item-active" : ""}`}
          onClick={() => {
            setModeId(m);
            setOpenMenu(null);
          }}
          role="menuitem"
        >
          {labelForMode(m)}
        </button>
      ))}
    </div>
  );

  const verbosityMenu = canAdjustVerbosity ? (
    <div className="wb-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
      <div className="wb-menu-top">
        <MenuTitleRow
          title="Verbosity"
          description={MENU_DESCRIPTIONS.verbosity}
          tooltipId="wb-menu-tooltip-verbosity"
        />
      </div>
      {(["terse", "default", "verbose"] as SessionViewVerbosity[]).map((level) => (
        <button
          key={level}
          type="button"
          className={`wb-menu-item ${verbosity === level ? "wb-menu-item-active" : ""}`}
          onClick={() => {
            props.onSetVerbosity?.(level);
            setOpenMenu(null);
          }}
          role="menuitem"
        >
          {labelForVerbosity(level)}
        </button>
      ))}
    </div>
  ) : null;

  const activeModelData = useMemo(() => {
    if (variant === "activeSession") {
      const models = (props as ActiveSessionProps).availableModels;
      const parsed = parseModelId((props as ActiveSessionProps).currentModelId, catalog);
      return { models, catalog, parsed, loading: false, fromProviderOptions: false };
    }

    const ns = newSession;
    const primary = ns?.draftTracks[0] ?? null;
    if (!primary) return { models: [], catalog: buildModelCatalog([]), parsed: parseModelId(""), loading: false, fromProviderOptions: true };
    const opts = ns?.providerOptions[primary.providerId];
    const models = buildModelsForProvider(primary.providerId, opts);
    const parsed = parseModelId(primary.modelId, catalog);
    const loading = !opts;
    return { models, catalog, parsed, loading, fromProviderOptions: true };
  }, [newSession, props, variant]);

  const providerIdsToEnsure = useMemo(() => {
    if (!newSession) return [];
    return [newSession.draftTracks[0]?.providerId ?? newSession.defaultProviderId].filter(Boolean);
  }, [newSession?.defaultProviderId, newSession?.draftTracks]);

  // Proactively probe provider options so the model list (and effort variants) populate
  // without requiring the user to manually focus/expand a config panel.
  useEffect(() => {
    if (!newSession) return;
    for (const providerId of providerIdsToEnsure) {
      if (newSession.providerOptions[providerId]) continue;
      const st = newSession.providersById[providerId];
      if (!(st?.installed && st.health === "ok")) continue;
      newSession.ensureProviderOptions(providerId).catch(() => {});
    }
  }, [newSession?.ensureProviderOptions, newSession?.providerOptions, newSession?.providersById, providerIdsToEnsure]);

  // Seed the primary draft model from provider-advertised defaults (when available),
  // so the UI shows the current model + effort (e.g. `gpt-5.2/xhigh`) immediately.
  useEffect(() => {
    if (!newSession) return;
    const primary = newSession.draftTracks[0] ?? null;
    if (!primary) return;
    if (primary.modelId.trim().length > 0) return;
    const opts = newSession.providerOptions[primary.providerId];
    const next = modelIdFromProviderOptions(opts);
    if (!next) return;
    newSession.setDraftTracks((prev) => prev.map((t, idx) => (idx === 0 ? { ...t, modelId: next } : t)));
  }, [newSession?.draftTracks, newSession?.providerOptions, newSession?.setDraftTracks]);

  const showModelEffort = true;

  const currentBase = activeModelData.parsed.base || activeModelData.catalog.baseIds[0] || "";
  const currentEffort = activeModelData.parsed.effort;
  const effortOptions = activeModelData.catalog.effortsByBase[currentBase] ?? [];

  const setActiveModelId = useCallback(
    (nextFullId: string) => {
      if (variant === "activeSession") {
        (props as ActiveSessionProps).onSetModelId(nextFullId);
        return;
      }
      const ns = props as NewSessionProps;
      ns.setDraftTracks((prev) => prev.map((t, idx) => (idx === 0 ? { ...t, modelId: nextFullId } : t)));
    },
    [props, variant],
  );

  const modelMenu = (
    <div className="wb-menu wb-model-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
      <div className="wb-menu-top">
        <MenuTitleRow title="Model" description={MENU_DESCRIPTIONS.model} tooltipId="wb-menu-tooltip-model" />
        <input
          className="wb-menu-search"
          value={variant === "activeSession" ? "" : ""}
          onChange={() => {}}
          placeholder={activeModelData.loading ? "Loading models…" : "Search models"}
          aria-label="Search models"
          disabled
        />
      </div>

      {activeModelData.catalog.baseIds.length > 0 ? (
        activeModelData.catalog.baseIds.map((b) => (
          <button
            key={b}
            type="button"
            className={`wb-menu-item ${b === currentBase ? "wb-menu-item-active" : ""}`}
            onClick={() => {
              const next = deriveFullModelIdForBase(activeModelData.catalog, b, currentEffort);
              setActiveModelId(next);
              setOpenMenu(null);
            }}
          >
            {activeModelData.catalog.displayNameByBase[b] ?? b}
          </button>
        ))
      ) : (
        <div className="wb-menu-empty">
          <div style={{ marginBottom: 6 }}>{activeModelData.loading ? "Loading models…" : "Enter model id"}</div>
          <input
            className="wb-menu-search"
            value={activeModelData.parsed.full}
            onChange={(e) => setActiveModelId(e.target.value)}
            placeholder="model_id"
            aria-label="Model id"
          />
        </div>
      )}
    </div>
  );

  const effortMenu = (
    <div className="wb-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
      <div className="wb-menu-top">
        <MenuTitleRow title="Effort" description={MENU_DESCRIPTIONS.effort} tooltipId="wb-menu-tooltip-effort" />
      </div>
      {effortOptions.map((eff) => (
        <button
          key={eff}
          type="button"
          className={`wb-menu-item ${eff === currentEffort ? "wb-menu-item-active" : ""}`}
          onClick={() => {
            const nextFull = deriveFullModelIdForBase(activeModelData.catalog, currentBase, eff);
            setActiveModelId(nextFull);
            setOpenMenu(null);
          }}
        >
          {formatEffortLabel(eff)}
        </button>
      ))}
    </div>
  );

  const harnessControl = useMemo(() => {
    if (variant === "activeSession") {
      const as = props as ActiveSessionProps;
      return {
        label: as.harnessLabel,
        logoSrc: as.harnessLogoSrc,
        invert: as.harnessLogoInvert,
        locked: true,
      };
    }

    const ns = props as NewSessionProps;
    const primary = ns.draftTracks[0] ?? null;
    const providerId = primary?.providerId ?? ns.defaultProviderId;
    const info = ns.harnessCatalog.find((h) => h.id === providerId);
    const label = info?.label ?? providerId;
    return { label, logoSrc: info?.logoSrc, invert: info?.invertInDark, locked: false };
  }, [props, variant]);

  const [harnessSearch, setHarnessSearch] = useState("");
  const [providerAuthBusy, setProviderAuthBusy] = useState<Record<string, boolean>>({});
  const [providerVerifyBusy, setProviderVerifyBusy] = useState<Record<string, boolean>>({});
  const [providerActionNotice, setProviderActionNotice] = useState<string | null>(null);
  const [providerActionError, setProviderActionError] = useState<string | null>(null);

  useEffect(() => {
    if (openMenu !== "harness") {
      setProviderActionNotice(null);
      setProviderActionError(null);
    }
  }, [openMenu]);
  const toggleHarness = useCallback(
    (providerId: string) => {
      if (variant !== "newSession") return;
      const ns = props as NewSessionProps;
      const st = ns.providersById[providerId];
      const installed = st?.installed === true && st.health === "ok";
      if (!installed) return;
      const current = ns.draftTracks[0]?.providerId ?? ns.defaultProviderId;
      if (current === providerId) {
        setOpenMenu(null);
        return;
      }
      ns.setDraftTracks([{ key: `t${Date.now()}`, label: "", providerId, modelId: "" }]);
      ns.ensureProviderOptions(providerId).catch(() => {});
      setOpenMenu(null);
    },
    [props, variant],
  );
  const harnessMenu =
    variant === "newSession" ? (
      <div className="wb-menu wb-harness-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
        <div className="wb-menu-top">
          <MenuTitleRow title="Harness" description={MENU_DESCRIPTIONS.harness} tooltipId="wb-menu-tooltip-harness" />
          <input
            className="wb-menu-search"
            value={harnessSearch}
            onChange={(e) => setHarnessSearch(e.target.value)}
            placeholder="Search agents"
            aria-label="Search agents"
            autoFocus
          />
          {(() => {
            const ns = props as NewSessionProps;
            const hasSupportedMissing = Object.values(ns.providersById).some(
              (st) =>
                st.details?.install_supported === "true" &&
                (!(st.installed ?? false) || st.health !== "ok"),
            );
            const busy = ns.installAllBusy ?? false;
            return (
              <button
                type="button"
                className="wb-harness-install-all"
                onClick={() => ns.onInstallAllProviders()}
                disabled={!hasSupportedMissing || busy}
                title={hasSupportedMissing ? "Install all supported harnesses" : "No supported harnesses to install"}
              >
                {busy ? "Installing…" : "Install all"}
              </button>
            );
          })()}
        </div>

        <div className="wb-harness-list">
          {(() => {
            const ns = props as NewSessionProps;
            const q = harnessSearch.trim().toLowerCase();
            const order = new Map<string, number>(ns.harnessCatalog.map((h, idx) => [h.id, idx]));
            const extras = Object.keys(ns.providersById)
              .filter((id) => !order.has(id) && ns.providersById[id]?.details?.ui_hidden !== "true")
              .map((id) => ({ id, label: id, logoSrc: "" } as any))
              .sort((a, b) => String(a.id).localeCompare(String(b.id)));

            const all = [...ns.harnessCatalog, ...extras];
            const filtered = q
              ? all.filter(
                  (h: any) =>
                    String(h.id).toLowerCase().includes(q) || String(h.label).toLowerCase().includes(q),
                )
              : all;
            if (filtered.length === 0) return <div className="wb-menu-empty">No matching agents.</div>;

            const currentProviderId = ns.draftTracks[0]?.providerId ?? ns.defaultProviderId;
            return filtered.map((h: any) => {
              const id = String(h.id);
              const label = String(h.label ?? id);
              const providerStatus = ns.providersById[id];
              const installed = providerStatus?.installed === true && providerStatus?.health === "ok";
              const installSupported = providerStatus?.details?.install_supported === "true";
              const installUi = ns.providerInstallsById[id];
              const installRunning =
                installUi?.state === "running" || ns.providersById[id]?.details?.install_running === "true";
              const installFinishing = installUi?.state === "succeeded" && !installed;
              const installBusy = installRunning || installFinishing;
              const installPct =
                installUi?.state === "succeeded"
                  ? 100
                  : typeof installUi?.pct === "number"
                    ? installUi.pct
                    : null;
              const checked = id === currentProviderId;

              const opts = ns.providerOptions[id];
              const verifyStatus = String((opts as any)?.verify?.status ?? "");

              const statusUi = (() => {
                if (!opts) return null;
                if (opts.auth_required || verifyStatus === "auth_required") {
                  return { label: "Auth required", kind: "warn" as const };
                }
                if (verifyStatus === "network_error") {
                  return { label: "Offline", kind: "warn" as const };
                }
                if (verifyStatus === "error") {
                  return { label: "Error", kind: "err" as const };
                }
                if (opts.probe_ok === false) {
                  return { label: "Unhealthy", kind: "err" as const };
                }
                return null;
              })();

              const showVerifyButton = checked && !opts?.auth_required && verifyStatus !== "ok";

              return (
                <div key={id} className={`wb-harness-row ${installed ? "" : "wb-disabled"}`}>
                  <button
                    type="button"
                    className="wb-harness-row-main"
                    onClick={() => toggleHarness(id)}
                    disabled={!installed}
                  >
                    <span className={`wb-check ${checked ? "wb-check-on" : ""}`} aria-hidden="true">
                      {checked ? "✓" : ""}
                    </span>
                    {h.logoSrc ? (
                      <img
                        className={`wb-harness-logo ${h.invertInDark ? "wb-invert" : ""}`}
                        src={h.logoSrc}
                        alt=""
                      />
                    ) : (
                      <span className="wb-harness-logo-fallback" aria-hidden="true" />
                    )}
                    <span className="wb-harness-name">{label}</span>
                    {statusUi && (
                      <span className={`wb-harness-status wb-harness-status-${statusUi.kind}`}>
                        {statusUi.label}
                      </span>
                    )}
                  </button>

                  <div className="wb-harness-actions">
                    {!installed ? (
                      <button
                        type="button"
                        className="wb-harness-install"
                        onClick={(e) => {
                          e.stopPropagation();
                          ns.onInstallProvider(id);
                        }}
                        disabled={!installSupported || installBusy}
                        title={!installSupported ? "Install not supported yet" : "Install this harness"}
                        style={
                          installBusy && installPct !== null
                            ? ({ ["--wb-install-pct" as any]: `${Math.max(0, Math.min(100, installPct))}%` } as any)
                            : undefined
                        }
                      >
                        {installBusy
                          ? installFinishing
                            ? "Finalizing…"
                            : installPct === null
                              ? "Installing…"
                              : `${Math.max(0, Math.min(100, installPct))}%`
                          : installUi?.state === "failed"
                            ? "Retry"
                            : providerStatus?.installed
                              ? "Update"
                              : "Install"}
                      </button>
                    ) : (
                      <>
                        {opts?.auth_required && (
                          <button
                            type="button"
                            className="wb-harness-auth-btn"
                            onClick={async (e) => {
                              e.stopPropagation();
                              const wsId = props.workspaceIdForAutocomplete;
                              if (!wsId) {
                                setProviderActionError("No workspace selected.");
                                return;
                              }
                              setProviderActionNotice(null);
                              setProviderActionError(null);
                              setProviderAuthBusy((prev) => ({ ...prev, [id]: true }));
                              try {
                                const resp = await authenticateProviderForWorkspace(wsId, id);
                                if (resp.status !== "ok") {
                                  setProviderActionNotice(`Authentication status: ${resp.status}`);
                                }
                              } catch (err: any) {
                                setProviderActionError(err?.message ?? String(err));
                              } finally {
                                setProviderAuthBusy((prev) => ({ ...prev, [id]: false }));
                                ns.ensureProviderOptions(id, { force: true }).catch(() => {});
                              }
                            }}
                            disabled={providerAuthBusy[id] || !props.workspaceIdForAutocomplete}
                            title="Authenticate this provider"
                          >
                            {providerAuthBusy[id] ? "Auth…" : "Authenticate"}
                          </button>
                        )}

                        {showVerifyButton && (
                          <button
                            type="button"
                            className="wb-harness-verify-btn"
                            onClick={async (e) => {
                              e.stopPropagation();
                              const wsId = props.workspaceIdForAutocomplete;
                              if (!wsId) {
                                setProviderActionError("No workspace selected.");
                                return;
                              }
                              setProviderActionNotice(null);
                              setProviderActionError(null);
                              setProviderVerifyBusy((prev) => ({ ...prev, [id]: true }));
                              try {
                                const resp = await verifyProviderForWorkspace(wsId, id);
                                if (resp.status !== "ok") {
                                  setProviderActionNotice(
                                    resp.status === "network_error"
                                      ? `Verify failed: offline/unreachable.`
                                      : `Verify failed: ${resp.status}`,
                                  );
                                }
                              } catch (err: any) {
                                setProviderActionError(err?.message ?? String(err));
                              } finally {
                                setProviderVerifyBusy((prev) => ({ ...prev, [id]: false }));
                                ns.ensureProviderOptions(id, { force: true }).catch(() => {});
                              }
                            }}
                            disabled={providerVerifyBusy[id] || !props.workspaceIdForAutocomplete}
                            title="Send a tiny prompt to confirm credentials and connectivity"
                          >
                            {providerVerifyBusy[id] ? "Verifying…" : "Verify"}
                          </button>
                        )}
                      </>
                    )}
                  </div>
                </div>
              );
            });
          })()}
        </div>

        {(providerActionNotice || providerActionError) && (
          <div className={`wb-harness-action-banner ${providerActionError ? "wb-harness-action-banner-error" : ""}`}>
            {providerActionError ?? providerActionNotice}
          </div>
        )}
      </div>
    ) : null;

  return (
    <div
      ref={rootRef}
      className={variant === "newSession" ? "wb-composer-card wb-new-composer-card" : "wb-composer wb-active-composer"}
    >
      {contextWindowDisplay && (
        <div
          className="wb-context-window"
          title={contextWindowDisplay.title}
          aria-label={contextWindowDisplay.title}
        >
          {contextWindowDisplay.summary}
        </div>
      )}
      {attachments.length > 0 && (
        <div className="wb-composer-attachments">
          {attachments.map((a, idx) => {
            if (a.kind !== "image" && a.kind !== "image_ref") return null;
            const src = imageAttachmentSrc(a);
            const name = attachmentDisplayName(a.name);
            return (
              <div key={idx} className="wb-attach-thumb" title={name}>
                <img className="wb-attach-thumb-img" src={src} alt={name} />
                <button
                  type="button"
                  className="wb-attach-thumb-remove"
                  aria-label={`Remove ${name}`}
                  title="Remove attachment"
                  onClick={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>
      )}

      <textarea
        ref={textareaRef}
        className={
          variant === "newSession"
            ? "wb-composer-textarea wb-new-composer-textarea"
            : "wb-composer-textarea wb-active-textarea"
        }
        placeholder={placeholder}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        disabled={!!inputDisabled}
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

      <div className="wb-composer-bottom">
        <div className="wb-switcher-row">
          {/* Harness */}
          <div className="wb-switcher-wrap">
            {variant === "newSession" ? (
              <button
                type="button"
                className="wb-switcher wb-menu-trigger wb-switcher-harness"
                ref={harnessTriggerRef}
                onClick={() => {
                  if (variant !== "newSession") return;
                  setOpenMenu((v) => (v === "harness" ? null : "harness"));
                  setHarnessSearch("");
                }}
                aria-haspopup={variant === "newSession" ? "menu" : undefined}
                aria-expanded={openMenu === "harness"}
                aria-label={harnessControl.label || "Harness"}
                title="Harness"
              >
                {harnessControl.logoSrc ? (
                  <img
                    className={`wb-switcher-logo ${harnessControl.invert ? "wb-invert" : ""}`}
                    src={harnessControl.logoSrc}
                    alt=""
                  />
                ) : (
                  <span className="wb-switcher-logo-fallback" />
                )}
                {harnessControl.label && <span className="wb-switcher-label">{harnessControl.label}</span>}
                <ChevronDown size={14} />
              </button>
            ) : (
              <div
                className="wb-harness-display"
                role="img"
                aria-label={harnessControl.label || "Harness"}
                title="Harness"
              >
                {harnessControl.logoSrc ? (
                  <img
                    className={`wb-switcher-logo ${harnessControl.invert ? "wb-invert" : ""}`}
                    src={harnessControl.logoSrc}
                    alt=""
                  />
                ) : (
                  <span className="wb-switcher-logo-fallback" />
                )}
              </div>
            )}
            {variant === "newSession" && openMenu === "harness" && harnessMenu}
          </div>

          {/* Model */}
          {showModelEffort && (
            <div className="wb-switcher-wrap">
              <button
                type="button"
                className="wb-switcher wb-menu-trigger"
                ref={modelTriggerRef}
                onClick={() => setOpenMenu((v) => (v === "model" ? null : "model"))}
                aria-haspopup="menu"
                aria-expanded={openMenu === "model"}
                title="Model"
              >
                <span className="wb-switcher-label">
                  {(currentBase && (activeModelData.catalog.displayNameByBase[currentBase] ?? currentBase)) || "Model"}
                </span>
                <ChevronDown size={14} />
              </button>
              {openMenu === "model" && modelMenu}
            </div>
          )}

          {/* Effort (conditional) */}
          {showModelEffort && effortOptions.length > 0 && (
            <div className="wb-switcher-wrap">
              <button
                type="button"
                className="wb-switcher wb-menu-trigger"
                ref={effortTriggerRef}
                onClick={() => setOpenMenu((v) => (v === "effort" ? null : "effort"))}
                aria-haspopup="menu"
                aria-expanded={openMenu === "effort"}
                title="Effort"
              >
                <span className="wb-switcher-label">
                  {(() => {
                    const eff = currentEffort ?? pickDefaultEffort(effortOptions);
                    return eff ? formatEffortLabel(eff) : "Effort";
                  })()}
                </span>
                <ChevronDown size={14} />
              </button>
              {openMenu === "effort" && effortMenu}
            </div>
          )}

          {/* Mode */}
          <div className="wb-switcher-wrap">
            <button
              type="button"
              className="wb-switcher wb-menu-trigger"
              ref={modeTriggerRef}
              onClick={() => setOpenMenu((v) => (v === "mode" ? null : "mode"))}
              aria-haspopup="menu"
              aria-expanded={openMenu === "mode"}
              title="Mode"
            >
              <span className="wb-switcher-label">{labelForMode(modeId)}</span>
              <ChevronDown size={14} />
            </button>
            {openMenu === "mode" && modeMenu}
          </div>

        </div>

        <div className="wb-action-row">
          {canAdjustVerbosity ? (
            <>
              <button
                type="button"
                className="wb-icon wb-menu-trigger"
                ref={verbosityTriggerRef}
                onClick={() => setOpenMenu((v) => (v === "verbosity" ? null : "verbosity"))}
                aria-haspopup="menu"
                aria-expanded={openMenu === "verbosity"}
                aria-label="Verbosity"
                title="Verbosity"
              >
                <Ellipsis size={14} />
              </button>
              {openMenu === "verbosity" && verbosityMenu}
            </>
          ) : null}

          <button
            type="button"
            className="wb-icon"
            onClick={() => fileInputRef.current?.click()}
            title="Attach image"
            aria-label="Attach image"
          >
            <Image size={14} />
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            multiple
            style={{ display: "none" }}
            onChange={async (e) => {
              const files = Array.from(e.target.files ?? []);
              const next = await imageFilesToInlineAttachments(files);
              setAttachments((prev) => [...prev, ...next]);
              e.target.value = "";
            }}
          />

          <button
            type="button"
            className={`wb-icon ${recording ? "wb-icon-active" : ""}`}
            title={recordDisabledReason ?? (recording ? "Stop recording" : "Record")}
            aria-label="Record"
            disabled={!onToggleRecording}
            onClick={() => onToggleRecording?.()}
          >
            {recording ? <Square size={14} /> : <Mic size={14} />}
          </button>

          <button
            type="button"
            className="wb-send"
            onClick={() => {
              if (showStop) {
                onInterrupt?.();
                return;
              }
              onSend();
            }}
            disabled={sendActionDisabled}
            title={sendActionTitle}
            aria-label={sendActionLabel}
          >
            {showStop ? <Square size={14} className="wb-stop-icon" /> : <ArrowUp size={14} />}
          </button>
        </div>
      </div>
    </div>
  );
}
