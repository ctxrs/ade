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
  listMessages,
  Message,
  postMessage,
  Session,
  trackDiff,
  idToString,
  interruptSession,
} from "../api/client";
import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { oneDark } from "react-syntax-highlighter/dist/esm/styles/prism";

type StreamEvent = {
  event_type: string;
  payload_json: any;
  created_at: string;
};

export default function SessionPage() {
  const { id } = useParams<{ id: string }>();
  const [session, setSession] = useState<Session | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [stream, setStream] = useState<StreamEvent[]>([]);
  const [queue, setQueue] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [diff, setDiff] = useState<string>("");
  const [atBottom, setAtBottom] = useState(true);
  const [hasNewActivity, setHasNewActivity] = useState(false);
  const [interruptBanner, setInterruptBanner] = useState<string | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const virtuosoRef = useRef<VirtuosoHandle>(null);

  const refresh = async () => {
    if (!id) return;
    const s = await getSession(id);
    setSession(s);
    setMessages(await listMessages(id));
    setQueue(await listQueue(id));
    const trackId = idToString(s.track_id);
    const d = await trackDiff(trackId);
    setDiff(d.diff);
  };

  useEffect(() => {
    refresh();
  }, [id]);

  useEffect(() => {
    if (!id) return;
    const ws = new WebSocket(
      `${window.location.protocol === "https:" ? "wss" : "ws"}://${window.location.host}/api/sessions/${id}/stream`,
    );
    wsRef.current = ws;
    ws.onmessage = (ev) => {
      try {
        const data = JSON.parse(ev.data);
        setStream((s) => [...s, data]);
        if (data.event_type === "turn_interrupted") {
          setInterruptBanner(`Interrupted at ${new Date(data.created_at).toLocaleTimeString()}.`);
        }
        refresh();
      } catch {
        // ignore
      }
    };
    return () => ws.close();
  }, [id]);

  const renderedMessages = useMemo(() => {
    const assistantChunks = stream
      .filter((e) => e.event_type === "assistant_chunk")
      .map((e) => e.payload_json?.content_fragment ?? e.payload_json?.content)
      .filter(Boolean)
      .join("");
    const assistantComplete = stream
      .filter((e) => e.event_type === "assistant_complete")
      .map((e) => e.payload_json?.full_content ?? e.payload_json?.content)
      .filter(Boolean)
      .join("\n");

    const synthetic: Message[] = [];
    if (assistantChunks || assistantComplete) {
      synthetic.push({
        id: "stream",
        session_id: id ?? "",
        role: "assistant",
        content: assistantComplete || assistantChunks,
        delivery: "immediate",
        created_at: new Date().toISOString(),
      } as any);
    }
    return [...messages, ...synthetic];
  }, [messages, stream, id]);

  const contextIndicator = useMemo(() => {
    const done = [...stream]
      .reverse()
      .find((e) => e.event_type === "done" && e.payload_json?.context_window);
    if (!done) return null;
    return done.payload_json.context_window as {
      context_tokens_estimate: number;
      remaining_fraction: number;
    };
  }, [stream]);

  useEffect(() => {
    if (!atBottom && (stream.length > 0 || messages.length > 0)) {
      setHasNewActivity(true);
    }
  }, [stream.length, messages.length, atBottom]);

  const sendNow = async () => {
    if (!id || !input.trim()) return;
    await postMessage(id, input.trim());
    setInput("");
    refresh();
  };

  const onSend = async (e: React.FormEvent) => {
    e.preventDefault();
    await sendNow();
  };

  const onRemoveQueued = async (messageId: string) => {
    await deleteMessage(messageId);
    refresh();
  };

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

        {interruptBanner && <div className="banner">{interruptBanner}</div>}

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
          data={renderedMessages}
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
          itemContent={(_, m) => (
            <CollapsibleMessage message={m} />
          )}
        />

        {hasNewActivity && (
          <button
            type="button"
            className="new-activity"
            onClick={() => {
              virtuosoRef.current?.scrollToIndex({ index: renderedMessages.length - 1 });
            }}
          >
            New activity ↓
          </button>
        )}

        <form onSubmit={onSend} className="composer">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Send a message…"
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                e.preventDefault();
                sendNow();
              }
            }}
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

function CollapsibleMessage({ message }: { message: Message }) {
  const content = message.content || "";
  const lines = content.split("\n");
  const isLong = lines.length > 20 || content.length > 1500;
  const [expanded, setExpanded] = useState(!isLong);
  const shown = expanded ? content : lines.slice(0, 20).join("\n");

  return (
    <div className={`msg ${message.role}`}>
      <div className="role">{message.role}</div>
      <div id={`msg-${idToString(message.id)}`}>
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
        {shown}
      </ReactMarkdown>
      </div>
      {message.delivery === "queued" && <span className="badge">Queued</span>}
      {isLong && (
        <button
          type="button"
          className="link"
          aria-expanded={expanded}
          aria-controls={`msg-${idToString(message.id)}`}
          onClick={() => setExpanded((e) => !e)}
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}
