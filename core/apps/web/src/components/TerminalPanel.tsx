import React, {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { ChevronDown, Plus, SplitSquareVertical, Terminal as TerminalIcon, X } from "lucide-react";
import type { TerminalSession } from "@ctx/types";
import {
  authToken,
  createWorkspaceTerminal,
  deleteTerminal,
  resolveDaemonWsBaseUrl,
  idToString,
  listWorkspaceTerminals,
  type CreateTerminalRequest,
} from "../api/client";
import type {
  PersistedWorkbenchTerminalLayoutV1,
  SplitDirection,
  TerminalGroupState,
  TerminalLayoutNode,
  TerminalPanelScopeState,
  TerminalScope,
} from "../workbench/types";
import {
  loadWorkbenchTerminalLayoutV1,
  loadWorkbenchTerminalTitlesV1,
  saveWorkbenchTerminalLayoutV1,
  saveWorkbenchTerminalTitlesV1,
} from "../workbench/persistence";
import { randomUuid } from "../utils/randomUuid";

export type TerminalPanelHandle = {
  createTerminal: (opts: CreateTerminalOptions) => Promise<string | null>;
  focusTerminal: (terminalId: string) => void;
  focusActive: () => void;
  setScope: (scope: TerminalScope) => void;
};

export type CreateTerminalOptions = {
  cwd?: string | null;
  taskId?: string | null;
  sessionId?: string | null;
  worktreeId?: string | null;
  scope?: TerminalScope;
};

type TerminalPanelProps = {
  workspaceId: string;
  activeTaskId: string | null;
  activeSessionId: string | null;
  open: boolean;
  height: number;
  onRequestClose: () => void;
};

type TerminalClient = {
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

const emptyScopeState = (): TerminalPanelScopeState => ({
  groups: [],
  activeGroupId: null,
  tabOrder: [],
});

const defaultPanelState = (): PersistedWorkbenchTerminalLayoutV1 => ({
  v: 1,
  scope: "workspace",
  scopes: {
    task: emptyScopeState(),
    workspace: emptyScopeState(),
  },
});

function leafIds(node: TerminalLayoutNode | null, out: string[] = []): string[] {
  if (!node) return out;
  if (node.kind === "leaf") {
    out.push(node.id);
    return out;
  }
  leafIds(node.first, out);
  leafIds(node.second, out);
  return out;
}

function firstLeafId(node: TerminalLayoutNode | null): string | null {
  if (!node) return null;
  if (node.kind === "leaf") return node.id;
  return firstLeafId(node.first) ?? firstLeafId(node.second);
}

function findLeafIdForTerminal(node: TerminalLayoutNode | null, terminalId: string): string | null {
  if (!node) return null;
  if (node.kind === "leaf") return node.terminalId === terminalId ? node.id : null;
  return findLeafIdForTerminal(node.first, terminalId) ?? findLeafIdForTerminal(node.second, terminalId);
}

function findTerminalIdForLeaf(node: TerminalLayoutNode | null, leafId: string | null): string | null {
  if (!node || !leafId) return null;
  if (node.kind === "leaf") return node.id === leafId ? node.terminalId : null;
  return findTerminalIdForLeaf(node.first, leafId) ?? findTerminalIdForLeaf(node.second, leafId);
}

function terminalIdsInLayout(node: TerminalLayoutNode | null, out: string[] = []): string[] {
  if (!node) return out;
  if (node.kind === "leaf") {
    out.push(node.terminalId);
    return out;
  }
  terminalIdsInLayout(node.first, out);
  terminalIdsInLayout(node.second, out);
  return out;
}

function resolveActiveLeafId(node: TerminalLayoutNode | null, activeLeafId: string | null): string | null {
  if (!node) return null;
  const list = leafIds(node);
  if (activeLeafId && list.includes(activeLeafId)) return activeLeafId;
  return list[0] ?? null;
}

function createSingleTerminalGroup(terminalId: string): TerminalGroupState {
  const leafId = randomUuid();
  return {
    id: randomUuid(),
    layout: { kind: "leaf", id: leafId, terminalId },
    activeLeafId: leafId,
  };
}

function findGroupForTerminal(
  groups: TerminalGroupState[],
  terminalId: string,
): { group: TerminalGroupState; leafId: string } | null {
  for (const group of groups) {
    const leafId = findLeafIdForTerminal(group.layout, terminalId);
    if (leafId) return { group, leafId };
  }
  return null;
}

function pruneLayout(node: TerminalLayoutNode | null, allowed: Set<string>): TerminalLayoutNode | null {
  if (!node) return null;
  if (node.kind === "leaf") {
    return allowed.has(node.terminalId) ? node : null;
  }
  const first = pruneLayout(node.first, allowed);
  const second = pruneLayout(node.second, allowed);
  if (!first && !second) return null;
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
}

function updateSplitRatio(node: TerminalLayoutNode, splitId: string, ratio: number): TerminalLayoutNode {
  if (node.kind === "leaf") return node;
  if (node.id === splitId) return { ...node, ratio };
  return {
    ...node,
    first: updateSplitRatio(node.first, splitId, ratio),
    second: updateSplitRatio(node.second, splitId, ratio),
  };
}

function splitLeaf(
  node: TerminalLayoutNode,
  leafId: string,
  newTerminalId: string,
  direction: SplitDirection,
): TerminalLayoutNode {
  if (node.kind === "leaf") {
    if (node.id !== leafId) return node;
    const existing = node.terminalId;
    return {
      kind: "split",
      id: randomUuid(),
      direction,
      ratio: 0.5,
      first: { kind: "leaf", id: randomUuid(), terminalId: existing },
      second: { kind: "leaf", id: randomUuid(), terminalId: newTerminalId },
    };
  }
  return {
    ...node,
    first: splitLeaf(node.first, leafId, newTerminalId, direction),
    second: splitLeaf(node.second, leafId, newTerminalId, direction),
  };
}

function removeTerminalFromLayout(node: TerminalLayoutNode | null, terminalId: string): TerminalLayoutNode | null {
  if (!node) return null;
  if (node.kind === "leaf") {
    return node.terminalId === terminalId ? null : node;
  }
  const first = removeTerminalFromLayout(node.first, terminalId);
  const second = removeTerminalFromLayout(node.second, terminalId);
  if (!first && !second) return null;
  if (!first) return second;
  if (!second) return first;
  return { ...node, first, second };
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

export const TerminalPanel = forwardRef<TerminalPanelHandle, TerminalPanelProps>(function TerminalPanel(
  { workspaceId, activeTaskId, activeSessionId, open, height, onRequestClose },
  ref,
) {
  const [terminals, setTerminals] = useState<TerminalSession[]>([]);
  const [panelState, setPanelState] = useState<PersistedWorkbenchTerminalLayoutV1>(defaultPanelState);
  const [layoutHydrated, setLayoutHydrated] = useState(false);
  const [titlesHydrated, setTitlesHydrated] = useState(false);
  const [titleOverrides, setTitleOverrides] = useState<Record<string, string>>({});
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const renameInputRef = useRef<HTMLInputElement | null>(null);
  const [contextMenu, setContextMenu] = useState<{ terminalId: string; x: number; y: number } | null>(null);
  const [contextMenuStyle, setContextMenuStyle] = useState<React.CSSProperties | null>(null);
  const contextMenuRef = useRef<HTMLDivElement | null>(null);
  const clientsRef = useRef<Map<string, TerminalClient>>(new Map());
  const resizeFrameRef = useRef<number | null>(null);
  const autoCreateWorkspaceRef = useRef(false);

  const refreshTerminals = useCallback(async () => {
    if (!workspaceId) return;
    const list = await listWorkspaceTerminals(workspaceId);
    setTerminals(list);
  }, [workspaceId]);

  useEffect(() => {
    let cancelled = false;
    if (!workspaceId) return;
    loadWorkbenchTerminalLayoutV1(workspaceId)
      .then((loaded) => {
        if (cancelled) return;
        if (loaded) setPanelState(loaded);
        else setPanelState(defaultPanelState());
        setLayoutHydrated(true);
      })
      .catch(() => {
        if (cancelled) return;
        setLayoutHydrated(true);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  useEffect(() => {
    let cancelled = false;
    setTitlesHydrated(false);
    if (!workspaceId) {
      setTitleOverrides({});
      setTitlesHydrated(true);
      return;
    }
    loadWorkbenchTerminalTitlesV1(workspaceId)
      .then((loaded) => {
        if (cancelled) return;
        setTitleOverrides(loaded?.titles ?? {});
        setTitlesHydrated(true);
      })
      .catch(() => {
        if (cancelled) return;
        setTitleOverrides({});
        setTitlesHydrated(true);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  useEffect(() => {
    if (!layoutHydrated) return;
    if (!workspaceId) return;
    saveWorkbenchTerminalLayoutV1(workspaceId, panelState).catch(() => {});
  }, [layoutHydrated, panelState, workspaceId]);

  useEffect(() => {
    if (!titlesHydrated) return;
    if (!workspaceId) return;
    saveWorkbenchTerminalTitlesV1(workspaceId, { v: 1, titles: titleOverrides }).catch(() => {});
  }, [titleOverrides, titlesHydrated, workspaceId]);

  useEffect(() => {
    if (!renamingId) return;
    const frame = window.requestAnimationFrame(() => {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    });
    return () => window.cancelAnimationFrame(frame);
  }, [renamingId]);

  useEffect(() => {
    refreshTerminals().catch(() => {});
  }, [refreshTerminals]);

  useEffect(() => {
    if (!contextMenu) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && contextMenuRef.current?.contains(target)) return;
      setContextMenu(null);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setContextMenu(null);
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [contextMenu]);

  useLayoutEffect(() => {
    if (!contextMenu) {
      setContextMenuStyle(null);
      return;
    }
    const menu = contextMenuRef.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    const margin = 8;
    let left = contextMenu.x;
    let top = contextMenu.y;
    if (left + rect.width > window.innerWidth - margin) {
      left = window.innerWidth - margin - rect.width;
    }
    if (top + rect.height > window.innerHeight - margin) {
      top = window.innerHeight - margin - rect.height;
    }
    left = Math.max(margin, left);
    top = Math.max(margin, top);
    setContextMenuStyle({ left, top, position: "fixed" });
  }, [contextMenu]);

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
  }, [terminals]);

  useEffect(() => {
    return () => {
      for (const client of clientsRef.current.values()) {
        client.dispose();
      }
      clientsRef.current.clear();
    };
  }, [workspaceId]);

  const workspaceTerminals = terminals;
  const taskTerminals = useMemo(() => {
    if (!activeTaskId) return [];
    return terminals.filter((t) => idToString(t.task_id) === activeTaskId);
  }, [activeTaskId, terminals]);

  const scopeTerminals = panelState.scope === "workspace" ? workspaceTerminals : taskTerminals;

  const terminalsById = useMemo(() => {
    const map = new Map<string, TerminalSession>();
    for (const terminal of terminals) {
      const id = idToString(terminal.id);
      if (id) map.set(id, terminal);
    }
    return map;
  }, [terminals]);

  useEffect(() => {
    if (!open) {
      autoCreateWorkspaceRef.current = false;
    }
  }, [open]);


  useEffect(() => {
    if (!layoutHydrated) return;
    setPanelState((prev) => {
      const next = { ...prev };
      const taskState = reconcileScopeState(prev.scopes.task, taskTerminals);
      const workspaceState = reconcileScopeState(prev.scopes.workspace, workspaceTerminals);
      next.scopes = {
        task: taskState,
        workspace: workspaceState,
      };
      if (prev.scope === "task" && !activeTaskId) {
        next.scope = "workspace";
      }
      return next;
    });
  }, [activeTaskId, layoutHydrated, taskTerminals, workspaceTerminals]);

  const scopeState = panelState.scopes[panelState.scope];
  const activeGroup = useMemo(() => {
    if (scopeState.groups.length === 0) return null;
    const activeId = scopeState.activeGroupId ?? scopeState.groups[0].id;
    return scopeState.groups.find((group) => group.id === activeId) ?? scopeState.groups[0];
  }, [scopeState]);
  const activeGroupId = activeGroup?.id ?? null;
  const activeLayout = activeGroup?.layout ?? null;
  const activeLeafId = resolveActiveLeafId(activeLayout, activeGroup?.activeLeafId ?? null);
  const activeTerminalId = activeLayout ? findTerminalIdForLeaf(activeLayout, activeLeafId) : null;
  const orderedTerminals = useMemo(() => {
    const byId = new Map(scopeTerminals.map((t) => [idToString(t.id), t]));
    const ordered: TerminalSession[] = [];
    for (const id of scopeState.tabOrder) {
      const t = byId.get(id);
      if (t) ordered.push(t);
    }
    for (const t of scopeTerminals) {
      const id = idToString(t.id);
      if (id && !scopeState.tabOrder.includes(id)) ordered.push(t);
    }
    return ordered;
  }, [scopeState.tabOrder, scopeTerminals]);

  const groupInfoByTerminal = useMemo(() => {
    const info = new Map<string, { position: "top" | "middle" | "bottom"; size: number }>();
    scopeState.groups.forEach((group) => {
      const ids = terminalIdsInLayout(group.layout);
      if (ids.length <= 1) return;
      const groupSet = new Set(ids);
      const ordered = scopeState.tabOrder.filter((id) => groupSet.has(id));
      const orderedIds = ordered.length > 0 ? ordered : ids;
      orderedIds.forEach((id, idx) => {
        const position = idx === 0 ? "top" : idx === orderedIds.length - 1 ? "bottom" : "middle";
        info.set(id, { position, size: orderedIds.length });
      });
    });
    return info;
  }, [scopeState.groups, scopeState.tabOrder]);

  const updateScopeState = useCallback(
    (scope: TerminalScope, updater: (state: TerminalPanelScopeState) => TerminalPanelScopeState) => {
      setPanelState((prev) => ({
        ...prev,
        scopes: {
          ...prev.scopes,
          [scope]: updater(prev.scopes[scope]),
        },
      }));
    },
    [],
  );

  const focusTerminal = useCallback((terminalId: string) => {
    const client = clientsRef.current.get(terminalId);
    if (!client) return;
    client.focus();
  }, []);

  const scheduleFitAll = useCallback(() => {
    if (resizeFrameRef.current) window.cancelAnimationFrame(resizeFrameRef.current);
    resizeFrameRef.current = window.requestAnimationFrame(() => {
      for (const client of clientsRef.current.values()) {
        client.fit();
      }
    });
  }, []);

  const focusActive = useCallback(() => {
    if (!activeTerminalId) return;
    focusTerminal(activeTerminalId);
  }, [activeTerminalId, focusTerminal]);

  const setScope = useCallback(
    (nextScope: TerminalScope) => {
      setPanelState((prev) => {
        if (prev.scope === nextScope) return prev;
        if (nextScope === "task" && !activeTaskId) return prev;
        if (nextScope === "task") {
          autoCreateWorkspaceRef.current = true;
        }
        return { ...prev, scope: nextScope };
      });
    },
    [activeTaskId],
  );

  const createTerminalSession = useCallback(
    async (opts: CreateTerminalOptions, targetScope: TerminalScope, insertAfterId?: string | null) => {
      const effectiveScope = targetScope === "task" && !activeTaskId ? "workspace" : targetScope;
      if (!workspaceId) return null;
      const req: CreateTerminalRequest = {
        task_id: opts.taskId ?? null,
        session_id: opts.sessionId ?? null,
        worktree_id: opts.worktreeId ?? null,
        cwd: opts.cwd ?? null,
      };
      let created: TerminalSession;
      try {
        created = await createWorkspaceTerminal(workspaceId, req);
      } catch {
        return null;
      }
      setTerminals((prev) => [...prev, created]);
      const id = idToString(created.id);
      if (!id) return null;
      updateScopeState(effectiveScope, (state) => {
        let tabOrder = state.tabOrder;
        if (!tabOrder.includes(id)) {
          if (insertAfterId && tabOrder.includes(insertAfterId)) {
            const index = tabOrder.indexOf(insertAfterId);
            tabOrder = [...tabOrder.slice(0, index + 1), id, ...tabOrder.slice(index + 1)];
          } else {
            tabOrder = [...tabOrder, id];
          }
        }
        return { ...state, tabOrder };
      });
      if (panelState.scope !== effectiveScope) {
        if (effectiveScope === "task") {
          autoCreateWorkspaceRef.current = true;
        }
        setPanelState((prev) => ({ ...prev, scope: effectiveScope }));
      }
      return id;
    },
    [activeTaskId, panelState.scope, updateScopeState, workspaceId],
  );

  const createTerminalForScope = useCallback(
    async (opts: CreateTerminalOptions) => {
      const targetScope = opts.scope ?? panelState.scope;
      const effectiveScope = targetScope === "task" && !activeTaskId ? "workspace" : targetScope;
      const id = await createTerminalSession(opts, effectiveScope);
      if (!id) return null;
      updateScopeState(effectiveScope, (state) => {
        const nextGroup = createSingleTerminalGroup(id);
        return {
          ...state,
          groups: [...state.groups, nextGroup],
          activeGroupId: nextGroup.id,
        };
      });
      return id;
    },
    [activeTaskId, createTerminalSession, panelState.scope, updateScopeState],
  );

  useEffect(() => {
    if (!open || !layoutHydrated) return;
    if (panelState.scope !== "workspace") return;
    if (workspaceTerminals.length > 0) return;
    if (autoCreateWorkspaceRef.current) return;
    autoCreateWorkspaceRef.current = true;
    void createTerminalForScope({ scope: "workspace" });
  }, [createTerminalForScope, layoutHydrated, open, panelState.scope, workspaceTerminals.length]);

  useImperativeHandle(
    ref,
    () => ({
      createTerminal: async (opts) => {
        const terminal = await createTerminalForScope(opts);
        return terminal;
      },
      focusTerminal,
      focusActive,
      setScope,
    }),
    [createTerminalForScope, focusActive, focusTerminal, setScope],
  );

  const handleNewTerminal = useCallback(async () => {
    const scope = panelState.scope;
    const baseTaskId = scope === "task" ? activeTaskId : null;
    await createTerminalForScope({
      taskId: baseTaskId,
      sessionId: scope === "task" ? activeSessionId : null,
      scope,
    });
  }, [activeSessionId, activeTaskId, createTerminalForScope, panelState.scope]);

  const splitTerminalForId = useCallback(
    async (terminalId: string | null) => {
      if (!terminalId) return;
      const scope = panelState.scope;
      const baseTaskId = scope === "task" ? activeTaskId : null;
      const createdId = await createTerminalSession(
        {
          taskId: baseTaskId,
          sessionId: scope === "task" ? activeSessionId : null,
          scope,
        },
        scope,
        terminalId,
      );
      if (!createdId) return;
      updateScopeState(scope, (state) => {
        const match = findGroupForTerminal(state.groups, terminalId);
        if (!match) {
          const baseGroup = createSingleTerminalGroup(terminalId);
          const layout = splitLeaf(baseGroup.layout, baseGroup.activeLeafId ?? baseGroup.layout.id, createdId, "horizontal");
          const activeLeafId = findLeafIdForTerminal(layout, createdId) ?? firstLeafId(layout);
          return {
            ...state,
            groups: [...state.groups, { ...baseGroup, layout, activeLeafId }],
            activeGroupId: baseGroup.id,
          };
        }
        const groupIndex = state.groups.findIndex((group) => group.id === match.group.id);
        if (groupIndex < 0) return state;
        const layout = splitLeaf(match.group.layout, match.leafId, createdId, "horizontal");
        const activeLeafId = findLeafIdForTerminal(layout, createdId) ?? firstLeafId(layout);
        const nextGroups = [...state.groups];
        nextGroups[groupIndex] = {
          ...match.group,
          layout,
          activeLeafId: activeLeafId ?? match.group.activeLeafId,
        };
        return {
          ...state,
          groups: nextGroups,
          activeGroupId: match.group.id,
        };
      });
      focusTerminal(createdId);
      scheduleFitAll();
    },
    [
      activeSessionId,
      activeTaskId,
      createTerminalSession,
      focusTerminal,
      panelState.scope,
      scheduleFitAll,
      updateScopeState,
    ],
  );

  const handleUnsplitTerminal = useCallback(
    (terminalId: string | null) => {
      if (!terminalId) return;
      const scope = panelState.scope;
      updateScopeState(scope, (state) => {
        const match = findGroupForTerminal(state.groups, terminalId);
        if (!match) return state;
        const ids = terminalIdsInLayout(match.group.layout);
        if (ids.length < 2) return state;
        const updatedLayout = removeTerminalFromLayout(match.group.layout, terminalId);
        if (!updatedLayout) return state;
        const updatedGroup: TerminalGroupState = {
          ...match.group,
          layout: updatedLayout,
          activeLeafId: resolveActiveLeafId(updatedLayout, match.group.activeLeafId),
        };
        const newGroup = createSingleTerminalGroup(terminalId);
        const groups = [...state.groups];
        const index = groups.findIndex((group) => group.id === match.group.id);
        if (index >= 0) {
          groups.splice(index, 1, updatedGroup);
          groups.splice(index + 1, 0, newGroup);
        } else {
          groups.push(newGroup);
        }
        return {
          ...state,
          groups,
          activeGroupId: newGroup.id,
        };
      });
      scheduleFitAll();
    },
    [panelState.scope, scheduleFitAll, updateScopeState],
  );

  const beginRenameTerminal = useCallback(
    (terminalId: string) => {
      const current = titleOverrides[terminalId] ?? terminalsById.get(terminalId)?.title ?? "";
      setRenamingId(terminalId);
      setRenameValue(current);
    },
    [terminalsById, titleOverrides],
  );

  const cancelRenameTerminal = useCallback(() => {
    setRenamingId(null);
    setRenameValue("");
  }, []);

  const commitRenameTerminal = useCallback(() => {
    if (!renamingId) return;
    const terminal = terminalsById.get(renamingId);
    const fallback = terminal?.title ?? "";
    const trimmed = renameValue.trim();
    setTitleOverrides((prev) => {
      const next = { ...prev };
      if (!trimmed || trimmed === fallback) {
        delete next[renamingId];
      } else {
        next[renamingId] = trimmed;
      }
      return next;
    });
    setRenamingId(null);
    setRenameValue("");
  }, [renameValue, renamingId, terminalsById]);

  useEffect(() => {
    if (open) return;
    setContextMenu(null);
    cancelRenameTerminal();
  }, [cancelRenameTerminal, open]);

  const handleKillTerminal = useCallback(
    async (terminalId: string | null) => {
      if (!terminalId) return;
      try {
        await deleteTerminal(terminalId);
      } catch {
        // ignore failures for already-closed terminals
      }
      setTerminals((prev) => prev.filter((t) => idToString(t.id) !== terminalId));
      setTitleOverrides((prev) => {
        if (!(terminalId in prev)) return prev;
        const next = { ...prev };
        delete next[terminalId];
        return next;
      });
      if (renamingId === terminalId) {
        cancelRenameTerminal();
      }
      setPanelState((prev) => {
        const next = { ...prev };
        (Object.keys(prev.scopes) as TerminalScope[]).forEach((scope) => {
          const state = prev.scopes[scope];
          const tabOrder = state.tabOrder.filter((id) => id !== terminalId);
          let groups = state.groups
            .map((group) => {
              const layout = removeTerminalFromLayout(group.layout, terminalId);
              if (!layout) return null;
              const activeLeafId = resolveActiveLeafId(layout, group.activeLeafId);
              return { ...group, layout, activeLeafId };
            })
            .filter(Boolean) as TerminalGroupState[];
          if (groups.length === 0 && tabOrder.length > 0) {
            groups = [createSingleTerminalGroup(tabOrder[0])];
          }
          let activeGroupId = state.activeGroupId;
          if (activeGroupId && !groups.some((group) => group.id === activeGroupId)) {
            activeGroupId = groups[0]?.id ?? null;
          }
          next.scopes[scope] = { ...state, tabOrder, groups, activeGroupId };
        });
        return next;
      });
    },
    [cancelRenameTerminal, renamingId],
  );

  const handleSelectTerminal = useCallback(
    (terminalId: string) => {
      updateScopeState(panelState.scope, (state) => {
        const match = findGroupForTerminal(state.groups, terminalId);
        if (match) {
          const nextGroups = state.groups.map((group) =>
            group.id === match.group.id ? { ...group, activeLeafId: match.leafId } : group,
          );
          return { ...state, groups: nextGroups, activeGroupId: match.group.id };
        }
        const nextGroup = createSingleTerminalGroup(terminalId);
        return {
          ...state,
          groups: [...state.groups, nextGroup],
          activeGroupId: nextGroup.id,
        };
      });
      focusTerminal(terminalId);
    },
    [focusTerminal, panelState.scope, updateScopeState],
  );

  const handleScopeChange = useCallback(
    (next: TerminalScope) => {
      if (next === panelState.scope) return;
      setPanelState((prev) => ({ ...prev, scope: next }));
    },
    [panelState.scope],
  );

  const handleLayoutActivate = useCallback(
    (leafId: string, _terminalId: string) => {
      updateScopeState(panelState.scope, (state) => ({
        ...state,
        groups: state.groups.map((group) =>
          group.id === (state.activeGroupId ?? state.groups[0]?.id) ? { ...group, activeLeafId: leafId } : group,
        ),
        activeGroupId: state.activeGroupId ?? state.groups[0]?.id ?? null,
      }));
    },
    [panelState.scope, updateScopeState],
  );

  const handleSplitResize = useCallback(
    (splitId: string, ratio: number) => {
      updateScopeState(panelState.scope, (state) => {
        if (state.groups.length === 0) return state;
        const activeGroupId = state.activeGroupId ?? state.groups[0].id;
        const groups = state.groups.map((group) =>
          group.id === activeGroupId
            ? { ...group, layout: updateSplitRatio(group.layout, splitId, ratio) }
            : group,
        );
        return { ...state, groups, activeGroupId };
      });
      scheduleFitAll();
    },
    [panelState.scope, scheduleFitAll, updateScopeState],
  );

  useLayoutEffect(() => {
    if (!open) return;
    scheduleFitAll();
    return () => {
      if (resizeFrameRef.current) window.cancelAnimationFrame(resizeFrameRef.current);
      resizeFrameRef.current = null;
    };
  }, [height, open, scheduleFitAll]);

  useLayoutEffect(() => {
    if (!open) return;
    scheduleFitAll();
  }, [activeGroupId, open, scheduleFitAll]);

  useEffect(() => {
    if (!open) return;
    const onResize = () => {
      scheduleFitAll();
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [open, scheduleFitAll]);

  const scopeDisabled = panelState.scope === "task" && !activeTaskId;
  const contextTerminal = contextMenu ? terminalsById.get(contextMenu.terminalId) ?? null : null;
  const contextGroupInfo = contextMenu ? findGroupForTerminal(scopeState.groups, contextMenu.terminalId) : null;
  const contextGroupSize = contextGroupInfo ? terminalIdsInLayout(contextGroupInfo.group.layout).length : 0;
  const canUnsplit = contextGroupSize > 1;

  return (
    <div className="wb-terminal-panel">
      <div className="wb-terminal-body" aria-hidden={!open}>
        <div className="wb-terminal-tabs">
          <div className="wb-terminal-toolbar">
            <button type="button" className="wb-terminal-action" title="New terminal" onClick={handleNewTerminal}>
              <Plus size={14} />
            </button>
            <div className="wb-terminal-toolbar-spacer" />
            <button type="button" className="wb-terminal-action" title="Hide terminal panel" onClick={onRequestClose}>
              <ChevronDown size={14} />
            </button>
          </div>
          <div className="wb-terminal-scope">
            <button
              type="button"
              className={panelState.scope === "task" ? "wb-terminal-scope-active" : ""}
              onClick={() => handleScopeChange("task")}
              disabled={!activeTaskId}
            >
              Task
            </button>
            <button
              type="button"
              className={panelState.scope === "workspace" ? "wb-terminal-scope-active" : ""}
              onClick={() => handleScopeChange("workspace")}
            >
              Workspace
            </button>
          </div>
          <div className="wb-terminal-tablist" aria-disabled={scopeDisabled}>
            {orderedTerminals.length === 0 && (
              <div className="wb-terminal-tab-empty">No terminals</div>
            )}
            {orderedTerminals.map((terminal) => {
              const id = idToString(terminal.id);
              if (!id) return null;
              const active = id === activeTerminalId;
              const exited = terminal.status === "exited";
              const groupInfo = groupInfoByTerminal.get(id) ?? null;
              const title = titleOverrides[id] ?? terminal.title;
              const isRenaming = renamingId === id;
              return (
                <div
                  key={id}
                  className={`wb-terminal-tab ${active ? "wb-terminal-tab-active" : ""} ${
                    exited ? "wb-terminal-tab-exited" : ""
                  }`}
                  onClick={() => handleSelectTerminal(id)}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    setContextMenu({ terminalId: id, x: e.clientX, y: e.clientY });
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      handleSelectTerminal(id);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  {groupInfo ? (
                    <span
                      className={`wb-terminal-tab-split wb-terminal-tab-split-${groupInfo.position}`}
                      aria-hidden="true"
                    />
                  ) : (
                    <TerminalIcon size={14} />
                  )}
                  {isRenaming ? (
                    <input
                      ref={renameInputRef}
                      className="wb-terminal-tab-input"
                      value={renameValue}
                      onChange={(e) => setRenameValue(e.target.value)}
                      onClick={(e) => e.stopPropagation()}
                      onMouseDown={(e) => e.stopPropagation()}
                      onBlur={commitRenameTerminal}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          commitRenameTerminal();
                        } else if (e.key === "Escape") {
                          e.preventDefault();
                          cancelRenameTerminal();
                        }
                      }}
                    />
                  ) : (
                    <span className="wb-terminal-tab-title">{title}</span>
                  )}
                  {exited && <span className="wb-terminal-tab-exit">Exited</span>}
                  <span className="wb-terminal-tab-spacer" />
                  {!isRenaming && (
                    <span className="wb-terminal-tab-actions">
                      <button
                        type="button"
                        className="wb-terminal-tab-action"
                        onClick={(e) => {
                          e.stopPropagation();
                          void splitTerminalForId(id);
                        }}
                        title="Split terminal"
                        aria-label="Split terminal"
                      >
                        <SplitSquareVertical size={12} />
                      </button>
                      <button
                        type="button"
                        className="wb-terminal-tab-action wb-terminal-tab-close"
                        onClick={(e) => {
                          e.stopPropagation();
                          void handleKillTerminal(id);
                        }}
                        title="Kill terminal"
                        aria-label="Kill terminal"
                      >
                        <X size={12} />
                      </button>
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
        <div className="wb-terminal-view">
          {scopeState.groups.length > 0 ? (
            scopeState.groups.map((group) => {
              const groupActiveLeafId = resolveActiveLeafId(group.layout, group.activeLeafId);
              const isActive = group.id === (activeGroupId ?? scopeState.groups[0].id);
              return (
                <div
                  key={group.id}
                  className={`wb-terminal-group ${isActive ? "wb-terminal-group-active" : "wb-terminal-group-hidden"}`}
                >
                  <TerminalSplitView
                    node={group.layout}
                    activeLeafId={groupActiveLeafId}
                    onActivate={handleLayoutActivate}
                    onResize={handleSplitResize}
                    clients={clientsRef}
                  />
                </div>
              );
            })
          ) : (
            <div className="wb-terminal-empty">No terminals yet.</div>
          )}
        </div>
      </div>
      {contextMenu && (
        <div
          ref={contextMenuRef}
          className="wb-menu wb-terminal-menu"
          role="menu"
          style={contextMenuStyle ?? { left: contextMenu.x, top: contextMenu.y, position: "fixed" }}
        >
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              setContextMenu(null);
              if (contextTerminal) beginRenameTerminal(contextMenu.terminalId);
            }}
            disabled={!contextTerminal}
            role="menuitem"
          >
            Rename
          </button>
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              setContextMenu(null);
              void splitTerminalForId(contextTerminal ? contextMenu.terminalId : null);
            }}
            disabled={!contextTerminal}
            role="menuitem"
          >
            Split
          </button>
          <button
            type="button"
            className="wb-menu-item"
            onClick={() => {
              setContextMenu(null);
              handleUnsplitTerminal(contextTerminal ? contextMenu.terminalId : null);
            }}
            disabled={!contextTerminal || !canUnsplit}
            role="menuitem"
          >
            Unsplit
          </button>
          <button
            type="button"
            className="wb-menu-item wb-menu-item-danger"
            onClick={() => {
              setContextMenu(null);
              void handleKillTerminal(contextTerminal ? contextMenu.terminalId : null);
            }}
            disabled={!contextTerminal}
            role="menuitem"
          >
            Kill terminal
          </button>
        </div>
      )}
    </div>
  );

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

  function reconcileScopeState(state: TerminalPanelScopeState, scopeTerminals: TerminalSession[]): TerminalPanelScopeState {
    const ids = scopeTerminals.map((t) => idToString(t.id)).filter(Boolean) as string[];
    const allowed = new Set(ids);
    const tabOrder = state.tabOrder.filter((id) => allowed.has(id));
    const newIds = ids.filter((id) => !tabOrder.includes(id));
    const nextOrder = tabOrder.concat(newIds);
    let groups = state.groups
      .map((group) => {
        const layout = pruneLayout(group.layout, allowed);
        if (!layout) return null;
        const activeLeafId = resolveActiveLeafId(layout, group.activeLeafId);
        return { ...group, layout, activeLeafId };
      })
      .filter(Boolean) as TerminalGroupState[];

    if (groups.length === 0 && nextOrder.length > 0) {
      groups = [createSingleTerminalGroup(nextOrder[0])];
    }

    let activeGroupId = state.activeGroupId;
    if (activeGroupId && !groups.some((group) => group.id === activeGroupId)) {
      activeGroupId = groups[0]?.id ?? null;
    }
    if (!activeGroupId && groups.length > 0) {
      activeGroupId = groups[0].id;
    }
    return {
      groups,
      activeGroupId,
      tabOrder: nextOrder,
    };
  }
});

function TerminalSplitView({
  node,
  activeLeafId,
  onActivate,
  onResize,
  clients,
}: {
  node: TerminalLayoutNode;
  activeLeafId: string | null;
  onActivate: (leafId: string, terminalId: string) => void;
  onResize: (splitId: string, ratio: number) => void;
  clients: React.MutableRefObject<Map<string, TerminalClient>>;
}) {
  if (node.kind === "leaf") {
    return (
      <TerminalLeaf
        leaf={node}
        active={node.id === activeLeafId}
        onActivate={onActivate}
        clients={clients}
      />
    );
  }
  const isHorizontal = node.direction === "horizontal";
  const ratio = node.ratio;
  return (
    <div className={`wb-terminal-split ${isHorizontal ? "wb-terminal-split-horizontal" : "wb-terminal-split-vertical"}`}>
      <div className="wb-terminal-split-pane" style={{ flexBasis: `${ratio * 100}%` }}>
        <TerminalSplitView node={node.first} activeLeafId={activeLeafId} onActivate={onActivate} onResize={onResize} clients={clients} />
      </div>
      <TerminalSplitResizer direction={node.direction} onResize={(next) => onResize(node.id, next)} />
      <div className="wb-terminal-split-pane" style={{ flexBasis: `${(1 - ratio) * 100}%` }}>
        <TerminalSplitView node={node.second} activeLeafId={activeLeafId} onActivate={onActivate} onResize={onResize} clients={clients} />
      </div>
    </div>
  );
}

function TerminalSplitResizer({
  direction,
  onResize,
}: {
  direction: SplitDirection;
  onResize: (ratio: number) => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      const parent = ref.current?.parentElement;
      if (!parent) return;
      const rect = parent.getBoundingClientRect();
      const onMove = (ev: MouseEvent) => {
        const nextRatio = direction === "horizontal"
          ? Math.min(0.9, Math.max(0.1, (ev.clientX - rect.left) / rect.width))
          : Math.min(0.9, Math.max(0.1, (ev.clientY - rect.top) / rect.height));
        onResize(nextRatio);
      };
      const onUp = () => {
        window.removeEventListener("mousemove", onMove);
        window.removeEventListener("mouseup", onUp);
      };
      window.addEventListener("mousemove", onMove);
      window.addEventListener("mouseup", onUp);
    },
    [direction, onResize],
  );
  return (
    <div
      ref={ref}
      className={`wb-terminal-splitter ${direction === "horizontal" ? "wb-terminal-splitter-vertical" : "wb-terminal-splitter-horizontal"}`}
      onMouseDown={onMouseDown}
    />
  );
}

function TerminalLeaf({
  leaf,
  active,
  onActivate,
  clients,
}: {
  leaf: Extract<TerminalLayoutNode, { kind: "leaf" }>;
  active: boolean;
  onActivate: (leafId: string, terminalId: string) => void;
  clients: React.MutableRefObject<Map<string, TerminalClient>>;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalId = leaf.terminalId;
  useLayoutEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const client = clients.current.get(terminalId);
    if (!client) return;
    client.attach(el);
    const ro = new ResizeObserver(() => client.fit());
    ro.observe(el);
    return () => ro.disconnect();
  }, [clients, terminalId]);

  return (
    <div
      ref={containerRef}
      className={`wb-terminal-pane ${active ? "wb-terminal-pane-active" : ""}`}
      onMouseDown={() => onActivate(leaf.id, terminalId)}
    />
  );
}
