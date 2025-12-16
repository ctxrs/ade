import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type React from "react";
import { createPortal } from "react-dom";
import { blobUrl, type MessageAttachment, type ProviderOptions, type ProviderStatus } from "../api/client";
import { shouldSendOnEnter } from "../utils/keyboard";
import { buildModelCatalog, composeModelId, formatEffortLabel, parseModelId } from "../utils/modelEffort";
import { ComposerAutocompleteMenu } from "./ComposerAutocompleteMenu";
import { useComposerAutocomplete, type SlashCommandDescriptor } from "../state/useComposerAutocomplete";
import {
  IconArrowUp,
  IconAt,
  IconChevronDown,
  IconInfo,
  IconImage,
  IconLaptop,
  IconMic,
  IconSlash,
  IconStop,
} from "./workbenchIcons";
import type { HarnessCatalogEntry } from "../utils/harnessCatalog";
import { imageFilesToInlineAttachments } from "../utils/messageAttachments";

export type WorkbenchModeId = "default" | "research" | "plan" | "review";
export type WorkbenchEnvTarget = "local" | "worktree" | "container";

const MENU_DESCRIPTIONS = {
  harness: `Agent harnesses are the low-level wrappers around models that provide the basic plumbing to allow the model to interact with the workspace. This normally includes features like filesystem access, shell access, configurations to set up MCP servers, and more. Despite similiarities between them, different harnesses will have varying tools, capabilities, and performance - even if used with the same underlying models. From here, you can install agent harnesses you haven't used before, switch between them for new tasks, and even run multiple agent harnesses in parallel on the same task. This can be useful to compare performance or to survey multiple different approaches to the same problem.`,
  model: `You can switch between different models here. Model selection offers a tradeoff between cost, latency, and intelligence - but it also offers an opportunity to leverage the differences in their weights for collaboration. Even if two different models score similarly on popular coding benchmarks, they might have different "habits" - or biases. This means that if you are working on a pernicious bug fix, you might want multiple different models to both look at the problem from a different angle.`,
  effort: `Some models have a "thinking effort" or "reasoning effort" setting, while others do not. The effort level simply corresponds to how many tokens a model spends on thinking while solving a problem. Models that offer high or extra high can sometimes be very powerful, at the expense of latency and cost. However, you can also experience an unintended negative consequence from extra high thinking: if the model is emitting lots of thinking tokens that don't add much value, this will cause the context window to fill up faster (not just from thinking tokens alone, but also from more excessive tool calls like reading files). Performance on coding tasks declines as context increases beyond the minimum context needed to solve the problem, so effort level is a key lever in tuning your agent for optimal performance.`,
  mode: `Modes are basically just prompts, sometimes combined with access limitations. For example, the review mode is nothing more than prompting the agent to tell it to review the code and putting it in a read-only access level. That sounds fairly simple, but there is a hidden benefit: developers who build agent harnesses and models in conjunction will often train their custom model to use their bespoke harness, including its different modes. So in a way, this prompt can be more than just a regular prompt. It is a special prompt than has been trained on via reinforcement learning to achieve certain outcomes. For example, OpenAI trained their codex model to use their codex harness in review mode, so as to output only high value review comments with priority details. If you give the exact same prompt to a model that has not undergone the same RL, it will emit much less useful review comments. We recommend using RPIR (Research, Plan, Implement, Review) pattern for most changes except for small and easy ones.`,
  isolation: `If you are new to using an ADE, you likely have your agents running in Local isolation mode, which basically means no isolation. In local mode, your agents work on the locally checked-out branch and could collide with other agents or your own changes. This results in dirty working branches, possible collisions, and risks of lost changes. An improvement is using git worktrees. They create a totally separate workspace that is disk-efficient. You can spin up many agents to all work in different worktrees and they won't collide with eachother. When they are done, you can approve and merge their changes back into the local working branch. This is a very powerful and resource-efficient isolation pattern. Finally there is container-level isolation. This is the most isolated environment, but it consumes many more resources: you have to run all of your processes again inside the container, and you have to copy all of the disk space. Despite the additional overhead, container-based isolation is most powerful when your agents need to test your application on the same ports. A simple example: if you have a key part of your application that always runs on port 3000 and you want your agent to be able to test it, worktrees won't save you: only one process can serve requests on that port. Containers solve that problem because you could have many agents working in different containers, and they can all claim their own port 3000 as theirs without worrying about collisions. Depending on your application, you may or may not need this. Containers of course also improved security isolation properties which worktrees cannot.`,
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
        <IconInfo size={14} />
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

type OpenMenuId = "harness" | "model" | "effort" | "mode" | "env";

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
  providerOptions: Record<string, ProviderOptions | undefined>;
  ensureProviderOptions: (providerId: string) => Promise<ProviderOptions | undefined>;

  draftTracks: DraftTrack[];
  setDraftTracks: React.Dispatch<React.SetStateAction<DraftTrack[]>>;
  defaultProviderId: string;
  useMultipleAgents: boolean;
  setUseMultipleAgents: (next: boolean) => void;

  envTarget: WorkbenchEnvTarget;
  setEnvTarget: (next: WorkbenchEnvTarget) => void;
};

