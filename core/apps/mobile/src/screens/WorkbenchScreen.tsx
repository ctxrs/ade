import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import React, { useEffect, useMemo, useState } from "react";
import { Alert, Pressable, ScrollView, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { AlignJustify, Ellipsis, SquarePen } from "lucide-react-native";

import type { RootStackParamList } from "../navigation/types";
import { idToString } from "../api/client";
import { createContextStyles, useContextTokens } from "../theme";
import { IconButton } from "../components/IconButton";
import { SideDrawer } from "../components/SideDrawer";
import { WorkbenchDrawer } from "../components/WorkbenchDrawer";
import { useWorkspaceSelection } from "../state/WorkspaceSelectionProvider";
import { useMaybeWorkspaceCatchupSnapshot, useMaybeWorkspaceCatchupStore } from "../state/workspaceCatchupStore";
import { useWorkbenchSelection } from "../state/WorkbenchSelectionProvider";
import { useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { WorkbenchConversation } from "../components/WorkbenchConversation";
import { TaskActionSheet } from "../components/TaskActionSheet";
import { useConnection } from "../state/ConnectionProvider";
import { WorkbenchComposerOptionsSheet } from "../components/WorkbenchComposerOptionsSheet";

type Props = NativeStackScreenProps<RootStackParamList, "Workbench">;

export function WorkbenchScreen({ navigation }: Props): React.JSX.Element {
  const theme = useContextTokens();
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [taskMenuOpen, setTaskMenuOpen] = useState(false);
  const [optionsOpen, setOptionsOpen] = useState(false);
  const { config } = useConnection();
  const { workspaceId } = useWorkspaceSelection();
  const snapshot = useMaybeWorkspaceCatchupSnapshot();
  const catchupStore = useMaybeWorkspaceCatchupStore();
  const selection = useWorkbenchSelection();
  const activeTaskId = selection.taskId;
  const supervisor = useSessionSupervisor();

  useEffect(() => {
    if (!workspaceId) {
      selection.clearSelection();
      setDrawerOpen(true);
      return;
    }
    if (!snapshot?.initialized) return;
    if (activeTaskId && snapshot.tasksById[activeTaskId]) return;
    const fallback = snapshot.activeIds[0] ?? null;
    if (fallback) selection.setSelection({ taskId: fallback, trackId: null, sessionId: null });
  }, [workspaceId, activeTaskId, snapshot?.initialized, snapshot?.activeIds, snapshot?.tasksById, selection]);

  const title = useMemo(() => {
    const taskTitle = activeTaskId && snapshot?.tasksById?.[activeTaskId]?.task?.title;
    return taskTitle || "Workbench";
  }, [activeTaskId, snapshot?.tasksById]);

  const activeTask = useMemo(() => {
    if (!snapshot?.initialized || !activeTaskId) return null;
    return snapshot.tasksById[activeTaskId] ?? null;
  }, [snapshot?.initialized, snapshot?.tasksById, activeTaskId]);

  const selectedTrack = useMemo(() => {
    const tracks = activeTask?.tracks ?? [];
    if (!tracks.length) return null;
    if (selection.trackId) {
      const found = tracks.find((t) => idToString(t.track.id) === selection.trackId);
      if (found) return found;
    }
    return tracks[0] ?? null;
  }, [activeTask?.tracks, selection.trackId]);

  const selectedSessionId = useMemo(() => {
    if (!selectedTrack) return null;
    const sessions = selectedTrack.sessions ?? [];
    const mainSessions = sessions.filter((s) => s.session.relationship !== "sub_agent");
    const candidates = mainSessions.length > 0 ? mainSessions : sessions;
    if (selection.sessionId && sessions.some((s) => idToString(s.session.id) === selection.sessionId)) {
      return selection.sessionId;
    }
    const primary = idToString(selectedTrack.primary_session_id ?? "");
    if (primary && sessions.some((s) => idToString(s.session.id) === primary)) return primary;
    const running = candidates.find((s) => s.session.status === "active" || s.session.status === "running");
    if (running) return idToString(running.session.id);
    const newest = [...candidates].sort((a, b) =>
      String(b.session.created_at ?? "").localeCompare(String(a.session.created_at ?? "")),
    )[0];
    return newest ? idToString(newest.session.id) : null;
  }, [selectedTrack, selection.sessionId]);

  const sessionEntry = useSessionEntry(selectedSessionId ?? "");

  useEffect(() => {
    if (!snapshot?.initialized) return;
    if (!activeTask) return;
    const tracks = activeTask.tracks ?? [];
    if (!tracks.length) return;
    const currentTrackId = selection.trackId;
    const resolvedTrackId = idToString((selectedTrack as any)?.track?.id ?? "");
    if (!currentTrackId || currentTrackId !== resolvedTrackId) {
      selection.setSelection({ trackId: resolvedTrackId || null, sessionId: null });
      return;
    }
    if (selectedSessionId && selectedSessionId !== selection.sessionId) {
      selection.setSelection({ sessionId: selectedSessionId });
    }
  }, [snapshot?.initialized, activeTask, selectedTrack, selectedSessionId, selection]);

  const warmSessionIds = useMemo(() => {
    if (!snapshot?.initialized) return { activeTaskSessionIds: [] as string[], warm: [] as string[] };
    const activeTaskSummary = activeTaskId ? snapshot.tasksById[activeTaskId] : null;
    const activeTaskSessionIds: string[] = [];
    if (activeTaskSummary) {
      for (const tr of activeTaskSummary.tracks) {
        for (const s of tr.sessions) {
          const sid = idToString(s.session.id);
          if (sid) activeTaskSessionIds.push(sid);
        }
      }
    }

    const activeSet = new Set(activeTaskSessionIds);
    const candidates: Array<{ id: string; running: boolean; updatedAt: number }> = [];
    for (const taskId of snapshot.activeIds) {
      const task = snapshot.tasksById[taskId];
      if (!task) continue;
      for (const tr of task.tracks) {
        for (const sess of tr.sessions) {
          const sid = idToString(sess.session.id);
          if (!sid || activeSet.has(sid)) continue;
          const last =
            Date.parse(sess.last_message_at ?? "") ||
            Date.parse(sess.session.updated_at ?? "") ||
            Date.parse(sess.session.created_at ?? "") ||
            0;
          const running = sess.session.status === "active" || sess.session.status === "running";
          candidates.push({ id: sid, running, updatedAt: last });
        }
      }
    }
    candidates.sort((a, b) => {
      if (a.running !== b.running) return a.running ? -1 : 1;
      return b.updatedAt - a.updatedAt;
    });
    return { activeTaskSessionIds, warm: candidates.map((c) => c.id).slice(0, 20) };
  }, [snapshot?.initialized, snapshot?.activeIds, snapshot?.tasksById, activeTaskId]);

  useEffect(() => {
    supervisor.setActiveTaskSessionIds(warmSessionIds.activeTaskSessionIds);
    supervisor.setWarmSessionIds(warmSessionIds.warm);
  }, [supervisor, warmSessionIds.activeTaskSessionIds, warmSessionIds.warm]);

  return (
    <SafeAreaView style={styles.safe} edges={["bottom"]}>
      <SideDrawer
        open={drawerOpen}
        onRequestClose={() => setDrawerOpen(false)}
        drawer={
          <WorkbenchDrawer
            activeTaskId={activeTaskId}
            onSelectTaskId={(taskId) => {
              selection.setSelection({ taskId, trackId: null, sessionId: null });
              setDrawerOpen(false);
            }}
            onNavigate={(route) => {
              setDrawerOpen(false);
              navigation.navigate(route);
            }}
          />
        }
      >
        <View style={styles.header}>
          <IconButton
            icon={<AlignJustify color={theme.colors.text} size={20} />}
            onPress={() => setDrawerOpen((v) => !v)}
            accessibilityLabel={drawerOpen ? "Close navigation drawer" : "Open navigation drawer"}
          />
          <Text style={styles.headerTitle} numberOfLines={1}>
            {title}
          </Text>
          <View style={styles.headerRight}>
            <IconButton
              icon={<SquarePen color={theme.colors.text} size={20} />}
              onPress={() => navigation.navigate("NewTask")}
              accessibilityLabel="New conversation"
            />
            <IconButton
              icon={<Ellipsis color={theme.colors.text} size={20} />}
              onPress={() => setTaskMenuOpen(true)}
              disabled={!activeTaskId}
              accessibilityLabel="Task menu"
            />
          </View>
        </View>

        <View style={styles.body}>
          {activeTask && activeTask.tracks.length > 1 ? (
            <ScrollView
              horizontal
              showsHorizontalScrollIndicator={false}
              contentContainerStyle={styles.trackRow}
            >
              {activeTask.tracks.map((t) => {
                const tid = idToString(t.track.id);
                const selected = selection.trackId === tid || (!selection.trackId && tid === idToString((selectedTrack as any)?.track?.id ?? ""));
                const label = t.track.label || "Track";
                const running = t.sessions.some((s) => s.session.status === "active" || s.session.status === "running");
                return (
                  <Pressable
                    key={tid}
                    onPress={() => selection.setSelection({ trackId: tid, sessionId: null })}
                    style={[styles.trackCard, selected ? styles.trackCardActive : null]}
                  >
                    <Text style={styles.trackLabel} numberOfLines={1}>
                      {label}
                    </Text>
                    <Text style={styles.trackMeta} numberOfLines={1}>
                      {running ? "Running" : t.track.status}
                    </Text>
                  </Pressable>
                );
              })}
            </ScrollView>
          ) : null}

          {selectedSessionId ? (
            <WorkbenchConversation
              sessionId={selectedSessionId}
              onOpenOptions={() => setOptionsOpen(true)}
              onPressDictation={() => {
                Alert.alert(
                  "Dictation",
                  "Dictation isn’t wired up on native yet. We’ll implement it once we have a clean PCM streaming path that matches the web daemon endpoint.",
                );
              }}
            />
          ) : (
            <View style={styles.emptyState}>
              <Text style={styles.placeholder}>
                {workspaceId ? "Select a task to start." : "Select a workspace to start."}
              </Text>
            </View>
          )}
        </View>
      </SideDrawer>
      <TaskActionSheet open={taskMenuOpen} taskId={activeTaskId} onClose={() => setTaskMenuOpen(false)} />
      {config && workspaceId && activeTaskId ? (
        <WorkbenchComposerOptionsSheet
          open={optionsOpen}
          onClose={() => setOptionsOpen(false)}
          conn={config}
          workspaceId={workspaceId}
          taskId={activeTaskId}
          currentProviderId={sessionEntry?.session?.provider_id}
          currentModelId={sessionEntry?.session?.model_id}
          onCreated={({ trackId, sessionId }) => {
            catchupStore?.refreshActive();
            selection.setSelection({ taskId: activeTaskId, trackId, sessionId });
            supervisor.refreshSession(sessionId);
          }}
        />
      ) : null}
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: t.spacing.sm,
    paddingHorizontal: t.spacing.lg,
    paddingVertical: t.spacing.sm,
    borderBottomWidth: 1,
    borderBottomColor: t.colors.border,
    backgroundColor: t.colors.surface,
  },
  headerTitle: {
    flex: 1,
    color: t.colors.text,
    fontWeight: "800",
    fontSize: t.typography.sizes.md,
    textAlign: "center",
  },
  headerRight: { flexDirection: "row", alignItems: "center", gap: t.spacing.xs },
  body: {
    flex: 1,
    paddingTop: 0,
  },
  trackRow: {
    paddingHorizontal: t.spacing.lg,
    gap: t.spacing.sm,
    paddingBottom: t.spacing.md,
  },
  trackCard: {
    minWidth: 120,
    maxWidth: 200,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surfaceAlt,
    gap: 2,
  },
  trackCardActive: { borderColor: t.colors.accent },
  trackLabel: { color: t.colors.text, fontWeight: "800" },
  trackMeta: { color: t.colors.muted, fontSize: t.typography.sizes.xs, textTransform: "uppercase" },
  placeholder: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.md,
  },
  emptyState: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: t.spacing.lg,
  },
}));
