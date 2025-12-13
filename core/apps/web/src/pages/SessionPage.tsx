import { useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Virtuoso, VirtuosoHandle } from "react-virtuoso";
import { Link, useParams } from "react-router-dom";
import {
  cancelSession,
  deleteMessage,
  getSession,
  listQueue,
  listSessionEvents,
  Message,
  MessageAttachment,
  postMessage,
  Session,
  SessionEvent,
  setSessionMode,
  setSessionModel,
  authenticateSession,
  trackDiff,
  idToString,
  interruptSession,
} from "../api/client";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";

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
    }
  | {
      kind: "tool";
      id: string;
      created_at: string;
      event_type: string;
      payload_json: any;
    }
  | {
      kind: "meta";
      id: string;
      created_at: string;
      title: string;
      payload_json: any;
    };

export default function SessionPage() {
  const { id } = useParams<{ id: string }>();
  const [session, setSession] = useState<Session | null>(null);
  const [events, setEvents] = useState<SessionEvent[]>([]);
  const [queue, setQueue] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [draftAttachments, setDraftAttachments] = useState<MessageAttachment[]>([]);
  const [diff, setDiff] = useState<string>("");
  const [atBottom, setAtBottom] = useState(true);
  const [hasNewActivity, setHasNewActivity] = useState(false);
  const [interruptBanner, setInterruptBanner] = useState<string | null>(null);
  const [authMethodId, setAuthMethodId] = useState<string>("");
  const [authBusy, setAuthBusy] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const refreshAll = async () => {
    if (!id) return;
    const s = await getSession(id);
    setSession(s);
    setEvents(await listSessionEvents(id));
    setQueue(await listQueue(id));
    const trackId = idToString(s.track_id);
    const d = await trackDiff(trackId);
    setDiff(d.diff);
  };

  const refreshQueue = async () => {
    if (!id) return;
    setQueue(await listQueue(id));
  };

  const refreshDiff = async () => {
    if (!session) return;
    const trackId = idToString(session.track_id);
    const d = await trackDiff(trackId);
    setDiff(d.diff);
  };

  useEffect(() => {
    refreshAll();
  }, [id]);

  useEffect(() => {
    if (!id) return;
    const token = (() => {
      try {
        return sessionStorage.getItem("contextAuthToken");
      } catch {
        return null;
      }
    })();
    const qs = token ? `?token=${encodeURIComponent(token)}` : "";
    const ws = new WebSocket(
      `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/sessions/${id}/stream${qs}`,
    );
    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        setEvents((s) => [...s, data]);
        if (data.event_type === "turn_interrupted") {
          setInterruptBanner(`Interrupted at ${new Date(data.created_at).toLocaleTimeString()}.`);
        }
        if (data.event_type === "done") {
          refreshQueue();
          refreshDiff();
        }
      } catch {
        // ignore
      }
    };
    return () => ws.close();
  }, [id, session]);

  const threadItems = useMemo(() => buildThreadItems(events), [events]);

  const contextIndicator = useMemo(() => {
    const done = [...events]
      .reverse()
      .find((e) => e.event_type === "done" && e.payload_json?.context_window);
    if (!done) return null;
    return done.payload_json.context_window as {
      context_tokens_estimate: number;
      remaining_fraction: number;
    };
  }, [events]);

  const planEntries = useMemo(() => {
    const last = [...events].reverse().find((e) => e.event_type === "plan");
    const update = last?.payload_json?.acp_update ?? last?.payload_json;
    const entries = update?.entries ?? [];
    return Array.isArray(entries) ? (entries as any[]) : [];
  }, [events]);

  const authUi = useMemo(() => deriveAuthUi(events), [events]);

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
  }, [events]);

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

  useEffect(() => {
    if (!atBottom && threadItems.length > 0) {
      setHasNewActivity(true);
    }
  }, [threadItems.length, atBottom]);

  const sendNow = async () => {
    if (!id || !input.trim()) return;
    await postMessage(id, input.trim(), undefined, draftAttachments);
    setInput("");
    setDraftAttachments([]);
    await refreshQueue();
  };

  const onSend = async (e: React.FormEvent) => {
    e.preventDefault();
    await sendNow();
  };

  const onRemoveQueued = async (messageId: string) => {
    await deleteMessage(messageId);
    await refreshQueue();
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

  return (
    <div className="page split">
      <div className="left">
        {session && (
          <div className="header">
            <Link to={`/tasks/${idToString(session.task_id)}`}>← Task</Link>
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
            </div>
          </div>
        )}

        {(modelOptions.length > 0 || modeOptions.length > 0) && id && (
          <div className="card">
            {modelOptions.length > 0 && (
              <label>
                Model
                <select
                  value={currentModelId}
                  onChange={async (e) => {
                    const next = e.target.value;
                    const updated = await setSessionModel(id, next);
                    setSession(updated);
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
                    setSession(updated);
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

        {planEntries.length > 0 && <PlanPanel entries={planEntries} />}

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

        <Virtuoso
          style={{ height: "70vh" }}
          data={threadItems}
          ref={virtuosoRef}
          followOutput="auto"
          atBottomStateChange={(b) => {
            setAtBottom(b);
            if (b) setHasNewActivity(false);
          }}
          components={{
            List: (props) => <div {...props} role="list" />,
            Item: (props) => <div {...props} role="listitem" />,
          }}
          itemContent={(_, item) => <ThreadItemView item={item} />}
        />

        {hasNewActivity && (
          <button
            type="button"
            className="new-activity"
            onClick={() => {
              virtuosoRef.current?.scrollToIndex({ index: threadItems.length - 1 });
            }}
          >
            New activity ↓
          </button>
        )}

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
                          setDraftAttachments((prev) =>
                            prev.filter((_, i) => i !== idx),
                          )
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

        <div className="sr-only" aria-live="polite">
          {session && (atBottom ? "Agent output updating." : "New agent activity.")}
        </div>
      </div>

      <div className="right">
        <h2>Diff</h2>
        <pre className="diff">{diff || "No changes."}</pre>
      </div>
    </div>
  );
}

function ThreadItemView({ item }: { item: ThreadItem }) {
  switch (item.kind) {
    case "assistant":
      return (
        <AssistantEntry
          id={item.id}
          content={item.content}
          thought={item.thought}
          isComplete={item.is_complete}
        />
      );
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
    case "tool":
      return (
        <ToolEventCard
          id={item.id}
          createdAt={item.created_at}
          eventType={item.event_type}
          payload={item.payload_json}
        />
      );
    case "meta":
      return (
        <MetaEventCard
          id={item.id}
          createdAt={item.created_at}
          title={item.title}
          payload={item.payload_json}
        />
      );
  }
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
  isComplete,
}: {
  id: string;
  content: string;
  thought: string;
  isComplete: boolean;
}) {
  const [showThought, setShowThought] = useState(false);
  return (
    <div className="msg assistant">
      <div className="row">
        <div className="role">assistant</div>
        <div className="muted">{isComplete ? "complete" : "streaming…"}</div>
      </div>
      <div id={`msg-${id}`}>
        <Markdown content={content} />
      </div>
      {thought.trim() && (
        <>
          <button
            type="button"
            className="link"
            aria-expanded={showThought}
            aria-controls={`thought-${id}`}
            onClick={() => setShowThought((s) => !s)}
          >
            {showThought ? "Hide thought" : "Show thought"}
          </button>
          {showThought && (
            <pre id={`thought-${id}`} className="thought">
              {thought}
            </pre>
          )}
        </>
      )}
    </div>
  );
}

function ToolEventCard({
  id,
  createdAt,
  eventType,
  payload,
}: {
  id: string;
  createdAt: string;
  eventType: string;
  payload: any;
}) {
  const [expanded, setExpanded] = useState(false);
  const update = payload?.acp_update ?? payload;
  const toolCallId = payload?.tool_call_id ?? update?.toolCallId ?? "";
  const title = update?.title ?? update?.toolCall?.title ?? eventType;
  const status = update?.status ?? update?.toolCall?.status ?? "";
  const kind = update?.kind ?? update?.toolCall?.kind ?? "";

  return (
    <div className="card tool">
      <div className="row">
        <strong>{title}</strong>
        <span className="muted">
          {eventType}
          {kind ? ` · ${kind}` : ""}
          {status ? ` · ${status}` : ""}
        </span>
      </div>
      {toolCallId && <div className="muted">id: {toolCallId}</div>}
      <div className="row">
        <button type="button" onClick={() => setExpanded((e) => !e)}>
          {expanded ? "Hide details" : "Show details"}
        </button>
        <span className="muted">{new Date(createdAt).toLocaleTimeString()}</span>
      </div>
      {expanded && <pre className="json">{JSON.stringify(payload, null, 2)}</pre>}
    </div>
  );
}

function MetaEventCard({
  createdAt,
  title,
  payload,
}: {
  id: string;
  createdAt: string;
  title: string;
  payload: any;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="card meta">
      <div className="row">
        <strong>{title}</strong>
        <span className="muted">{new Date(createdAt).toLocaleTimeString()}</span>
      </div>
      <button type="button" onClick={() => setExpanded((e) => !e)}>
        {expanded ? "Hide" : "Show"}
      </button>
      {expanded && <pre className="json">{JSON.stringify(payload, null, 2)}</pre>}
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

function buildThreadItems(events: SessionEvent[]): ThreadItem[] {
  const items: ThreadItem[] = [];
  const assistantByTurn: Record<string, ThreadItem & { kind: "assistant" }> = {};

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
      case "assistant_chunk": {
        const fragment = ev.payload_json?.content_fragment ?? "";
        if (!assistantByTurn[turnId]) {
          const item: ThreadItem & { kind: "assistant" } = {
            kind: "assistant",
            id: `assistant-${turnId}`,
            created_at: ev.created_at,
            content: fragment,
            thought: "",
            is_complete: false,
          };
          assistantByTurn[turnId] = item;
          items.push(item);
        } else {
          assistantByTurn[turnId].content += fragment;
        }
        break;
      }
      case "assistant_complete": {
        const full = ev.payload_json?.full_content ?? ev.payload_json?.content ?? "";
        if (!assistantByTurn[turnId]) {
          const item: ThreadItem & { kind: "assistant" } = {
            kind: "assistant",
            id: `assistant-${turnId}`,
            created_at: ev.created_at,
            content: full,
            thought: "",
            is_complete: true,
          };
          assistantByTurn[turnId] = item;
          items.push(item);
        } else {
          assistantByTurn[turnId].content = full || assistantByTurn[turnId].content;
          assistantByTurn[turnId].is_complete = true;
        }
        break;
      }
      case "thought_chunk": {
        const fragment = ev.payload_json?.content_fragment ?? "";
        if (!assistantByTurn[turnId]) {
          const item: ThreadItem & { kind: "assistant" } = {
            kind: "assistant",
            id: `assistant-${turnId}`,
            created_at: ev.created_at,
            content: "",
            thought: fragment,
            is_complete: false,
          };
          assistantByTurn[turnId] = item;
          items.push(item);
        } else {
          assistantByTurn[turnId].thought += fragment;
        }
        break;
      }
      case "tool_call":
      case "tool_call_update":
      case "tool_result": {
        items.push({
          kind: "tool",
          id,
          created_at: ev.created_at,
          event_type: ev.event_type,
          payload_json: ev.payload_json,
        });
        break;
      }
      case "plan": {
        // Rendered in the plan bar above.
        break;
      }
      case "init": {
        items.push({
          kind: "meta",
          id,
          created_at: ev.created_at,
          title: "Init",
          payload_json: ev.payload_json,
        });
        break;
      }
      case "notice": {
        items.push({
          kind: "meta",
          id,
          created_at: ev.created_at,
          title: "Notice",
          payload_json: ev.payload_json,
        });
        break;
      }
      case "error": {
        items.push({
          kind: "meta",
          id,
          created_at: ev.created_at,
          title: "Error",
          payload_json: ev.payload_json,
        });
        break;
      }
      default:
        break;
    }
  }

  return items;
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
