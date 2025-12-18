import { afterEach, describe, expect, it, vi } from "vitest";

import type { Message, Session, SessionEvent, SessionTurn } from "../api/client";

vi.mock("../api/client", () => {
  const idToString = (id: any): string => (typeof id === "string" ? id : id?.["0"]);
  return {
    idToString,
    getDaemonBaseUrl: () => null,
    getHealth: vi.fn(async () => ({ daemon_url: "http://127.0.0.1:4399" })),
    getSession: vi.fn(),
    listQueue: vi.fn(),
    listMessages: vi.fn(),
    listSessionTurnsPage: vi.fn(async () => []),
    listTurnTools: vi.fn(async () => []),
    listSessionEventsTail: vi.fn(),
    listSessionEventsPage: vi.fn(),
    trackDiff: vi.fn(async () => ({ diff: "" })),
  };
});

import {
  getSession,
  listMessages,
  listQueue,
  listSessionEventsTail,
  listSessionEventsPage,
  listSessionTurnsPage,
  listTurnTools,
} from "../api/client";

const mkSession = (sessionId: string, trackId: string): Session => ({
  id: { 0: sessionId },
  track_id: { 0: trackId },
  task_id: { 0: "task-1" },
  workspace_id: { 0: "ws-1" },
  worktree_id: { 0: "wt-1" },
  provider_id: "fake",
  model_id: "fake-model",
  agent_role: "assistant",
  status: "idle",
});

async function waitForCondition(cond: () => boolean, timeoutMs = 1000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (cond()) return;
    await new Promise((r) => setTimeout(r, 0));
  }
  throw new Error("Timed out waiting for condition");
}

describe("SessionSupervisor", () => {
  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it("refreshes Messages when backfill observes done", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
    const trackId = "track-1";

    (getSession as any).mockResolvedValue(mkSession(sessionId, trackId));
    (listSessionEventsTail as any).mockResolvedValue([]);
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

    (listSessionEventsPage as any).mockResolvedValue([
      {
        seq: 1,
        id: { 0: "e1" },
        session_id: { 0: sessionId },
        event_type: "done",
        payload_json: {},
        created_at: new Date().toISOString(),
      } satisfies SessionEvent,
    ]);

    const sup = new SessionSupervisor();
    await (sup as any).ensureLoaded(sessionId);
    await (sup as any).backfillSession(sessionId);

    const snap = sup.getSnapshot();
    expect((listMessages as any).mock.calls.length).toBe(2);
    expect(snap.sessions[sessionId]?.messages.length).toBe(2);
  });

  it("ingests assistant_complete without forcing a Messages refresh", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
    const trackId = "track-1";

    (getSession as any).mockResolvedValue(mkSession(sessionId, trackId));
    (listSessionEventsTail as any).mockResolvedValue([]);
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

    (listSessionEventsPage as any).mockResolvedValue([
      {
        seq: 2,
        id: { 0: "e1" },
        session_id: { 0: sessionId },
        event_type: "assistant_complete",
        payload_json: {},
        created_at: new Date().toISOString(),
      } satisfies SessionEvent,
    ]);

    const sup = new SessionSupervisor();
    await (sup as any).ensureLoaded(sessionId);
    await (sup as any).backfillSession(sessionId);

    const snap = sup.getSnapshot();
    expect(snap.sessions[sessionId]?.events.some((e) => e.event_type === "assistant_complete")).toBe(true);
    expect((listMessages as any).mock.calls.length).toBe(1);
    expect(snap.sessions[sessionId]?.messages.length).toBe(1);
  });

  it("parses Blob-like WS frames and ingests assistant_complete without forcing a Messages refresh", async () => {
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
    const trackId = "track-1";

    (getSession as any).mockResolvedValue(mkSession(sessionId, trackId));
    (listSessionEventsTail as any).mockResolvedValue([]);
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
      await (sup as any).ensureLoaded(sessionId);

      await (sup as any).openWebSocket("ws://example.test");
      expect(sockets.length).toBe(1);

      const frame = {
        text: async () =>
          JSON.stringify({
            seq: 3,
            id: { 0: "e1" },
            session_id: { 0: sessionId },
            event_type: "assistant_complete",
            payload_json: {},
            created_at: new Date().toISOString(),
          }),
      };

      sockets[0].emitMessage(frame);
      await waitForCondition(() => (sup.getSnapshot().sessions[sessionId]?.events.length ?? 0) === 1);

      const snap = sup.getSnapshot();
      expect(snap.sessions[sessionId]?.events.length).toBe(1);
      expect(snap.sessions[sessionId]?.events[0]?.event_type).toBe("assistant_complete");
      expect((listMessages as any).mock.calls.length).toBe(1);
      expect(snap.sessions[sessionId]?.messages.length).toBe(1);
    } finally {
      (globalThis as any).WebSocket = originalWebSocket;
    }
  });

  it("polls visible sessions even when connected + idle (recovers from missed WS events)", async () => {
    vi.useFakeTimers();
    const { SessionSupervisor } = await import("./sessionSupervisor");

    const sessionId = "session-1";
    const trackId = "track-1";

    (getSession as any).mockResolvedValue(mkSession(sessionId, trackId));
    (listSessionEventsTail as any).mockResolvedValue([
      {
        seq: 1,
        id: { 0: "e0" },
        session_id: { 0: sessionId },
        event_type: "done",
        payload_json: {},
        created_at: new Date().toISOString(),
      } satisfies SessionEvent,
    ]);
    (listMessages as any).mockResolvedValue([]);
    (listQueue as any).mockResolvedValue([]);
    (listSessionEventsPage as any).mockResolvedValue([]);

    const sup = new SessionSupervisor();
    sup.openSession(sessionId);
    await (sup as any).ensureLoaded(sessionId);
    // Simulate an apparently healthy WS connection; polling should still run for visible sessions.
    (sup as any).snapshot = { ...(sup as any).snapshot, connection: "connected" };

    await vi.advanceTimersByTimeAsync(1600);
    await waitForCondition(() => (listSessionEventsPage as any).mock.calls.length > 0);
  });
});
