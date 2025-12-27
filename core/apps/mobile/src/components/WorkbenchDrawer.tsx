import React, { useMemo, useState } from "react";
import { FlatList, Modal, Pressable, Text, TextInput, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Plus, ChevronDown, Check } from "lucide-react-native";

import { createWorkspace, listWorkspaces, idToString, type WorkspaceSummary } from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import { useWorkspaceSelection } from "../state/WorkspaceSelectionProvider";
import { createContextStyles, useContextTokens } from "../theme";
import { IconButton } from "./IconButton";
import { useWorkspaceCatchupSnapshot, useWorkspaceCatchupStore, type WorkspaceCatchupItem } from "../state/workspaceCatchupStore";
import { TaskActionSheet } from "./TaskActionSheet";
import { useSessionCacheSnapshot, type SessionCacheEntry } from "../state/sessionSupervisor";

export type WorkbenchDrawerProps = {
  activeTaskId: string | null;
  onSelectTaskId: (taskId: string) => void;
  onNavigate: (route: "Settings" | "Diagnostics" | "Connection") => void;
};

export function WorkbenchDrawer({ activeTaskId, onSelectTaskId, onNavigate }: WorkbenchDrawerProps): React.JSX.Element {
  const theme = useContextTokens();
  const insets = useSafeAreaInsets();
  const { config } = useConnection();
  const { workspaceId, workspaceName, setWorkspace } = useWorkspaceSelection();
  const [showArchived, setShowArchived] = useState(false);
  const [workspacePickerOpen, setWorkspacePickerOpen] = useState(false);
  const [createOpen, setCreateOpen] = useState(false);
  const [taskMenuOpen, setTaskMenuOpen] = useState(false);
  const [menuTaskId, setMenuTaskId] = useState<string | null>(null);

  const workspacesQuery = useQuery({
    queryKey: ["workspaces"],
    enabled: Boolean(config),
    queryFn: () => listWorkspaces(config!),
  });

  const createMutation = useMutation({
    mutationFn: async (payload: { name: string; rootPath: string }) =>
      createWorkspace(config!, payload.rootPath, payload.name.trim() ? payload.name.trim() : undefined),
    onSuccess: (ws) => {
      workspacesQuery.refetch().catch(() => {});
      setWorkspace(ws.id, ws.name);
      setCreateOpen(false);
    },
  });

  const body = workspaceId ? (
    <TaskListBody
      activeTaskId={activeTaskId}
      showArchived={showArchived}
      onSelectTaskId={onSelectTaskId}
      onOpenTaskMenu={(taskId) => {
        setMenuTaskId(taskId);
        setTaskMenuOpen(true);
      }}
    />
  ) : null;

  return (
    <View style={[styles.wrap, { paddingBottom: Math.max(insets.bottom, theme.spacing.md) }]}>
      <View style={styles.top}>
        <Pressable
          style={styles.workspaceRow}
          onPress={() => setWorkspacePickerOpen(true)}
          accessibilityRole="button"
        >
          <View style={{ flex: 1 }}>
            <Text style={styles.workspaceLabel}>Workspace</Text>
            <Text style={styles.workspaceName} numberOfLines={1}>
              {workspaceName ?? "Select workspace…"}
            </Text>
          </View>
          <ChevronDown size={18} color={theme.colors.muted} />
        </Pressable>

        <View style={styles.tabRow}>
          <Pressable
            onPress={() => setShowArchived(false)}
            style={[styles.tab, !showArchived ? styles.tabActive : null]}
          >
            <Text style={[styles.tabText, !showArchived ? styles.tabTextActive : null]}>Active</Text>
          </Pressable>
          <Pressable
            onPress={() => setShowArchived(true)}
            style={[styles.tab, showArchived ? styles.tabActive : null]}
          >
            <Text style={[styles.tabText, showArchived ? styles.tabTextActive : null]}>Archived</Text>
          </Pressable>

          <View style={{ flex: 1 }} />
          <IconButton
            icon={<Plus size={18} color={theme.colors.text} />}
            onPress={() => setCreateOpen(true)}
            disabled={!config}
            size={34}
          />
        </View>
      </View>

      <View style={styles.divider} />
      {workspaceId ? body : <EmptyState />}

      <View style={styles.footer}>
        <Pressable style={styles.footerItem} onPress={() => onNavigate("Diagnostics")}>
          <Text style={styles.footerText}>Diagnostics</Text>
        </Pressable>
        <Pressable style={styles.footerItem} onPress={() => onNavigate("Settings")}>
          <Text style={styles.footerText}>Settings</Text>
        </Pressable>
        <Pressable style={styles.footerItem} onPress={() => onNavigate("Connection")}>
          <Text style={styles.footerText}>Connection</Text>
        </Pressable>
      </View>

      <WorkspacePickerModal
        open={workspacePickerOpen}
        onClose={() => setWorkspacePickerOpen(false)}
        workspaces={workspacesQuery.data ?? []}
        loading={workspacesQuery.isLoading}
        selectedId={workspaceId}
        onSelect={(ws) => {
          setWorkspace(ws.id, ws.name);
          setWorkspacePickerOpen(false);
        }}
      />

      <CreateWorkspaceModal
        open={createOpen}
        busy={createMutation.isPending}
        error={createMutation.error ? String(createMutation.error) : null}
        onClose={() => setCreateOpen(false)}
        onCreate={(p) => createMutation.mutate(p)}
      />

      <TaskActionSheet
        open={taskMenuOpen}
        taskId={menuTaskId}
        onClose={() => {
          setTaskMenuOpen(false);
          setMenuTaskId(null);
        }}
      />
    </View>
  );
}

