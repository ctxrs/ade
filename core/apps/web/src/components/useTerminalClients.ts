import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import type { TerminalSession } from "@ctx/types";
import { useEffect, useRef, type Dispatch, type SetStateAction } from "react";
import { authToken, idToString, resolveDaemonWsBaseUrl } from "../api/client";

export type TerminalClient = {
  id: string;
  terminal: Terminal;
  fitAddon: FitAddon;
  element: HTMLElement | null;
  status: TerminalSession["status"];
  exitCode: number | null;
  attach: (el: HTMLElement) => void;
  fit: () => void;
  focus: () => void;
  dispose: () => void;
  reconnect: () => void;
};

type TerminalStatusMessage = {
  type: "status";
  status: TerminalSession["status"];
  exit_code?: number | null;
};

export function useTerminalClients(
  terminals: TerminalSession[],
  setTerminals: Dispatch<SetStateAction<TerminalSession[]>>,
  workspaceId: string,
) {
  const clientsRef = useRef<Map<string, TerminalClient>>(new Map());

  useEffect(() => {
    const map = clientsRef.current;
    const seen = new Set<string>();
    for (const terminal of terminals) {
      const id = idToString(terminal.id);
      if (!id) continue;
      seen.add(id);
      if (map.has(id)) {
        const client = map.get(id)!;
        client.status = terminal.status;
        client.exitCode = terminal.exit_code ?? null;
        continue;
      }
      const client = createClient(terminal, (status, exitCode) => {
        setTerminals((prev) =>
          prev.map((t) =>
            idToString(t.id) === id
              ? {
                  ...t,
                  status,
                  exit_code: exitCode ?? null,
                }
              : t,
          ),
        );
      });
      map.set(id, client);
    }
    for (const id of Array.from(map.keys())) {
      if (!seen.has(id)) {
        const client = map.get(id);
        client?.dispose();
        map.delete(id);
      }
    }
  }, [setTerminals, terminals]);

  useEffect(() => {
    return () => {
      for (const client of clientsRef.current.values()) {
        client.dispose();
      }
      clientsRef.current.clear();
    };
  }, [workspaceId]);

  return clientsRef;
}

function buildTerminalWsUrl(terminalId: string): string {
  const wsBase = resolveDaemonWsBaseUrl();
  const baseUrl = `${wsBase}/api/terminals/${terminalId}/stream`;
  const token = authToken();
  if (!token) return baseUrl;
  return `${baseUrl}?token=${encodeURIComponent(token)}`;
}

function terminalTheme() {
  const styles = getComputedStyle(document.documentElement);
  const read = (name: string, fallback: string) => styles.getPropertyValue(name).trim() || fallback;
  return {
    background: read("--panel", "#252526"),
    foreground: read("--text", "#d4d4d4"),
    cursor: read("--text", "#d4d4d4"),
    selectionBackground: "rgba(255, 255, 255, 0.2)",
  };
}

function terminalFontFamily() {
  const styles = getComputedStyle(document.documentElement);
  return styles.getPropertyValue("--mono").trim() || "monospace";
}

function createClient(
  terminal: TerminalSession,
  onStatus: (status: TerminalSession["status"], exitCode: number | null) => void,
): TerminalClient {
  const id = idToString(terminal.id);
  const term = new Terminal({
    fontFamily: terminalFontFamily(),
    fontSize: 12,
    lineHeight: 1.15,
    scrollback: 2000,
    cursorBlink: true,
    theme: terminalTheme(),
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);

  let socket: WebSocket | null = null;
  let element: HTMLElement | null = null;
  const canFit = () => {
    if (!element || !element.isConnected) return false;
    if (element.closest(".wb-terminal-group-hidden")) return false;
    return element.clientWidth > 0 && element.clientHeight > 0;
  };

  const sendResize = () => {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    const cols = term.cols;
    const rows = term.rows;
    socket.send(JSON.stringify({ type: "resize", cols, rows }));
  };

  const fitNow = () => {
    if (!canFit()) return;
    fitAddon.fit();
    sendResize();
  };

  const connect = () => {
    if (socket && socket.readyState === WebSocket.OPEN) return;
    socket = new WebSocket(buildTerminalWsUrl(id));
    socket.binaryType = "arraybuffer";
    socket.addEventListener("open", () => {
      sendResize();
    });
    socket.addEventListener("close", () => {
    });
    socket.addEventListener("message", (ev) => {
      if (typeof ev.data === "string") {
        try {
          const msg = JSON.parse(ev.data) as TerminalStatusMessage;
          if (msg.type === "status") {
            onStatus(msg.status, msg.exit_code ?? null);
            return;
          }
        } catch {
          // ignore
        }
        term.write(ev.data);
        return;
      }
      if (ev.data instanceof ArrayBuffer) {
        term.write(new Uint8Array(ev.data));
        return;
      }
      if (ev.data instanceof Blob) {
        void ev.data.arrayBuffer().then((buf) => term.write(new Uint8Array(buf)));
      }
    });
  };

  term.onData((data) => {
    if (!socket || socket.readyState !== WebSocket.OPEN) return;
    socket.send(data);
  });

  connect();

  return {
    id,
    terminal: term,
    fitAddon,
    element,
    status: terminal.status,
    exitCode: terminal.exit_code ?? null,
    attach: (el) => {
      if (element === el) return;
      element = el;
      if (term.element) {
        el.replaceChildren(term.element);
      } else {
        term.open(el);
      }
      requestAnimationFrame(fitNow);
    },
    fit: () => {
      fitNow();
    },
    focus: () => {
      term.focus();
    },
    dispose: () => {
      socket?.close();
      term.dispose();
    },
    reconnect: () => {
      socket?.close();
      connect();
    },
  };
}
