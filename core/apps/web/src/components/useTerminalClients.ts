import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal, type ILink, type ILinkProvider } from "@xterm/xterm";
import type { TerminalSession } from "@ctx/types";
import { useEffect, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { idToString } from "../api/client";
import { getDaemonWsUrl } from "../api/daemonConnection";
import { openExternalLink } from "../utils/desktop";
import { readCssVar, useThemeVariant, withAlpha, type ThemeVariant } from "../utils/theme";

export type TerminalConnectionStatus = "connected" | "reconnecting" | "disconnected";

type E2ETerminalClientHandle = {
  close: () => void;
  getConnectionStatus: () => TerminalConnectionStatus;
};

type WindowWithE2ETerminalHooks = Window & {
  __ctxE2ETerminalClients?: Map<string, E2ETerminalClientHandle>;
  __ctxE2ETerminals?: Map<string, Terminal>;
};

function getE2ETerminalClientRegistry(): Map<string, E2ETerminalClientHandle> | null {
  if (typeof window === "undefined") return null;
  try {
    if (window.sessionStorage.getItem("ctxE2E") !== "1") return null;
  } catch {
    return null;
  }
  const w = window as WindowWithE2ETerminalHooks;
  if (!w.__ctxE2ETerminalClients) {
    w.__ctxE2ETerminalClients = new Map<string, E2ETerminalClientHandle>();
  }
  return w.__ctxE2ETerminalClients;
}

function getE2ETerminalRegistry(): Map<string, Terminal> | null {
  // E2E-only hook: allow Playwright tests to introspect xterm state (buffer ydisp/baseY)
  // without relying on renderer-specific DOM text.
  if (typeof window === "undefined") return null;
  try {
    if (window.sessionStorage.getItem("ctxE2E") !== "1") return null;
  } catch {
    return null;
  }
  const w = window as WindowWithE2ETerminalHooks;
  if (!w.__ctxE2ETerminals) {
    w.__ctxE2ETerminals = new Map<string, Terminal>();
  }
  return w.__ctxE2ETerminals;
}

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
      client.terminal.options.theme = theme;
    }
  }, [themeVariant]);

  return clientsRef;
}