function EmptyState(): React.JSX.Element {
  return (
    <View style={styles.empty}>
      <Text style={styles.emptyTitle}>No workspace selected</Text>
      <Text style={styles.emptyText}>Pick a workspace to view tasks.</Text>
    </View>
  );
}

function TaskListBody({
  activeTaskId,
  showArchived,
  onSelectTaskId,
  onOpenTaskMenu,
}: {
  activeTaskId: string | null;
  showArchived: boolean;
  onSelectTaskId: (taskId: string) => void;
  onOpenTaskMenu: (taskId: string) => void;
}): React.JSX.Element {
  const snapshot = useWorkspaceCatchupSnapshot();
  const store = useWorkspaceCatchupStore();
  const theme = useContextTokens();
  const sessionSnap = useSessionCacheSnapshot();

  const ids = showArchived ? snapshot.archivedIds : snapshot.activeIds;
  const items = useMemo(() => {
    return ids.map((id) => snapshot.tasksById[id]).filter((t): t is WorkspaceCatchupItem => Boolean(t));
  }, [ids, snapshot.tasksById]);

  const taskLiveInfo = useMemo(() => {
    const workingByTask = new Set<string>();
    const errorByTask = new Set<string>();
    const lastAssistantMsByTask: Record<string, number> = {};
    for (const entry of Object.values(sessionSnap.sessions)) {
      const taskId = entry.session ? idToString(entry.session.task_id) : "";
      if (!taskId) continue;
      if (isAgentStillWorking(entry)) workingByTask.add(taskId);
      const hasErrorStatus = entry.session?.status === "failed" || entry.session?.status === "cancelled";
      const hasErrorInSupervisor = Boolean(entry.error);
      const hasErrorEvent = entry.events.some((ev) => ev.event_type === "error");
      if (hasErrorStatus || hasErrorInSupervisor || hasErrorEvent) errorByTask.add(taskId);
      const ms = lastAssistantMessageMs(entry.messages);
      if (ms !== null) lastAssistantMsByTask[taskId] = Math.max(lastAssistantMsByTask[taskId] ?? 0, ms);
    }
    return { workingByTask, errorByTask, lastAssistantMsByTask };
  }, [sessionSnap.sessions]);

  const loading = showArchived ? snapshot.fetchState.archived === "loading" : snapshot.fetchState.active === "loading";
  const onRefresh = () => {
    if (showArchived) store.refreshArchived();
    else store.refreshActive();
  };

  return (
    <FlatList
      data={items}
      keyExtractor={(item) => item.id}
      refreshing={loading}
      onRefresh={onRefresh}
      contentContainerStyle={{ padding: theme.spacing.lg, gap: theme.spacing.sm }}
      renderItem={({ item }) => {
        const selected = item.id === activeTaskId;
        const title = item.task.title ?? "New conversation";
        const indicators = computeTaskIndicators(item, taskLiveInfo);
        return (
          <Pressable
            onPress={() => onSelectTaskId(item.id)}
            onLongPress={() => onOpenTaskMenu(item.id)}
            style={[styles.taskRow, selected ? styles.taskRowActive : null]}
          >
            <View style={styles.taskLeading}>
              {indicators.kind ? (
                <View
                  style={[
                    styles.dot,
                    indicators.kind === "unread"
                      ? styles.dotUnread
                      : indicators.kind === "error"
                        ? styles.dotError
                        : styles.dotRunning,
                  ]}
                />
              ) : (
                <View style={styles.dotSpacer} />
              )}
            </View>
            <View style={{ flex: 1 }}>
              <Text style={styles.taskTitle} numberOfLines={1}>
                {title}
              </Text>
              <Text style={styles.taskMeta} numberOfLines={1}>
                {(() => {
                  const status = indicators.working ? "Running" : String(item.task.status || "active");
                  const age = formatRelativeAgeShort(item.task.last_activity_at ?? item.task.updated_at ?? item.task.created_at);
                  return age ? `${status} • ${age}` : status;
                })()}
              </Text>
            </View>
          </Pressable>
        );
      }}
      ListEmptyComponent={
        snapshot.initialized ? (
          <View style={styles.empty}>
            <Text style={styles.emptyText}>{showArchived ? "No archived tasks." : "No tasks yet."}</Text>
          </View>
        ) : (
          <View style={styles.empty}>
            <Text style={styles.emptyText}>Loading…</Text>
          </View>
        )
      }
    />
  );
}

