import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SessionSupervisor } from "./sessionSupervisor";
import type { Message, Session, SessionEvent } from "../api/client";

vi.mock("../api/client", () => {
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
  return {
    idToString,
    getDaemonBaseUrl: () => null,
    getHealth: vi.fn(async () => ({ daemon_url: "http://127.0.0.1:4399" })),
    getSession: vi.fn(),
    listQueue: vi.fn(),
    listMessages: vi.fn(),
    listSessionEvents: vi.fn(),
    listSessionEventsPage: vi.fn(),
    trackDiff: vi.fn(async () => ({ diff: "" })),
  };
});

import { getSession, listMessages, listQueue, listSessionEvents, listSessionEventsPage } from "../api/client";

describe("SessionSupervisor", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.resetAllMocks();
  });

  it("refreshes Messages when polling backfills a turn boundary event", async () => {
    const sessionId = "session-1";
    const trackId = "track-1";

    const session: Session = {
      id: { 0: sessionId },
      track_id: { 0: trackId },
      task_id: { 0: "task-1" },
      workspace_id: { 0: "ws-1" },
      worktree_id: { 0: "wt-1" },
      provider_id: "fake",
      model_id: "fake-model",
      agent_role: "assistant",
      status: "idle",
    };

    (getSession as any).mockResolvedValue(session);
    (listSessionEvents as any).mockResolvedValue([]);
    (listQueue as any).mockResolvedValue([]);

    const initialMessages: Message[] = [
      {
        id: { 0: "m1" },
        session_id: { 0: sessionId },
        role: "user",
        content: "hi",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];
    const updatedMessages: Message[] = [
      ...initialMessages,
      {
        id: { 0: "m2" },
        session_id: { 0: sessionId },
        role: "assistant",
        content: "my name is ...",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];

    let messagesCalls = 0;
    (listMessages as any).mockImplementation(async () => {
      messagesCalls += 1;
      return messagesCalls === 1 ? initialMessages : updatedMessages;
    });

    let pageCalls = 0;
    (listSessionEventsPage as any).mockImplementation(async () => {
      pageCalls += 1;
      if (pageCalls !== 1) return [];
      const ev: SessionEvent = {
        id: { 0: "e1" },
        session_id: { 0: sessionId },
        event_type: "done",
        payload_json: {},
        created_at: new Date().toISOString(),
      };
      return [ev];
    });

    const sup = new SessionSupervisor();
    const close = sup.openSession(sessionId);

    await Promise.resolve();
    await Promise.resolve();

    await vi.advanceTimersByTimeAsync(1600);
    await Promise.resolve();
    await Promise.resolve();

    const snap = sup.getSnapshot();
    expect(snap.sessions[sessionId]?.messages.length).toBe(2);

    close();
  });

  it("refreshes Messages when polling backfills assistant_complete", async () => {
    const sessionId = "session-1";
    const trackId = "track-1";

    const session: Session = {
      id: { 0: sessionId },
      track_id: { 0: trackId },
      task_id: { 0: "task-1" },
      workspace_id: { 0: "ws-1" },
      worktree_id: { 0: "wt-1" },
      provider_id: "fake",
      model_id: "fake-model",
      agent_role: "assistant",
      status: "idle",
    };

    (getSession as any).mockResolvedValue(session);
    (listSessionEvents as any).mockResolvedValue([]);
    (listQueue as any).mockResolvedValue([]);

    const initialMessages: Message[] = [
      {
        id: { 0: "m1" },
        session_id: { 0: sessionId },
        role: "user",
        content: "hi",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];
    const updatedMessages: Message[] = [
      ...initialMessages,
      {
        id: { 0: "m2" },
        session_id: { 0: sessionId },
        role: "assistant",
        content: "my name is ...",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];

    let messagesCalls = 0;
    (listMessages as any).mockImplementation(async () => {
      messagesCalls += 1;
      return messagesCalls === 1 ? initialMessages : updatedMessages;
    });

    let pageCalls = 0;
    (listSessionEventsPage as any).mockImplementation(async () => {
      pageCalls += 1;
      if (pageCalls !== 1) return [];
      const ev: SessionEvent = {
        id: { 0: "e1" },
        session_id: { 0: sessionId },
        event_type: "assistant_complete",
        payload_json: {},
        created_at: new Date().toISOString(),
      };
      return [ev];
    });

    const sup = new SessionSupervisor();
    const close = sup.openSession(sessionId);

    await Promise.resolve();
    await Promise.resolve();

    await vi.advanceTimersByTimeAsync(1600);
    await Promise.resolve();
    await Promise.resolve();

    const snap = sup.getSnapshot();
    expect(snap.sessions[sessionId]?.messages.length).toBe(2);

    close();
  });

  it("parses WS message frames delivered as Blob-like objects and refreshes on assistant_complete", async () => {
    const sessionId = "session-1";
    const trackId = "track-1";

    const session: Session = {
      id: { 0: sessionId },
      track_id: { 0: trackId },
      task_id: { 0: "task-1" },
      workspace_id: { 0: "ws-1" },
      worktree_id: { 0: "wt-1" },
      provider_id: "fake",
      model_id: "fake-model",
      agent_role: "assistant",
      status: "idle",
    };

    (getSession as any).mockResolvedValue(session);
    (listSessionEvents as any).mockResolvedValue([]);
    (listSessionEventsPage as any).mockResolvedValue([]);
    (listQueue as any).mockResolvedValue([]);

    const initialMessages: Message[] = [
      {
        id: { 0: "m1" },
        session_id: { 0: sessionId },
        role: "user",
        content: "hi",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];
    const updatedMessages: Message[] = [
      ...initialMessages,
      {
        id: { 0: "m2" },
        session_id: { 0: sessionId },
        role: "assistant",
        content: "done",
        delivery: "immediate",
        created_at: new Date().toISOString(),
      },
    ];

    let messagesCalls = 0;
    (listMessages as any).mockImplementation(async () => {
      messagesCalls += 1;
      return messagesCalls === 1 ? initialMessages : updatedMessages;
    });

    const originalWebSocket = (globalThis as any).WebSocket;
    const sockets: any[] = [];

    class FakeWebSocket {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      readyState = FakeWebSocket.CONNECTING;
      private listeners = new Map<string, Array<{ cb: (ev: any) => void; once: boolean }>>();

      constructor(public url: string) {
        sockets.push(this);
        queueMicrotask(() => {
          this.readyState = FakeWebSocket.OPEN;
          this.dispatch("open", {});
        });
      }

      addEventListener(type: string, cb: (ev: any) => void, opts?: any) {
        const once = Boolean(opts?.once);
        const list = this.listeners.get(type) ?? [];
        list.push({ cb, once });
        this.listeners.set(type, list);
      }

      send(_data: string) {}
      close() {}

      private dispatch(type: string, ev: any) {
        const list = this.listeners.get(type);
        if (!list) return;
        for (const l of [...list]) {
          l.cb(ev);
          if (l.once) {
            const idx = list.indexOf(l);
            if (idx >= 0) list.splice(idx, 1);
          }
        }
      }

      emitMessage(data: any) {
        this.dispatch("message", { data });
      }
    }

    (globalThis as any).WebSocket = FakeWebSocket as any;
    try {
      const sup = new SessionSupervisor();
      const close = sup.openSession(sessionId);

      await Promise.resolve();
      await Promise.resolve();

      await (sup as any).openWebSocket("ws://example.test");
      expect(sockets.length).toBe(1);

      const frame = {
        text: async () =>
          JSON.stringify({
            id: { 0: "e1" },
            session_id: { 0: sessionId },
            event_type: "assistant_complete",
            payload_json: {},
            created_at: new Date().toISOString(),
          }),
      };

      sockets[0].emitMessage(frame);
      await Promise.resolve();
      await Promise.resolve();

      const snap = sup.getSnapshot();
      expect(snap.sessions[sessionId]?.events.length).toBe(1);
      expect(snap.sessions[sessionId]?.messages.length).toBe(2);

      close();
    } finally {
      (globalThis as any).WebSocket = originalWebSocket;
    }
  });
});
