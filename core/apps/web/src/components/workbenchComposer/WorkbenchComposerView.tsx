import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ArrowUp, ChevronDown, Ellipsis, Image, Square } from "lucide-react";
import { shouldSendOnEnter } from "../../utils/keyboard";
import { buildModelCatalog, formatEffortLabel, parseModelId } from "../../utils/modelEffort";
import { errorMessage } from "../../utils/errorMessage";
import {
  isReadyVisibleHarnessProviderStatus,
} from "../../utils/providerInventory";
import { shouldHydrateProviderModels } from "../../pages/workbenchShell/useWorkbenchProviders";
import { ComposerAutocompleteMenu } from "../ComposerAutocompleteMenu";
import { useComposerAutocomplete } from "../../state/useComposerAutocomplete";
import { imageFilesToMessageAttachments } from "../../utils/messageAttachments";
import type { SessionViewVerbosity } from "../../state/uiStateStore";
import { MenuTitleRow } from "./WorkbenchComposerMenu";
import { WorkbenchComposerHarnessMenu } from "./WorkbenchComposerHarnessMenu";
import {
  MENU_DESCRIPTIONS,
  attachmentDisplayName,
  buildModelsForProvider,
  clamp,
  describeContextWindow,
  deriveFullModelIdForBase,
  imageAttachmentSrc,
  labelForVerbosity,
  modelIdFromProviderOptions,
  nextAutoSeededModelId,
  pickDefaultEffort,
  shouldShowLoadingProviderModels,
} from "./WorkbenchComposer.utils";
import type { ActiveSessionProps, NewSessionProps, WorkbenchComposerProps } from "./WorkbenchComposer.types";

type OpenMenuId = "harness" | "model" | "effort" | "verbosity";

const logoClasses = (base: string, invertInDark?: boolean, invertInLight?: boolean) =>
  [base, invertInDark ? "wb-invert" : "", invertInLight ? "wb-invert-light" : ""].filter(Boolean).join(" ");