function computeTaskIndicators(
  item: WorkspaceCatchupItem,
  live: { workingByTask: Set<string>; errorByTask: Set<string>; lastAssistantMsByTask: Record<string, number> },
): { kind: null | "unread" | "error" | "running"; working: boolean } {
  const taskId = item.id;
  const working = live.workingByTask.has(taskId);
  const hasError = live.errorByTask.has(taskId);
  const serverLastAssistantMs = parseMs(item.task.last_assistant_message_at ?? null);
  const liveLastAssistantMs = live.lastAssistantMsByTask[taskId] ?? null;
  const lastAssistantMs =
    liveLastAssistantMs !== null && serverLastAssistantMs !== null
      ? Math.max(liveLastAssistantMs, serverLastAssistantMs)
      : liveLastAssistantMs ?? serverLastAssistantMs;
  const seenMs = parseMs(item.task.assistant_seen_at ?? null);
  const unread = !working && lastAssistantMs !== null && (seenMs === null || lastAssistantMs > seenMs);
  const kind = hasError ? "error" : unread ? "unread" : working ? "running" : null;
  return { kind, working };
}

function parseMs(ts: string | null | undefined): number | null {
  if (!ts) return null;
  const ms = Date.parse(ts);
  return Number.isFinite(ms) ? ms : null;
}

function formatRelativeAgeShort(ts: string | null | undefined): string {
  const ms = ts ? Date.parse(ts) : NaN;
  if (!Number.isFinite(ms)) return "";
  const delta = Math.max(0, Date.now() - ms);
  const sec = Math.floor(delta / 1000);
  if (sec < 30) return "Now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 48) return `${hr}h`;
  const days = Math.floor(hr / 24);
  return `${days}d`;
}

function lastAssistantMessageMs(messages: import("@context/types").Message[]): number | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== "assistant") continue;
    const ms = Date.parse(m.created_at ?? "");
    if (Number.isFinite(ms)) return ms;
  }
  return null;
}

