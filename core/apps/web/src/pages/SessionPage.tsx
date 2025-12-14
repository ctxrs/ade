import { forwardRef, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { GroupedVirtuoso, GroupedVirtuosoHandle, Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Link, useParams } from "react-router-dom";
import {
  cancelSession,
  deleteMessage,
  Message,
  MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  setSessionMode,
  setSessionModel,
  authenticateSession,
  idToString,
  interruptSession,
} from "../api/client";
import { useOpenSession, useSessionCacheSnapshot, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";
import { DiffReviewPane } from "../components/DiffReviewPane";

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
  useOpenSession(id ?? "", { watchDiff: true });

  const entry = useSessionEntry(id ?? "");
  const session: Session | null = entry?.session ?? null;
  const events: SessionEvent[] = entry?.events ?? [];
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
  const workbenchThreadView = useMemo(() => buildWorkbenchThreadViewModel(events), [eventsKey]);

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

  const effortOptions = useMemo(() => {
    const currentBase = String(currentModelId).split("/")[0];
    const efforts = new Set<string>();
    for (const o of modelOptions) {
      const [base, effort] = String(o.id).split("/");
      if (base === currentBase && effort) efforts.add(effort);
    }
    return [...efforts];
  }, [modelOptions, currentModelId]);

  const threadActivityCount = variant === "workbench" ? wbFlatItems.length : threadItems.length;

  useEffect(() => {
    if (!atBottom && threadActivityCount > 0) {
      setHasNewActivity(true);
    }
  }, [threadActivityCount, atBottom]);

  const sendNow = async () => {
    if (!id || !input.trim()) return;
    await postMessage(id, input.trim(), undefined, draftAttachments);
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
    });
  };

  const commandSuggestions = useMemo(() => {
    const provider = session?.provider_id;
    const common = ["/compact"];
    if (provider === "codex") {
      return ["/review", "/review-branch", "/review-commit", "/init", "/compact", "/logout"];
    }
    if (provider === "claude") {
      return ["/login", "/logout", "/compact", "/help"];
    }
    if (provider === "gemini") {
      return common;
    }
    return common;
  }, [session?.provider_id]);

  const showCommandSuggestions = input.trimStart().startsWith("/");
  const virtuosoStyle = variant === "workbench" ? ({ flex: 1 } as const) : ({ height: "70vh" } as const);

  const wrapperClass = variant === "workbench" ? "wb-session-view" : "page split";
  const leftClass = variant === "workbench" ? "wb-session-left" : "left";

  const renderThreadItem = (item: ThreadItem) => {
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
    <div className={wrapperClass}>
      <div className={leftClass}>
        {entry?.error && (
          <div className="banner">
            <span className="error">{entry.error}</span>
          </div>
        )}
        {entry?.loading && !entry?.error && <div className="banner">Loading…</div>}
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
            {modelOptions.length > 0 && (
              <label>
                Model
                <select
                  value={currentModelId}
                  onChange={async (e) => {
                    const next = e.target.value;
                    const updated = await setSessionModel(id, next);
                    supervisor.setSession(updated);
                  }}
                >
                  {modelOptions.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </label>
            )}

            {effortOptions.length > 0 && (
              <label>
                Effort
                <select
                  value={String(currentModelId).split("/")[1] ?? ""}
                  onChange={async (e) => {
                    const base = String(currentModelId).split("/")[0];
                    const next = `${base}/${e.target.value}`;
                    const updated = await setSessionModel(id, next);
                    supervisor.setSession(updated);
                  }}
                >
                  {effortOptions.map((eff) => (
                    <option key={eff} value={eff}>
                      {eff}
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

        {variant === "workbench" ? (
          <GroupedVirtuoso
            style={virtuosoStyle}
            data={wbFlatItems}
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
            itemContent={(_index, _groupIndex, item) => renderThreadItem(item)}
          />
        ) : (
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
        )}

        {hasNewActivity && (
          <button
            type="button"
            className="new-activity"
            onClick={() => {
              if (variant === "workbench") {
                groupedVirtuosoRef.current?.scrollToIndex({ index: "LAST", align: "end" });
              } else if (threadItems.length > 0) {
                virtuosoRef.current?.scrollToIndex({ index: threadItems.length - 1 });
              }
            }}
          >
            New activity ↓
          </button>
        )}

        {variant === "legacy" && <ActivityBar planEntries={planEntries} diffText={diff} />}

        {variant === "workbench" ? (
          <WorkbenchComposer
            session={session}
            modelOptions={modelOptions}
            effortOptions={effortOptions}
            currentModelId={currentModelId}
            onSetModel={async (next) => {
              if (!id) return;
              const updated = await setSessionModel(id, next);
              supervisor.setSession(updated);
            }}
            onSetEffort={async (effort) => {
              if (!id) return;
              const base = String(currentModelId).split("/")[0];
              const next = `${base}/${effort}`;
              const updated = await setSessionModel(id, next);
              supervisor.setSession(updated);
            }}
            textareaRef={textareaRef}
            input={input}
            setInput={setInput}
            onSend={sendNow}
            onInterrupt={() => id && interruptSession(id)}
            onInsertAtFile={() => {
              const p = window.prompt("Insert @ file path (relative to repo):");
              if (!p) return;
              insertIntoComposer(`@${p} `);
            }}
            onInsertSlash={() => insertIntoComposer("/")}
            attachments={draftAttachments}
            setAttachments={setDraftAttachments}
          />
        ) : (
          <form onSubmit={onSend} className="composer">
            <div className="row">
              <button
                type="button"
                onClick={() => {
                  const p = window.prompt("Insert @ file path (relative to repo):");
                  if (!p) return;
                  insertIntoComposer(`@${p} `);
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
                    const next: MessageAttachment[] = [];
                    for (const f of files) {
                      const dataUrl = await readFileAsDataUrl(f);
                      const idx = dataUrl.indexOf("base64,");
                      if (idx === -1) continue;
                      next.push({
                        kind: "image",
                        mime_type: f.type || "image/*",
                        data_base64: dataUrl.slice(idx + "base64,".length),
                        name: f.name,
                      });
                    }
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
                    if (a.kind !== "image") return null;
                    const src = `data:${a.mime_type};base64,${a.data_base64}`;
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
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  sendNow();
                }
              }}
            />
            {showCommandSuggestions && (
              <div className="card">
                <div className="muted">Commands</div>
                <div className="row" style={{ flexWrap: "wrap" }}>
                  {commandSuggestions.map((c) => (
                    <button
                      key={c}
                      type="button"
                      onClick={() => {
                        setInput(c + " ");
                        requestAnimationFrame(() => textareaRef.current?.focus());
                      }}
                    >
                      {c}
                    </button>
                  ))}
                </div>
              </div>
            )}
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
            onDiffUpdated={(d) => id && supervisor.setDiff(id, d)}
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
      </div>
    </button>
  );
}

function WorkbenchComposer({
  session,
  modelOptions,
  effortOptions,
  currentModelId,
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
              {a.kind === "image" ? (a.name ?? "image") : "attachment"} ×
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
          if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
            e.preventDefault();
            onSend();
            return;
          }
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            onSend();
          }
        }}
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
                    {eff}
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
              const next: MessageAttachment[] = [];
              for (const f of files) {
                const dataUrl = await readFileAsDataUrl(f);
                const idx = dataUrl.indexOf("base64,");
                if (idx === -1) continue;
                next.push({
                  kind: "image",
                  mime_type: f.type || "image/*",
                  data_base64: dataUrl.slice(idx + "base64,".length),
                  name: f.name,
                });
              }
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
            if (a.kind !== "image") return null;
            const src = `data:${a.mime_type};base64,${a.data_base64}`;
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

function Markdown({ content }: { content: string }) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        code({ inline, className, children }) {
          const match = /language-(\w+)/.exec(className || "");
          const codeString = String(children).replace(/\n$/, "");
          if (inline) {
            return <code className={className}>{children}</code>;
          }
          return (
            <div className="codeblock">
              <div className="row code-header">
                <span className="muted">{match?.[1] ?? "code"}</span>
                <button
                  type="button"
                  onClick={() => navigator.clipboard.writeText(codeString)}
                >
                  Copy
                </button>
              </div>
              <SyntaxHighlighter style={oneDark} language={match?.[1]} PreTag="div">
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

function buildWorkbenchThreadViewModel(events: SessionEvent[]): WorkbenchThreadView {
  type InternalGroup = {
    key: string;
    header: WorkbenchTurnHeader | null;
    first_at: string;
    toolItems: Array<Extract<ThreadItem, { kind: "tool" }>>;
    toolById: Map<string, Extract<ThreadItem, { kind: "tool" }>>;
    assistant: Extract<ThreadItem, { kind: "assistant" }> | null;
    thought_first_at: string | null;
    thought_last_at: string | null;
    assistant_first_at: string | null;
    assistant_complete_at: string | null;
  };

  const debugEvents: SessionEvent[] = [];
  const groupsInOrder: InternalGroup[] = [];
  const groupsByKey = new Map<string, InternalGroup>();

  let currentTurnKey: string | null = null;

  const ensureGroup = (key: string, createdAt: string): InternalGroup => {
    const existing = groupsByKey.get(key);
    if (existing) return existing;
    const g: InternalGroup = {
      key,
      header: null,
      first_at: createdAt,
      toolItems: [],
      toolById: new Map(),
      assistant: null,
      thought_first_at: null,
      thought_last_at: null,
      assistant_first_at: null,
      assistant_complete_at: null,
    };
    groupsByKey.set(key, g);
    groupsInOrder.push(g);
    return g;
  };

  const ensureAssistant = (g: InternalGroup, createdAt: string) => {
    if (g.assistant) return g.assistant;
    g.assistant = {
      kind: "assistant",
      id: `assistant-${g.key}`,
      created_at: createdAt,
      content: "",
      thought: "",
      is_complete: false,
    };
    return g.assistant;
  };

  const ensureTool = (g: InternalGroup, toolCallId: string, createdAt: string) => {
    const existing = g.toolById.get(toolCallId);
    if (existing) return existing;
    const t: Extract<ThreadItem, { kind: "tool" }> = {
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

  const groupKeyForEvent = (ev: SessionEvent) => {
    const rawTurnId = idToString((ev as any).turn_id) || "";
    if (rawTurnId) {
      currentTurnKey = rawTurnId;
      return rawTurnId;
    }
    return currentTurnKey ?? "no-turn";
  };

  for (const ev of events) {
    const eventId = idToString(ev.id) || `${ev.created_at}`;
    const turnKey = groupKeyForEvent(ev);
    const g = ensureGroup(turnKey, ev.created_at);
    if (ev.created_at < g.first_at) g.first_at = ev.created_at;

    switch (ev.event_type) {
      case "user_message": {
        // If we ever see multiple user messages for the same turn id, split them into separate groups.
        if (g.header && g.header.id !== eventId) {
          const splitKey = `${turnKey}-u${eventId}`;
          currentTurnKey = splitKey;
          const g2 = ensureGroup(splitKey, ev.created_at);
          g2.header = {
            id: eventId,
            content: ev.payload_json?.content ?? "",
            attachments: Array.isArray(ev.payload_json?.attachments)
              ? (ev.payload_json.attachments as MessageAttachment[])
              : [],
            created_at: ev.created_at,
          };
          break;
        }
        g.header = {
          id: eventId,
          content: ev.payload_json?.content ?? "",
          attachments: Array.isArray(ev.payload_json?.attachments)
            ? (ev.payload_json.attachments as MessageAttachment[])
            : [],
          created_at: ev.created_at,
        };
        break;
      }
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
        const a = ensureAssistant(g, ev.created_at);
        g.assistant_first_at = g.assistant_first_at ?? ev.created_at;
        a.content += fragment;
        break;
      }
      case "assistant_complete": {
        const full = String(ev.payload_json?.full_content ?? ev.payload_json?.content ?? "");
        const a = ensureAssistant(g, ev.created_at);
        g.assistant_complete_at = ev.created_at;
        if (full) a.content = full;
        a.is_complete = true;
        break;
      }
      case "thought_chunk": {
        const fragment = String(ev.payload_json?.content_fragment ?? "");
        if (!fragment) break;
        const a = ensureAssistant(g, ev.created_at);
        g.thought_first_at = g.thought_first_at ?? ev.created_at;
        g.thought_last_at = ev.created_at;
        a.thought += fragment;
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

        const rawInput = update?.rawInput ?? update?.toolCall?.rawInput ?? update?.toolCall?.input ?? update?.input ?? null;
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
        // Non-thread display events still belong to the current group for ordering, but are not rendered.
        break;
      }
    }
  }

  const groups = groupsInOrder.map((g) => ({
    key: g.key,
    header: g.header,
    items: [
      ...g.toolItems,
      ...(g.assistant
        ? [
            {
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
            } satisfies Extract<ThreadItem, { kind: "assistant" }>,
          ]
        : []),
    ],
  }));

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

function readFileAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error("Failed to read file"));
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.readAsDataURL(file);
  });
}
