import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal, type ILink, type ILinkProvider } from "@xterm/xterm";
import type { TerminalSession } from "@ctx/types";
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { authToken, idToString, resolveDaemonWsBaseUrl } from "../api/client";
import { openExternalLink } from "../utils/desktop";
import { readCssVar, useThemeVariant, withAlpha, type ThemeVariant } from "../utils/theme";

export type TerminalConnectionStatus = "connected" | "reconnecting" | "disconnected";

export type TerminalClient = {
  id: string;
  terminal: Terminal;
  fitAddon: FitAddon;
  element: HTMLElement | null;
  status: TerminalSession["status"];
  exitCode: number | null;
  connectionStatus: TerminalConnectionStatus;
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

type TerminalPongMessage = {
  type: "pong";
};

type TerminalControlMessage = TerminalStatusMessage | TerminalPongMessage;

export function useTerminalClients(
  terminals: TerminalSession[],
  setTerminals: Dispatch<SetStateAction<TerminalSession[]>>,
  workspaceId: string,
) {
  const clientsRef = useRef<Map<string, TerminalClient>>(new Map());
  const [, setConnectionVersion] = useState(0);
  const themeVariant = useThemeVariant();

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
      const client = createClient(terminal, themeVariant, (status, exitCode) => {
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
      }, () => {
        setConnectionVersion((prev) => prev + 1);
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
  }, [setTerminals, terminals, themeVariant]);

  useEffect(() => {
    return () => {
      for (const client of clientsRef.current.values()) {
        client.dispose();
      }
      clientsRef.current.clear();
    };
  }, [workspaceId]);

  useEffect(() => {
    const theme = terminalTheme(themeVariant);
    for (const client of clientsRef.current.values()) {
      client.terminal.setOption("theme", theme);
    }
  }, [themeVariant]);

  return clientsRef;
}

function buildTerminalWsUrl(terminalId: string): string {
  const wsBase = resolveDaemonWsBaseUrl();
  const baseUrl = `${wsBase}/api/terminals/${terminalId}/stream`;
  const token = authToken();
  if (!token) return baseUrl;
  return `${baseUrl}?token=${encodeURIComponent(token)}`;
}

function terminalTheme(themeVariant: ThemeVariant) {
  const read = (name: string, fallback: string) => readCssVar(name, fallback);
  const accentFallback = themeVariant === "dark" ? "#3794ff" : "#005fb8";
  const panelFallback = themeVariant === "dark" ? "#252526" : "#f8f8f8";
  const textFallback = themeVariant === "dark" ? "#d4d4d4" : "#3b3b3b";
  const accent = read("--accent", accentFallback);
  const selectionFallback =
    themeVariant === "dark" ? "rgba(255, 255, 255, 0.2)" : "rgba(0, 0, 0, 0.12)";
  const selectionAlpha = themeVariant === "dark" ? 0.2 : 0.15;
  return {
    background: read("--panel", panelFallback),
    foreground: read("--text", textFallback),
    cursor: read("--text", textFallback),
    selectionBackground: withAlpha(accent, selectionAlpha, selectionFallback),
  };
}

function terminalFontFamily() {
  return readCssVar("--mono", "monospace");
}

const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 10_000;
const DISCONNECTED_AFTER_ATTEMPTS = 3;
const KEEPALIVE_INTERVAL_MS = 25_000;
const KEEPALIVE_TIMEOUT_MS = 75_000;

function createClient(
  terminal: TerminalSession,
  themeVariant: ThemeVariant,
  onStatus: (status: TerminalSession["status"], exitCode: number | null) => void,
  onConnectionChange: () => void,
): TerminalClient {
  const id = idToString(terminal.id);
  const term = new Terminal({
    fontFamily: terminalFontFamily(),
    fontSize: 12,
    lineHeight: 1.15,
    scrollback: 2000,
    cursorBlink: true,
    theme: terminalTheme(themeVariant),
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  const cleanupLinks = installWebLinksAddon(term);

  let client: TerminalClient;
  let socket: WebSocket | null = null;
  let element: HTMLElement | null = null;
  let reconnectTimer: number | null = null;
  let keepaliveTimer: number | null = null;
  let reconnectAttempts = 0;
  let disposed = false;
  let connectionStatus: TerminalConnectionStatus = "disconnected";
  let status: TerminalSession["status"] = terminal.status;
  let exitCode: number | null = terminal.exit_code ?? null;
  let lastServerMessageAt = Date.now();
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

  const updateInputState = () => {
    term.options.disableStdin = connectionStatus !== "connected" || status === "exited";
  };

  const clearReconnectTimer = () => {
    if (reconnectTimer === null) return;
    window.clearTimeout(reconnectTimer);
    reconnectTimer = null;
  };

  const clearKeepaliveTimer = () => {
    if (keepaliveTimer === null) return;
    window.clearInterval(keepaliveTimer);
    keepaliveTimer = null;
  };

  const startKeepalive = () => {
    if (keepaliveTimer !== null) return;
    keepaliveTimer = window.setInterval(() => {
      if (disposed) return;
      if (!socket || socket.readyState !== WebSocket.OPEN) return;
      const now = Date.now();
      if (now - lastServerMessageAt > KEEPALIVE_TIMEOUT_MS) {
        socket.close();
        return;
      }
      socket.send(JSON.stringify({ type: "ping" }));
    }, KEEPALIVE_INTERVAL_MS);
  };

  const setConnectionStatus = (next: TerminalConnectionStatus) => {
    if (connectionStatus === next) return;
    connectionStatus = next;
    client.connectionStatus = next;
    updateInputState();
    onConnectionChange();
  };

  const scheduleReconnect = () => {
    if (disposed) return;
    if (status === "exited") {
      setConnectionStatus("disconnected");
      return;
    }
    clearReconnectTimer();
    reconnectAttempts += 1;
    const backoff = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** (reconnectAttempts - 1));
    const jitter = 0.7 + Math.random() * 0.6;
    const delay = Math.round(backoff * jitter);
    const nextState =
      reconnectAttempts > DISCONNECTED_AFTER_ATTEMPTS ? "disconnected" : "reconnecting";
    setConnectionStatus(nextState);
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null;
      connect();
    }, delay);
  };

  const connect = () => {
    if (disposed) return;
    if (socket && (socket.readyState === WebSocket.OPEN || socket.readyState === WebSocket.CONNECTING)) {
      return;
    }
    const nextState =
      reconnectAttempts > DISCONNECTED_AFTER_ATTEMPTS ? "disconnected" : "reconnecting";
    setConnectionStatus(nextState);
    socket = new WebSocket(buildTerminalWsUrl(id));
    socket.binaryType = "arraybuffer";
    socket.addEventListener("open", () => {
      reconnectAttempts = 0;
      clearReconnectTimer();
      lastServerMessageAt = Date.now();
      setConnectionStatus("connected");
      sendResize();
      startKeepalive();
    });
    socket.addEventListener("close", () => {
      socket = null;
      clearKeepaliveTimer();
      scheduleReconnect();
    });
    socket.addEventListener("message", (ev) => {
      lastServerMessageAt = Date.now();
      if (typeof ev.data === "string") {
        try {
          const msg = JSON.parse(ev.data) as TerminalControlMessage;
          if (msg.type === "status") {
            status = msg.status;
            exitCode = msg.exit_code ?? null;
            client.status = status;
            client.exitCode = exitCode;
            updateInputState();
            onStatus(status, exitCode);
            return;
          }
          if (msg.type === "pong") {
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
    if (connectionStatus !== "connected" || status === "exited") return;
    socket.send(data);
  });

  client = {
    id,
    terminal: term,
    fitAddon,
    element,
    status: terminal.status,
    exitCode: terminal.exit_code ?? null,
    connectionStatus,
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
      disposed = true;
      clearReconnectTimer();
      clearKeepaliveTimer();
      cleanupLinks();
      socket?.close();
      socket = null;
      term.dispose();
    },
    reconnect: () => {
      reconnectAttempts = 0;
      socket?.close();
      if (!socket) {
        scheduleReconnect();
      }
    },
  };

  updateInputState();
  connect();

  return client;
}

export function installWebLinksAddon(term: Terminal) {
  const isModifierPressed = (event: { metaKey?: boolean; ctrlKey?: boolean }) =>
    !!event.metaKey || !!event.ctrlKey;
  let modifierPressed = false;
  let hoveredLink: ILink | null = null;

  const setDecorations = (link: ILink, active: boolean) => {
    if (!link.decorations) {
      link.decorations = { underline: active, pointerCursor: active };
      return;
    }
    link.decorations.underline = active;
    link.decorations.pointerCursor = active;
  };

  const updateHoveredLink = () => {
    if (!hoveredLink) return;
    setDecorations(hoveredLink, modifierPressed);
  };

  const setModifierPressed = (next: boolean) => {
    if (modifierPressed === next) return;
    modifierPressed = next;
    updateHoveredLink();
  };

  const handleKeyEvent = (event: KeyboardEvent) => {
    if (event.key !== "Meta" && event.key !== "Control") return;
    setModifierPressed(event.type === "keydown");
  };

  const handleWindowBlur = () => {
    setModifierPressed(false);
  };

  window.addEventListener("keydown", handleKeyEvent);
  window.addEventListener("keyup", handleKeyEvent);
  window.addEventListener("blur", handleWindowBlur);

  const openLink = (event: MouseEvent, uri: string) => {
    if (!isModifierPressed(event) && !modifierPressed) return;
    void openExternalLink(uri);
  };

  const wrapLink = (link: ILink) => {
    setDecorations(link, false);
    const originalHover = link.hover;
    const originalLeave = link.leave;
    link.hover = (event, text) => {
      hoveredLink = link;
      queueMicrotask(() => setDecorations(link, isModifierPressed(event) || modifierPressed));
      originalHover?.(event, text);
    };
    link.leave = (event, text) => {
      if (hoveredLink === link) {
        hoveredLink = null;
      }
      setDecorations(link, false);
      originalLeave?.(event, text);
    };
  };

  const originalRegisterLinkProvider = term.registerLinkProvider;
  term.registerLinkProvider = (provider: ILinkProvider) =>
    originalRegisterLinkProvider.call(term, {
      provideLinks: (bufferLineNumber, callback) => {
        provider.provideLinks(bufferLineNumber, (links) => {
          if (links) {
            for (const link of links) {
              wrapLink(link);
            }
          }
          callback(links);
        });
      },
    });

  try {
    term.loadAddon(new WebLinksAddon(openLink));
  } finally {
    term.registerLinkProvider = originalRegisterLinkProvider;
  }

  return () => {
    window.removeEventListener("keydown", handleKeyEvent);
    window.removeEventListener("keyup", handleKeyEvent);
    window.removeEventListener("blur", handleWindowBlur);
  };
}