function isAgentStillWorking(entry: SessionCacheEntry): boolean {
  const sess = entry.session;
  if (!sess) return false;
  if (sess.status === "failed" || sess.status === "cancelled") return false;
  if (sess.status === "completed") return false;
  if (sess.status !== "active" && sess.status !== "running") return false;

  const doneLike = new Set(["turn_completed", "turn_interrupted", "turn_failed", "run_done", "run_stopped", "session_done"]);
  const ignored = new Set(["heartbeat", "presence", "ws_connected", "ws_disconnected"]);
  for (let i = entry.events.length - 1; i >= 0; i--) {
    const t = String(entry.events[i]?.event_type ?? "").trim();
    if (!t || ignored.has(t)) continue;
    return !doneLike.has(t);
  }

  // Fallback: if last user msg is newer than last assistant msg, assume it is still working.
  const lastUser = lastRoleMessageMs(entry.messages, "user");
  if (lastUser === null) return false;
  const lastAssistant = lastRoleMessageMs(entry.messages, "assistant");
  return lastAssistant === null || lastUser > lastAssistant;
}

function lastRoleMessageMs(messages: import("@context/types").Message[], role: "user" | "assistant"): number | null {
  for (let i = messages.length - 1; i >= 0; i--) {
    const m = messages[i];
    if (m.role !== role) continue;
    const ms = Date.parse(m.created_at ?? "");
    if (Number.isFinite(ms)) return ms;
  }
  return null;
}

function WorkspacePickerModal({
  open,
  onClose,
  workspaces,
  loading,
  selectedId,
  onSelect,
}: {
  open: boolean;
  onClose: () => void;
  workspaces: WorkspaceSummary[];
  loading: boolean;
  selectedId: string | null;
  onSelect: (ws: WorkspaceSummary) => void;
}): React.JSX.Element {
  const theme = useContextTokens();
  return (
    <Modal visible={open} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.modalBackdrop} onPress={onClose} />
      <View style={styles.modalCard}>
        <Text style={styles.modalTitle}>Switch workspace</Text>
        {loading ? <Text style={styles.modalText}>Loading…</Text> : null}
        {workspaces.map((ws) => {
          const selected = idToString(ws.id) === (selectedId ?? "");
          return (
            <Pressable
              key={ws.id}
              style={styles.modalRow}
              onPress={() => onSelect(ws)}
              accessibilityRole="button"
              accessibilityLabel={ws.name}
            >
              <Text style={styles.modalRowText} numberOfLines={1}>
                {ws.name}
              </Text>
              {selected ? <Check size={18} color={theme.colors.accent} /> : null}
            </Pressable>
          );
        })}
      </View>
    </Modal>
  );
}

