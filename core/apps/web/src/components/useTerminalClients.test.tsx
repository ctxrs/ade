import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import type { TerminalSession } from "@ctx/types";

vi.mock("@xterm/xterm", () => {
  class MockTerminal {
    cols = 80;
    rows = 24;
    element: HTMLElement | null = null;
    options = { disableStdin: false };

    loadAddon() {}
    open(el: HTMLElement) {
      this.element = el;
    }
    write() {}
    dispose() {}
    focus() {}
    onData() {
      return { dispose() {} };
    }
  }

  return { Terminal: MockTerminal };
});

vi.mock("@xterm/addon-fit", () => {
  class MockFitAddon {
    fit() {}
  }

  return { FitAddon: MockFitAddon };
});

import { useTerminalClients } from "./useTerminalClients";

class MockWebSocket {
  static CONNECTING = 0;
  static OPEN = 1;
  static CLOSING = 2;
  static CLOSED = 3;
  static instances: MockWebSocket[] = [];

  readyState = MockWebSocket.CONNECTING;
  binaryType = "arraybuffer";
  url: string;
  sent: unknown[] = [];
  private listeners: Record<string, Array<(event: any) => void>> = {};

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
  }

  addEventListener(type: string, cb: (event: any) => void) {
    if (!this.listeners[type]) {
      this.listeners[type] = [];
    }
    this.listeners[type].push(cb);
  }

  send(data: unknown) {
    this.sent.push(data);
  }

  close() {
    this.readyState = MockWebSocket.CLOSED;
    this.emit("close", {});
  }

  open() {
    this.readyState = MockWebSocket.OPEN;
    this.emit("open", {});
  }

  private emit(type: string, event: any) {
    const handlers = this.listeners[type] ?? [];
    for (const handler of handlers) {
      handler(event);
    }
  }
}

let originalWebSocket: any;

const baseTerminal = (): TerminalSession => ({
  id: "terminal-1",
  workspace_id: "workspace-1",
  task_id: null,
  session_id: null,
  worktree_id: null,
  cwd: "/",
  shell: "/bin/bash",
  title: "bash",
  status: "running",
  exit_code: null,
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
});

const Harness = () => {
  const [terminals, setTerminals] = useState<TerminalSession[]>([baseTerminal()]);
  const clientsRef = useTerminalClients(terminals, setTerminals, "workspace-1");
  const client = clientsRef.current.get("terminal-1");
  return <div data-testid="status">{client?.connectionStatus ?? "missing"}</div>;
};

beforeEach(() => {
  MockWebSocket.instances = [];
  vi.useFakeTimers();
  vi.spyOn(Math, "random").mockReturnValue(0);
  originalWebSocket = (window as any).WebSocket;
  (window as any).WebSocket = MockWebSocket as any;
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  (window as any).WebSocket = originalWebSocket;
});

describe("useTerminalClients", () => {
  it("reconnects after close and updates connection state", async () => {
    render(<Harness />);
    expect(MockWebSocket.instances).toHaveLength(1);

    const first = MockWebSocket.instances[0];
    await act(async () => {
      first.open();
    });
    expect(screen.getByTestId("status")).toHaveTextContent("connected");

    await act(async () => {
      first.close();
    });
    expect(screen.getByTestId("status")).toHaveTextContent("reconnecting");

    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    expect(MockWebSocket.instances).toHaveLength(2);
    const second = MockWebSocket.instances[1];
    await act(async () => {
      second.open();
    });
    expect(screen.getByTestId("status")).toHaveTextContent("connected");
  });

  it("surfaces disconnected after repeated failures", async () => {
    render(<Harness />);
    expect(MockWebSocket.instances).toHaveLength(1);

    for (let i = 0; i < 4; i += 1) {
      const socket = MockWebSocket.instances[i];
      await act(async () => {
        socket.close();
      });
      await act(async () => {
        vi.advanceTimersByTime(5000);
      });
    }

    expect(screen.getByTestId("status")).toHaveTextContent("disconnected");
  });

  it("reconnects when keepalive stalls", async () => {
    render(<Harness />);
    expect(MockWebSocket.instances).toHaveLength(1);

    const first = MockWebSocket.instances[0];
    await act(async () => {
      first.open();
    });
    expect(screen.getByTestId("status")).toHaveTextContent("connected");

    await act(async () => {
      vi.advanceTimersByTime(110_000);
    });

    expect(MockWebSocket.instances.length).toBeGreaterThan(1);
    expect(screen.getByTestId("status")).toHaveTextContent(/reconnecting|disconnected/);
  });
});