function buildTerminalWsUrl(terminal: TerminalSession): string {
  return getDaemonWsUrl(terminal.stream_path);
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
  const ansiFallbacks =
    themeVariant === "dark"
      ? {
          black: "#000000",
          red: "#cd3131",
          green: "#0dbc79",
          yellow: "#e5e510",
          blue: "#2472c8",
          magenta: "#bc3fbc",
          cyan: "#11a8cd",
          white: "#e5e5e5",
          brightBlack: "#666666",
          brightRed: "#f14c4c",
          brightGreen: "#23d18b",
          brightYellow: "#f5f543",
          brightBlue: "#3b8eea",
          brightMagenta: "#d670d6",
          brightCyan: "#29b8db",
          brightWhite: "#e5e5e5",
        }
      : {
          black: "#000000",
          red: "#cd3131",
          green: "#107c10",
          yellow: "#949800",
          blue: "#0451a5",
          magenta: "#bc05bc",
          cyan: "#0598bc",
          white: "#555555",
          brightBlack: "#666666",
          brightRed: "#cd3131",
          brightGreen: "#14ce14",
          brightYellow: "#b5ba00",
          brightBlue: "#0451a5",
          brightMagenta: "#bc05bc",
          brightCyan: "#0598bc",
          brightWhite: "#a5a5a5",
        };
  const background = read("--terminal-bg", read("--panel", panelFallback));
  const foreground = read("--terminal-fg", read("--text", textFallback));
  const cursor = read("--terminal-cursor", foreground);
  const cursorAccent = read("--terminal-cursor-accent", background);
  const selectionBackground = read(
    "--terminal-selection-bg",
    withAlpha(accent, selectionAlpha, selectionFallback),
  );
  const selectionInactiveBackground = read(
    "--terminal-selection-inactive-bg",
    withAlpha(selectionBackground, 0.5, selectionBackground),
  );
  return {
    background,
    foreground,
    cursor,
    cursorAccent,
    selectionBackground,
    selectionInactiveBackground,
    black: read("--terminal-ansi-black", ansiFallbacks.black),
    red: read("--terminal-ansi-red", ansiFallbacks.red),
    green: read("--terminal-ansi-green", ansiFallbacks.green),
    yellow: read("--terminal-ansi-yellow", ansiFallbacks.yellow),
    blue: read("--terminal-ansi-blue", ansiFallbacks.blue),
    magenta: read("--terminal-ansi-magenta", ansiFallbacks.magenta),
    cyan: read("--terminal-ansi-cyan", ansiFallbacks.cyan),
    white: read("--terminal-ansi-white", ansiFallbacks.white),
    brightBlack: read("--terminal-ansi-bright-black", ansiFallbacks.brightBlack),
    brightRed: read("--terminal-ansi-bright-red", ansiFallbacks.brightRed),
    brightGreen: read("--terminal-ansi-bright-green", ansiFallbacks.brightGreen),
    brightYellow: read("--terminal-ansi-bright-yellow", ansiFallbacks.brightYellow),
    brightBlue: read("--terminal-ansi-bright-blue", ansiFallbacks.brightBlue),
    brightMagenta: read("--terminal-ansi-bright-magenta", ansiFallbacks.brightMagenta),
    brightCyan: read("--terminal-ansi-bright-cyan", ansiFallbacks.brightCyan),
    brightWhite: read("--terminal-ansi-bright-white", ansiFallbacks.brightWhite),
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

// When the terminal is hidden/collapsed (height 0), xterm can get into a bad scroll state
// if we keep feeding it output. Buffer a bounded amount while unfittable and flush on the
// next successful fit.
const PENDING_OUTPUT_MAX_BYTES = 512 * 1024;

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
  getE2ETerminalRegistry()?.set(id, term);

  let client: TerminalClient;
  let socket: WebSocket | null = null;
  let element: HTMLElement | null = null;
  let reconnectTimer: number | null = null;
  let keepaliveTimer: number | null = null;
  let reconnectAttempts = 0;
  let disposed = false;
  let connectionStatus: TerminalConnectionStatus = "disconnected";
  const e2eClientRegistry = getE2ETerminalClientRegistry();
  e2eClientRegistry?.set(id, {
    close: () => {
      try {
        socket?.close();
      } catch {
        // ignore
      }
    },
    getConnectionStatus: () => connectionStatus,
  });
  let status: TerminalSession["status"] = terminal.status;
  let exitCode: number | null = terminal.exit_code ?? null;
  let lastServerMessageAt = Date.now();
  let scrollToBottomOnNextFit = true;
  let pendingOutput: Array<string | Uint8Array | null> = [];
  let pendingOutputHead = 0;
  let pendingOutputBytes = 0;

  const pendingSizeOf = (chunk: string | Uint8Array) =>
    typeof chunk === "string" ? chunk.length : chunk.byteLength;
  const hasPendingOutput = () => pendingOutputHead < pendingOutput.length;

  const maybeCompactPendingOutput = () => {
    // Avoid O(n) shifting on drops while still keeping memory bounded.
    if (pendingOutputHead < 128) return;
    if (pendingOutputHead * 2 < pendingOutput.length) return;
    pendingOutput = pendingOutput.slice(pendingOutputHead);
    pendingOutputHead = 0;
  };

  const enqueuePendingOutput = (chunk: string | Uint8Array) => {
    pendingOutput.push(chunk);
    pendingOutputBytes += pendingSizeOf(chunk);
    while (pendingOutputBytes > PENDING_OUTPUT_MAX_BYTES && hasPendingOutput()) {
      const dropped = pendingOutput[pendingOutputHead];
      // Release dropped chunks immediately to keep actual memory usage bounded even if
      // compaction hasn't run yet (e.g. a few large WS messages while the panel is hidden).
      pendingOutput[pendingOutputHead] = null;
      pendingOutputHead += 1;
      if (dropped !== null) {
        pendingOutputBytes -= pendingSizeOf(dropped);
      }
    }
    maybeCompactPendingOutput();
  };

  const flushPendingOutput = (): boolean => {
    if (!hasPendingOutput()) return false;
    const chunks = pendingOutput;
    const start = pendingOutputHead;
    const end = chunks.length;
    pendingOutputHead = 0;
    pendingOutput = [];
    pendingOutputBytes = 0;
    for (let i = start; i < end; i += 1) {
      const chunk = chunks[i];
      if (chunk === null) continue;
      term.write(chunk);
    }
    return true;
  };

  const scrollToBottomIfRequested = () => {
    if (!scrollToBottomOnNextFit) return;
    scrollToBottomOnNextFit = false;
    term.scrollToBottom();
  };

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
    // xterm can get a stale viewport after being hidden/collapsed; force a repaint.
    term.refresh(0, Math.max(0, term.rows - 1));
    // If we buffered output while hidden, flush after we've established correct cols/rows.
    if (flushPendingOutput()) {
      term.refresh(0, Math.max(0, term.rows - 1));
    }
    scrollToBottomIfRequested();
  };

  const writeOrBufferOutput = (chunk: string | Uint8Array) => {
    if (!canFit()) {
      enqueuePendingOutput(chunk);
      return;
    }

    // Preserve stream ordering across hidden->visible transitions. Otherwise, new
    // chunks can be written while older buffered chunks flush later in `fitNow()`.
    if (hasPendingOutput()) {
      fitNow();
      // If we couldn't flush (visibility flipped again), keep buffering to avoid reordering.
      if (hasPendingOutput()) {
        enqueuePendingOutput(chunk);
        return;
      }
    }

    term.write(chunk);
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
    let wsUrl = "";
    try {
      wsUrl = buildTerminalWsUrl(terminal);
    } catch {
      scheduleReconnect();
      return;
    }
    socket = new WebSocket(wsUrl);
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
        writeOrBufferOutput(ev.data);
        return;
      }
      if (ev.data instanceof ArrayBuffer) {
        writeOrBufferOutput(new Uint8Array(ev.data));
        return;
      }
      if (ev.data instanceof Blob) {
        void ev.data.arrayBuffer().then((buf) => {
          writeOrBufferOutput(new Uint8Array(buf));
        });
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
      e2eClientRegistry?.delete(id);
      getE2ETerminalRegistry()?.delete(id);
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