function CreateWorkspaceModal({
  open,
  busy,
  error,
  onClose,
  onCreate,
}: {
  open: boolean;
  busy: boolean;
  error: string | null;
  onClose: () => void;
  onCreate: (payload: { name: string; rootPath: string }) => void;
}): React.JSX.Element {
  const theme = useContextTokens();
  const [name, setName] = useState("");
  const [rootPath, setRootPath] = useState("");

  return (
    <Modal visible={open} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.modalBackdrop} onPress={onClose} />
      <View style={styles.modalCard}>
        <Text style={styles.modalTitle}>Add workspace</Text>
        <Text style={styles.modalLabel}>Name</Text>
        <TextInput
          value={name}
          onChangeText={setName}
          placeholder="Workspace name"
          placeholderTextColor={theme.colors.muted}
          style={styles.modalInput}
          editable={!busy}
        />
        <Text style={styles.modalLabel}>Root path</Text>
        <TextInput
          value={rootPath}
          onChangeText={setRootPath}
          placeholder="/path/to/repo"
          placeholderTextColor={theme.colors.muted}
          style={styles.modalInput}
          editable={!busy}
        />
        {error ? <Text style={styles.modalError}>{error}</Text> : null}
        <View style={styles.modalActions}>
          <Pressable style={styles.modalButton} onPress={onClose} disabled={busy}>
            <Text style={styles.modalButtonText}>Cancel</Text>
          </Pressable>
          <Pressable
            style={[styles.modalButton, styles.modalButtonPrimary]}
            onPress={() => onCreate({ name, rootPath })}
            disabled={busy || !rootPath.trim()}
          >
            <Text style={[styles.modalButtonText, styles.modalButtonPrimaryText]}>{busy ? "Adding…" : "Add"}</Text>
          </Pressable>
        </View>
        <Text style={styles.modalHint}>
          Use an absolute path on the daemon host (devbox/laptop). This mirrors the web “Add workspace” flow.
        </Text>
      </View>
    </Modal>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  wrap: { flex: 1 },
  top: { padding: t.spacing.lg, gap: t.spacing.md },
  workspaceRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: t.spacing.sm,
    padding: t.spacing.md,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surfaceAlt,
  },
  workspaceLabel: { color: t.colors.muted, fontSize: t.typography.sizes.xs, fontWeight: "700", textTransform: "uppercase" },
  workspaceName: { color: t.colors.text, fontSize: t.typography.sizes.md, fontWeight: "800" },
  tabRow: { flexDirection: "row", alignItems: "center", gap: t.spacing.sm },
  tab: {
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.xs,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surfaceAlt,
  },
  tabActive: { borderColor: t.colors.accent },
  tabText: { color: t.colors.muted, fontWeight: "700", fontSize: t.typography.sizes.sm },
  tabTextActive: { color: t.colors.accent },
  divider: { height: 1, backgroundColor: t.colors.border },
  taskRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: t.spacing.sm,
    padding: t.spacing.md,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surfaceAlt,
  },
  taskRowActive: { borderColor: t.colors.accent },
  taskLeading: { width: 12, alignItems: "center" },
  dot: { width: 8, height: 8, borderRadius: 4 },
  dotUnread: { backgroundColor: t.colors.accent },
  dotError: { backgroundColor: t.colors.danger },
  dotRunning: { backgroundColor: t.colors.success },
  dotSpacer: { width: 8, height: 8 },
  taskTitle: { color: t.colors.text, fontWeight: "800", fontSize: t.typography.sizes.md },
  taskMeta: { color: t.colors.muted, fontSize: t.typography.sizes.xs, textTransform: "uppercase" },
  empty: { padding: t.spacing.lg, gap: t.spacing.xs },
  emptyTitle: { color: t.colors.text, fontWeight: "800", fontSize: t.typography.sizes.md },
  emptyText: { color: t.colors.muted },
  footer: {
    flexDirection: "row",
    borderTopWidth: 1,
    borderTopColor: t.colors.border,
    backgroundColor: t.colors.surface,
  },
  footerItem: { flex: 1, padding: t.spacing.md, alignItems: "center" },
  footerText: { color: t.colors.accent, fontWeight: "700" },
  modalBackdrop: { position: "absolute", left: 0, top: 0, right: 0, bottom: 0, backgroundColor: "rgba(0,0,0,0.6)" },
  modalCard: {
    marginTop: 120,
    marginHorizontal: t.spacing.lg,
    padding: t.spacing.lg,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surface,
    gap: t.spacing.sm,
  },
  modalTitle: { color: t.colors.text, fontWeight: "900", fontSize: t.typography.sizes.lg },
  modalText: { color: t.colors.muted },
  modalRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: t.spacing.sm,
    gap: t.spacing.sm,
  },
  modalRowText: { flex: 1, color: t.colors.text, fontWeight: "700" },
  modalLabel: { color: t.colors.muted, fontWeight: "700", textTransform: "uppercase", fontSize: t.typography.sizes.xs },
  modalInput: {
    borderWidth: 1,
    borderColor: t.colors.border,
    borderRadius: t.radii.md,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    backgroundColor: t.colors.surfaceAlt,
    color: t.colors.text,
  },
  modalError: { color: t.colors.danger, fontWeight: "700" },
  modalActions: { flexDirection: "row", justifyContent: "flex-end", gap: t.spacing.sm, marginTop: t.spacing.sm },
  modalButton: {
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
  modalButtonPrimary: { backgroundColor: t.colors.accent, borderColor: t.colors.accent },
  modalButtonText: { color: t.colors.text, fontWeight: "800" },
  modalButtonPrimaryText: { color: t.colors.text },
  modalHint: { color: t.colors.muted, fontSize: t.typography.sizes.xs, lineHeight: t.typography.sizes.xs + 4 },
}));