export function WorkbenchComposer(props: WorkbenchComposerProps) {
  const {
    variant,
    value,
    setValue,
    placeholder,
    inputDisabled,
    attachments,
    setAttachments,
    onAttachmentError,
    onSend,
    sendDisabled,
    sendDisabledReason,
    onInterrupt,
    isWorking,
    sessionIdForAutocomplete,
    workspaceIdForAutocomplete,
    slashCommands,
    recording,
  } = props;

  const newSession = variant === "newSession" ? (props as NewSessionProps) : null;
  const verbosity = props.verbosity ?? "default";
  const canAdjustVerbosity = variant === "newSession" && typeof props.onSetVerbosity === "function";
  const contextWindow =
    variant === "activeSession" ? (props as ActiveSessionProps).contextWindow ?? null : null;
  const showStop = !!onInterrupt && !!isWorking;
  const sendActionDisabled = !showStop && (!!sendDisabled || !!sendDisabledReason);
  const sendActionTitle = showStop ? "Stop" : sendDisabledReason ?? "Send";
  const sendActionLabel = showStop ? "Stop" : "Send";
  const contextWindowDisplay = useMemo(
    () => (variant === "activeSession" ? describeContextWindow(contextWindow) : null),
    [contextWindow, variant],
  );

  const [openMenu, setOpenMenu] = useState<OpenMenuId | null>(null);
  const [menuStyle, setMenuStyle] = useState<React.CSSProperties | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const mirrorRef = useRef<HTMLDivElement | null>(null);
  const lastHeightRef = useRef<number>(0);
  const restoreDraftTailRef = useRef(true);
  const autoSeededModelIdByProviderRef = useRef<Record<string, string>>({});
  const harnessTriggerRef = useRef<HTMLButtonElement | null>(null);
  const modelTriggerRef = useRef<HTMLButtonElement | null>(null);
  const effortTriggerRef = useRef<HTMLButtonElement | null>(null);
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
    const mirror = mirrorRef.current;
    if (!el || !mirror) return;

    const minHeightPx = variant === "newSession" ? 88 : 28;
    const maxHeightPx = variant === "newSession" ? 380 : 220;

    mirror.style.width = `${el.clientWidth}px`;
    mirror.textContent = value.length > 0 ? `${value}\n` : "\n";
    let measured = mirror.scrollHeight;
    if (!measured) {
      const styles = window.getComputedStyle(el);
      const lineHeight = Number.parseFloat(styles.lineHeight || "") || 20;
      const paddingTop = Number.parseFloat(styles.paddingTop || "") || 0;
      const paddingBottom = Number.parseFloat(styles.paddingBottom || "") || 0;
      const lines = Math.max(1, value.split("\n").length);
      measured = Math.ceil(lines * lineHeight + paddingTop + paddingBottom);
    }
    if (!measured) {
      measured = el.scrollHeight;
    }
    const next = Math.min(maxHeightPx, Math.max(minHeightPx, measured));
    const currentHeight =
      lastHeightRef.current ||
      Number.parseFloat(el.style.height || "0") ||
      el.getBoundingClientRect().height ||
      minHeightPx;
    const hasInlineHeight = el.style.height !== "";
    if (!hasInlineHeight || !Number.isFinite(currentHeight) || Math.abs(next - currentHeight) > 0.5) {
      el.style.height = `${next}px`;
      lastHeightRef.current = next;
    } else {
      lastHeightRef.current = currentHeight;
    }

    if (recording) el.scrollTop = el.scrollHeight;
  }, [recording, value, variant]);

  const clearSelectionBeforeSubmit = useCallback(() => {
    if (variant !== "newSession") return;

    setOpenMenu(null);
    autocomplete.dismiss();

    const textarea = textareaRef.current;
    if (textarea) {
      const cursor =
        textarea.selectionEnd
        ?? textarea.selectionStart
        ?? textarea.value.length;
      textarea.setSelectionRange(cursor, cursor);
      if (document.activeElement === textarea) {
        textarea.blur();
      }
    }

    window.getSelection()?.removeAllRanges();
  }, [autocomplete, variant]);

  const handleSendAction = useCallback(() => {
    if (showStop) {
      onInterrupt?.();
      return;
    }

    clearSelectionBeforeSubmit();
    onSend();
  }, [clearSelectionBeforeSubmit, onInterrupt, onSend, showStop]);

  useLayoutEffect(() => {
    resizeTextarea();
    const textarea = textareaRef.current;
    if (!textarea || !restoreDraftTailRef.current || value.length === 0) return;
    const end = textarea.value.length;
    textarea.setSelectionRange(end, end);
    textarea.scrollTop = textarea.scrollHeight;
    restoreDraftTailRef.current = false;
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
      if (target && typeof target.closest === "function" && target.closest(".wb-menu")) return;
      recomputeMenuPosition();
    };
    window.addEventListener("scroll", onAnyScroll, true);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener("resize", recomputeMenuPosition);
      window.removeEventListener("scroll", onAnyScroll, true);
    };
  }, [openMenu, recomputeMenuPosition]);

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
      const catalog = buildModelCatalog(models);
      const parsed = parseModelId((props as ActiveSessionProps).currentModelId, catalog);
      return { models, catalog, parsed, loading: false, fromProviderOptions: false };
    }

    const ns = newSession;
    const primary = ns?.draftHarness ?? null;
    if (!primary) return { models: [], catalog: buildModelCatalog([]), parsed: parseModelId(""), loading: false, fromProviderOptions: true };
    const opts = ns?.providerOptions[primary.providerId];
    const models = buildModelsForProvider(primary.providerId, opts);
    const catalog = buildModelCatalog(models);
    const parsed = parseModelId(primary.modelId, catalog);
    const loading = shouldShowLoadingProviderModels(primary.providerId, opts);
    return { models, catalog, parsed, loading, fromProviderOptions: true };
  }, [newSession, props, variant]);

  const providerIdsToEnsure = useMemo(() => {
    if (!newSession) return [];
    return [newSession.draftHarness?.providerId ?? newSession.defaultProviderId].filter(Boolean);
  }, [newSession?.defaultProviderId, newSession?.draftHarness]);

  // Proactively hydrate provider auth/config summary from providers/bootstrap.
  useEffect(() => {
    if (!newSession) return;
    for (const providerId of providerIdsToEnsure) {
      const st = newSession.providersById[providerId];
      if (!isReadyVisibleHarnessProviderStatus(st)) continue;
      newSession.ensureProviderAuthSummary(providerId).catch(() => {});
    }
  }, [newSession?.ensureProviderAuthSummary, newSession?.providerOptions, newSession?.providersById, providerIdsToEnsure]);

  // Seed the primary draft model from the daemon-backed saved preference when valid,
  // otherwise fall back to the provider-advertised current/default model.
  useEffect(() => {
    if (!newSession) return;
    const primary = newSession.draftHarness ?? null;
    if (!primary) return;
    if (primary.preferenceExplicit) return;
    const providerId = primary.providerId;
    const opts = newSession.providerOptions[providerId];
    const next = modelIdFromProviderOptions(opts);
    const previousAutoSeed = autoSeededModelIdByProviderRef.current[providerId] ?? null;
    const nextSeededModelId = nextAutoSeededModelId(primary.modelId, next, previousAutoSeed);
    if (!nextSeededModelId) return;
    autoSeededModelIdByProviderRef.current[providerId] = nextSeededModelId;
    newSession.setDraftHarness((prev) =>
      prev && prev.providerId === providerId ? { ...prev, modelId: nextSeededModelId } : prev,
    );
  }, [newSession?.draftHarness, newSession?.providerOptions, newSession?.setDraftHarness]);

  const showModelEffort = useMemo(() => {
    if (variant === "newSession") {
      return !!newSession?.draftHarness;
    }
    return true;
  }, [newSession?.draftHarness, variant]);

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
      ns.setDraftHarness((prev) =>
        prev ? { ...prev, modelId: nextFullId, preferenceExplicit: true } : prev,
      );
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
          {!activeModelData.loading ? (
            <input
              className="wb-menu-search"
              value={activeModelData.parsed.full}
              onChange={(e) => setActiveModelId(e.target.value)}
              placeholder="model_id"
              aria-label="Model id"
            />
          ) : null}
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
        invertInDark: as.harnessLogoInvert,
        invertInLight: as.harnessLogoInvertInLight,
        locked: true,
      };
    }

    const ns = props as NewSessionProps;
    const primary = ns.draftHarness ?? null;
    if (!primary) {
      return {
        label: "Select agent",
        logoSrc: "",
        invertInDark: false,
        invertInLight: false,
        locked: false,
      };
    }
    const providerId = primary.providerId;
    const info = ns.harnessCatalog.find((h) => h.id === providerId);
    const label = info?.label ?? providerId;
    return {
      label,
      logoSrc: info?.logoSrc,
      invertInDark: info?.invertInDark,
      invertInLight: info?.invertInLight,
      locked: false,
    };
  }, [props, variant]);

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
        onChange={(e) => {
          restoreDraftTailRef.current = false;
          setValue(e.target.value);
        }}
        disabled={!!inputDisabled}
        onKeyDown={(e) => {
          if (autocomplete.onKeyDown(e)) return;
          if (shouldSendOnEnter(e)) {
            e.preventDefault();
            handleSendAction();
          }
        }}
        onKeyUp={() => autocomplete.syncFromDom()}
        onClick={() => autocomplete.syncFromDom()}
        onSelect={() => autocomplete.syncFromDom()}
      />
      <div
        ref={mirrorRef}
        className="wb-composer-textarea wb-composer-mirror"
        aria-hidden="true"
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
                }}
                aria-haspopup={variant === "newSession" ? "menu" : undefined}
                aria-expanded={openMenu === "harness"}
                aria-label={harnessControl.label}
                title="Agents"
              >
                {harnessControl.logoSrc ? (
                  <img
                    className={logoClasses(
                      "wb-switcher-logo",
                      harnessControl.invertInDark,
                      harnessControl.invertInLight,
                    )}
                    src={harnessControl.logoSrc}
                    alt=""
                  />
                ) : null}
                {harnessControl.label && <span className="wb-switcher-label">{harnessControl.label}</span>}
                <ChevronDown size={14} />
              </button>
            ) : (
              <div
                className="wb-harness-display"
                role="img"
                aria-label={harnessControl.label}
                title="Agents"
              >
                {harnessControl.logoSrc ? (
                  <img
                    className={logoClasses(
                      "wb-switcher-logo",
                      harnessControl.invertInDark,
                      harnessControl.invertInLight,
                    )}
                    src={harnessControl.logoSrc}
                    alt=""
                  />
                ) : null}
              </div>
            )}
            {variant === "newSession" && openMenu === "harness" ? (
              <WorkbenchComposerHarnessMenu
                logoClasses={logoClasses}
                menuRef={menuRef}
                menuStyle={menuStyle}
                newSession={props as NewSessionProps}
                onClose={() => setOpenMenu(null)}
              />
            ) : null}
          </div>

          {/* Model */}
          {showModelEffort && (
            <div className="wb-switcher-wrap">
              <button
                type="button"
                className="wb-switcher wb-menu-trigger"
                ref={modelTriggerRef}
                onClick={() => {
                  if (variant === "newSession" && openMenu !== "model") {
                    const ns = props as NewSessionProps;
                    const providerId = ns.draftHarness?.providerId;
                    if (providerId) {
                      const opts = ns.providerOptions[providerId];
                      if (shouldHydrateProviderModels(providerId, opts, "explicit")) {
                        ns.ensureProviderAuthSummary(providerId, { trigger: "explicit" }).catch(() => {});
                      }
                    }
                  }
                  setOpenMenu((v) => (v === "model" ? null : "model"));
                }}
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
              onAttachmentError?.(null);
              try {
                const next = await imageFilesToMessageAttachments(files);
                setAttachments((prev) => [...prev, ...next]);
              } catch (error: unknown) {
                onAttachmentError?.(errorMessage(error));
              } finally {
                e.target.value = "";
              }
            }}
          />

          {/* First-launch gate: keep dictation wiring in place, but hide composer mic UI for now. */}
          {/*
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
          */}

          <button
            type="button"
            className="wb-send"
            onClick={handleSendAction}
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