type ActiveSessionProps = SharedProps & {
  variant: "activeSession";
  harnessLabel: string;
  harnessLogoSrc?: string;
  harnessLogoInvert?: boolean;

  envLabel: string;

  availableModels: Array<{ id: string; name?: string }>;
  currentModelId: string;
  onSetModelId: (next: string) => void;
};

export type WorkbenchComposerProps = NewSessionProps | ActiveSessionProps;

function insertTextAtCursor(value: string, insert: string, el: HTMLTextAreaElement | null) {
  if (!el) return { nextText: value + insert, nextCursor: (value + insert).length };
  const start = el.selectionStart ?? value.length;
  const end = el.selectionEnd ?? value.length;
  const nextText = value.slice(0, start) + insert + value.slice(end);
  const nextCursor = start + insert.length;
  return { nextText, nextCursor };
}

function labelForMode(mode: WorkbenchModeId): string {
  if (mode === "default") return "Default";
  if (mode === "research") return "Research";
  if (mode === "plan") return "Plan";
  return "Review";
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

  const [openMenu, setOpenMenu] = useState<OpenMenuId | null>(null);
  const [menuStyle, setMenuStyle] = useState<React.CSSProperties | null>(null);

  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const harnessTriggerRef = useRef<HTMLButtonElement | null>(null);
  const modelTriggerRef = useRef<HTMLButtonElement | null>(null);
  const effortTriggerRef = useRef<HTMLButtonElement | null>(null);
  const modeTriggerRef = useRef<HTMLButtonElement | null>(null);
  const envTriggerRef = useRef<HTMLButtonElement | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const autocomplete = useComposerAutocomplete({
    sessionId: sessionIdForAutocomplete,
    workspaceId: workspaceIdForAutocomplete ?? null,
    value,
    setValue,
    textareaRef,
    slashCommands,
  });

  useEffect(() => {
    if (variant !== "activeSession") return;
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "0px";
    const next = Math.min(220, Math.max(28, el.scrollHeight));
    el.style.height = `${next}px`;
  }, [variant, value]);

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
      return envTriggerRef.current;
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
        overflowY = "auto";
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
    window.addEventListener("scroll", recomputeMenuPosition, true);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", recomputeMenuPosition);
      window.removeEventListener("scroll", recomputeMenuPosition, true);
    };
  }, [openMenu, recomputeMenuPosition]);

  const onInsert = useCallback(
    (text: string) => {
      const el = textareaRef.current;
      const out = insertTextAtCursor(value, text, el);
      setValue(out.nextText);
      requestAnimationFrame(() => {
        if (!el) return;
        el.focus();
        el.setSelectionRange(out.nextCursor, out.nextCursor);
        requestAnimationFrame(() => autocomplete.syncFromDom());
      });
    },
    [autocomplete, setValue, value],
  );

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

  const activeModelData = useMemo(() => {
    if (variant === "activeSession") {
      const models = (props as ActiveSessionProps).availableModels;
      const catalog = buildModelCatalog(models);
      const parsed = parseModelId((props as ActiveSessionProps).currentModelId, catalog);
      return { models, catalog, parsed, loading: false, fromProviderOptions: false };
    }

    const ns = newSession;
    const primary = ns?.draftTracks[0] ?? null;
    if (!primary) return { models: [], catalog: buildModelCatalog([]), parsed: parseModelId(""), loading: false, fromProviderOptions: true };
    const opts = ns?.providerOptions[primary.providerId];
    const models = buildModelsForProvider(primary.providerId, opts);
    const catalog = buildModelCatalog(models);
    const parsed = parseModelId(primary.modelId, catalog);
    const loading = !opts;
    return { models, catalog, parsed, loading, fromProviderOptions: true };
  }, [newSession, props, variant]);

  const providerIdsToEnsure = useMemo(() => {
    if (!newSession) return [];
    if (newSession.useMultipleAgents) {
      return [...new Set(newSession.draftTracks.map((t) => t.providerId).filter(Boolean))];
    }
    return [newSession.draftTracks[0]?.providerId ?? newSession.defaultProviderId].filter(Boolean);
  }, [newSession?.defaultProviderId, newSession?.draftTracks, newSession?.useMultipleAgents]);

  // Proactively probe provider options so the model list (and effort variants) populate
  // without requiring the user to manually focus/expand a config panel.
  useEffect(() => {
    if (!newSession) return;
    for (const providerId of providerIdsToEnsure) {
      if (newSession.providerOptions[providerId]) continue;
      if (!(newSession.providersById[providerId]?.installed ?? false)) continue;
      newSession.ensureProviderOptions(providerId).catch(() => {});
    }
  }, [newSession?.ensureProviderOptions, newSession?.providerOptions, newSession?.providersById, providerIdsToEnsure]);

  // Seed the primary draft model from provider-advertised defaults (when available),
  // so the UI shows the current model + effort (e.g. `gpt-5.2/xhigh`) immediately.
  useEffect(() => {
    if (!newSession) return;
    const primary = newSession.draftTracks[0] ?? null;
    if (!primary) return;
    if (newSession.draftTracks.length !== 1) return;
    if (primary.modelId.trim().length > 0) return;
    const opts = newSession.providerOptions[primary.providerId];
    const next = modelIdFromProviderOptions(opts);
    if (!next) return;
    newSession.setDraftTracks((prev) => prev.map((t, idx) => (idx === 0 ? { ...t, modelId: next } : t)));
  }, [newSession?.draftTracks, newSession?.providerOptions, newSession?.setDraftTracks]);

  const showModelEffort = useMemo(() => {
    if (variant === "newSession") {
      return (newSession?.draftTracks.length ?? 0) === 1;
    }
    return true;
  }, [newSession?.draftTracks.length, variant]);

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

  const envControl = useMemo(() => {
    if (variant === "activeSession") {
      return { label: (props as ActiveSessionProps).envLabel, locked: true };
    }
    const ns = props as NewSessionProps;
    const label = ns.envTarget === "worktree" ? "Worktree" : ns.envTarget === "local" ? "Local" : "Container";
    return { label, locked: false };
  }, [props, variant]);

  const envMenu =
    variant === "newSession" ? (
      <div className="wb-menu wb-exec-menu" role="menu" ref={menuRef} style={menuStyle ?? undefined}>
        <div className="wb-menu-top">
          <MenuTitleRow title="Isolation" description={MENU_DESCRIPTIONS.isolation} tooltipId="wb-menu-tooltip-isolation" />
        </div>
        <button
          type="button"
          className={`wb-menu-item ${(props as NewSessionProps).envTarget === "worktree" ? "wb-menu-item-active" : ""}`}
          onClick={() => {
            (props as NewSessionProps).setEnvTarget("worktree");
            setOpenMenu(null);
          }}
        >
          Worktree
        </button>
        <button
          type="button"
          className={`wb-menu-item ${(props as NewSessionProps).envTarget === "local" ? "wb-menu-item-active" : ""}`}
          onClick={() => {
            (props as NewSessionProps).setEnvTarget("local");
            setOpenMenu(null);
          }}
        >
          Local
        </button>
        <button type="button" className="wb-menu-item" disabled>
          Container (soon)
        </button>
      </div>
    ) : null;

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
    const label = ns.draftTracks.length === 1 ? (info?.label ?? providerId) : `${ns.draftTracks.length} tracks`;
    return { label, logoSrc: info?.logoSrc, invert: info?.invertInDark, locked: false };
  }, [props, variant]);

  const [harnessSearch, setHarnessSearch] = useState("");
  const [expandedHarnessId, setExpandedHarnessId] = useState<string | null>(null);

  const toggleHarness = useCallback(
    (providerId: string) => {
      if (variant !== "newSession") return;
      const ns = props as NewSessionProps;
      const installed = ns.providersById[providerId]?.installed ?? false;
      if (!installed) return;
      ns.setDraftTracks((prev) => {
        const has = prev.some((t) => t.providerId === providerId);
        if (!ns.useMultipleAgents) {
          if (has) return prev;
          return [{ key: `t${Date.now()}`, label: "", providerId, modelId: "" }];
        }
        if (has) {
          const next = prev.filter((t) => t.providerId !== providerId);
          return next.length > 0
            ? next
            : [{ key: `t${Date.now()}`, label: "", providerId: ns.defaultProviderId, modelId: "" }];
        }
        return [...prev, { key: `t${Date.now()}`, label: "", providerId, modelId: "" }];
      });
      ns.ensureProviderOptions(providerId).catch(() => {});
      if (!ns.useMultipleAgents) {
        setOpenMenu(null);
        setExpandedHarnessId(null);
      }
    },
    [props, variant],
  );

  const updateTrackModel = useCallback(
    (key: string, nextFull: string) => {
      if (variant !== "newSession") return;
      const ns = props as NewSessionProps;
      ns.setDraftTracks((prev) => prev.map((t) => (t.key === key ? { ...t, modelId: nextFull } : t)));
    },
    [props, variant],
  );

  const addTrackForProvider = useCallback(
    (providerId: string) => {
      if (variant !== "newSession") return;
      const ns = props as NewSessionProps;
      ns.setDraftTracks((prev) => [...prev, { key: `t${Date.now()}`, label: "", providerId, modelId: "" }]);
    },
    [props, variant],
  );

  const removeTrackByKey = useCallback(
    (key: string) => {
      if (variant !== "newSession") return;
      const ns = props as NewSessionProps;
      ns.setDraftTracks((prev) => {
        const next = prev.filter((t) => t.key !== key);
        return next.length > 0 ? next : prev;
      });
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
          <label className="wb-menu-toggle">
            <span>Use Multiple Agents</span>
            <input
              type="checkbox"
              checked={(props as NewSessionProps).useMultipleAgents}
              onChange={(e) => {
                setExpandedHarnessId(null);
                (props as NewSessionProps).setUseMultipleAgents(e.target.checked);
              }}
            />
            <span className="wb-toggle" aria-hidden="true" />
          </label>
        </div>

        {(() => {
          const ns = props as NewSessionProps;
          const q = harnessSearch.trim().toLowerCase();
          const all = ns.harnessCatalog.concat(ns.providersById["fake"] ? ([{ id: "fake", label: "Fake" }] as any) : []);
          const filtered = q
            ? all.filter((h: any) => String(h.id).toLowerCase().includes(q) || String(h.label).toLowerCase().includes(q))
            : all;
          if (filtered.length === 0) return <div className="wb-menu-empty">No matching agents.</div>;

          const counts: Record<string, number> = {};
          for (const t of ns.draftTracks) counts[t.providerId] = (counts[t.providerId] ?? 0) + 1;

          return filtered.map((h: any) => {
            const id = String(h.id);
            const label = String(h.label ?? id);
            const installed = ns.providersById[id]?.installed ?? false;
            const count = counts[id] ?? 0;
            const checked = count > 0;
            const expanded = expandedHarnessId === id;
            const canConfigureModels = ns.useMultipleAgents && ns.draftTracks.length > 1 && checked;
            const rows = ns.draftTracks.filter((t) => t.providerId === id);

            const opts = ns.providerOptions[id];
            const models = buildModelsForProvider(id, opts);
            const catalog = buildModelCatalog(models);

            return (
              <div key={id} className={`wb-harness-row ${installed ? "" : "wb-disabled"}`}>
                <button type="button" className="wb-harness-row-main" onClick={() => toggleHarness(id)} disabled={!installed}>
                  <span className={`wb-check ${checked ? "wb-check-on" : ""}`} aria-hidden="true">
                    {checked ? "✓" : ""}
                  </span>
                  {h.logoSrc ? (
                    <img className={`wb-harness-logo ${h.invertInDark ? "wb-invert" : ""}`} src={h.logoSrc} alt="" />
                  ) : (
                    <span className="wb-harness-logo-fallback" aria-hidden="true" />
                  )}
                  <span className="wb-harness-name">{label}</span>
                  <span className="wb-harness-right">
                    <span className="wb-harness-count">{count > 0 ? `${count}x` : ""}</span>
                  </span>
                </button>

                {checked && (
                  <button
                    type="button"
                    className="wb-harness-expand wb-menu-trigger"
                    onClick={() => {
                      if (!canConfigureModels) return;
                      setExpandedHarnessId((prev) => (prev === id ? null : id));
                      ns.ensureProviderOptions(id).catch(() => {});
                    }}
                    disabled={!canConfigureModels}
                    title={canConfigureModels ? "Configure models" : "Enable multi-agent to configure"}
                  >
                    <IconChevronDown size={14} />
                  </button>
                )}

                {expanded && canConfigureModels && (
                  <div className="wb-harness-config">
                    {rows.map((t) => {
                      const parsed = parseModelId(t.modelId, catalog);
                      const base = parsed.base || catalog.baseIds[0] || "";
                      const efforts = catalog.effortsByBase[base] ?? [];
                      const eff = parsed.effort;
                      return (
                        <div key={t.key} className="wb-harness-track">
                          <div className="wb-harness-track-left">
                            <div className="wb-harness-track-title">Track</div>
                            {catalog.baseIds.length > 0 ? (
                              <>
                                <select
                                  className="wb-harness-model-select"
                                  value={base}
                                  onFocus={() => ns.ensureProviderOptions(id).catch(() => {})}
                                  onChange={(e) => {
                                    const nextBase = e.target.value;
                                    const next = deriveFullModelIdForBase(catalog, nextBase, eff);
                                    updateTrackModel(t.key, next);
                                  }}
                                >
                                  {catalog.baseIds.map((b) => (
                                    <option key={b} value={b}>
                                      {catalog.displayNameByBase[b] ?? b}
                                    </option>
                                  ))}
                                </select>
                                {efforts.length > 0 && (
                                  <select
                                    className="wb-harness-model-select"
                                    value={eff ?? pickDefaultEffort(efforts) ?? ""}
                                    onChange={(e) => {
                                      const nextEff = e.target.value || "";
                                      const next = deriveFullModelIdForBase(catalog, base, nextEff || null);
                                      updateTrackModel(t.key, next);
                                    }}
                                  >
                                    {efforts.map((x) => (
                                      <option key={x} value={x}>
                                        {formatEffortLabel(x)}
                                      </option>
                                    ))}
                                  </select>
                                )}
                              </>
                            ) : (
                              <input
                                className="wb-harness-model-input"
                                value={t.modelId}
                                placeholder={opts ? "model_id" : "Loading models…"}
                                onFocus={() => ns.ensureProviderOptions(id).catch(() => {})}
                                onChange={(e) => updateTrackModel(t.key, e.target.value)}
                              />
                            )}
                          </div>
                          <div className="wb-harness-track-right">
                            <button type="button" className="wb-harness-mini" onClick={() => addTrackForProvider(id)} title="Add another track">
                              +
                            </button>
                            <button
                              type="button"
                              className="wb-harness-mini"
                              onClick={() => removeTrackByKey(t.key)}
                              title="Remove track"
                              disabled={rows.length <= 1}
                            >
                              −
                            </button>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}

                {!installed && <div className="wb-harness-note">Not installed</div>}
              </div>
            );
          });
        })()}
      </div>
    ) : null;

  return (
    <div
      ref={rootRef}
      className={variant === "newSession" ? "wb-composer-card wb-new-composer-card" : "wb-composer wb-active-composer"}
    >
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
        className={variant === "newSession" ? "wb-composer-textarea" : "wb-composer-textarea wb-active-textarea"}
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
            <button
              type="button"
              className="wb-switcher wb-menu-trigger"
              ref={harnessTriggerRef}
              onClick={() => {
                if (variant !== "newSession") return;
                setOpenMenu((v) => (v === "harness" ? null : "harness"));
                setHarnessSearch("");
                setExpandedHarnessId(null);
              }}
              aria-haspopup={variant === "newSession" ? "menu" : undefined}
              aria-expanded={openMenu === "harness"}
              disabled={variant !== "newSession"}
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
              <span className="wb-switcher-label">{harnessControl.label}</span>
              {variant === "newSession" && <IconChevronDown size={14} />}
            </button>
            {openMenu === "harness" && harnessMenu}
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
                <IconChevronDown size={14} />
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
                <IconChevronDown size={14} />
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
              <IconChevronDown size={14} />
            </button>
            {openMenu === "mode" && modeMenu}
          </div>

          {/* Isolation */}
          <div className="wb-switcher-wrap">
            <button
              type="button"
              className="wb-switcher wb-menu-trigger"
              ref={envTriggerRef}
              onClick={() => {
                if (envControl.locked) return;
                setOpenMenu((v) => (v === "env" ? null : "env"));
              }}
              aria-haspopup={!envControl.locked ? "menu" : undefined}
              aria-expanded={openMenu === "env"}
              disabled={envControl.locked}
              title="Isolation"
            >
              <span className="wb-switcher-icon">
                <IconLaptop size={14} />
              </span>
              <span className="wb-switcher-label">{envControl.label}</span>
              {!envControl.locked && <IconChevronDown size={14} />}
            </button>
            {openMenu === "env" && envMenu}
          </div>
        </div>

        <div className="wb-action-row">
          {onInterrupt ? (
            <button type="button" className="wb-icon wb-menu-trigger" onClick={onInterrupt} aria-label="Interrupt" title="Interrupt">
              <IconStop size={14} />
            </button>
          ) : null}

          <button
            type="button"
            className="wb-icon wb-menu-trigger"
            onClick={() => onInsert("@")}
            aria-label="Insert @"
            title="Insert @"
          >
            <IconAt size={14} />
          </button>
          <button
            type="button"
            className="wb-icon wb-menu-trigger"
            onClick={() => onInsert("/")}
            aria-label="Insert /"
            title="Insert /"
          >
            <IconSlash size={14} />
          </button>

          <button
            type="button"
            className="wb-icon"
            onClick={() => fileInputRef.current?.click()}
            title="Attach image"
            aria-label="Attach image"
          >
            <IconImage size={14} />
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
            {recording ? <IconStop size={14} /> : <IconMic size={14} />}
          </button>

          <button
            type="button"
            className="wb-send"
            onClick={onSend}
            disabled={!!sendDisabled || !!sendDisabledReason}
            title={sendDisabledReason ?? "Send"}
            aria-label="Send"
          >
            <IconArrowUp size={14} />
          </button>
        </div>
      </div>
    </div>
  );
}
