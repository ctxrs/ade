import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import React, { useEffect, useMemo, useState } from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { useWorkspaceSelection } from "../state/WorkspaceSelectionProvider";
import { useWorkspaceCatchupSnapshot, useWorkspaceCatchupStore } from "../state/workspaceCatchupStore";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";
import { idToString } from "../api/client";

const TAB_ACTIVE = "active";
const TAB_ARCHIVED = "archived";

type TabKey = typeof TAB_ACTIVE | typeof TAB_ARCHIVED;

type Props = NativeStackScreenProps<RootStackParamList, "Tasks">;

export function TaskListScreen({ route, navigation }: Props): React.JSX.Element {
  const { workspaceId, workspaceName } = route.params;
  const { config } = useConnection();
  const { workspaceId: selectedId, setWorkspace } = useWorkspaceSelection();
  const store = useWorkspaceCatchupStore();
  const snapshot = useWorkspaceCatchupSnapshot();
  const theme = useContextTokens();
  const [tab, setTab] = useState<TabKey>(TAB_ACTIVE);

  useEffect(() => {
    if (workspaceId && selectedId !== workspaceId) {
      setWorkspace(workspaceId, workspaceName);
    }
  }, [workspaceId, workspaceName, selectedId, setWorkspace]);

  useEffect(() => {
    if (tab === TAB_ARCHIVED) {
      store.ensureArchivedLoaded();
    }
  }, [tab, store]);

  if (!config) {
    return <ErrorView message="Connect to a daemon first." />;
  }

  const ids = tab === TAB_ARCHIVED ? snapshot.archivedIds : snapshot.activeIds;
  const tasks = useMemo(
    () => ids.map((id) => snapshot.tasksById[id]).filter(Boolean),
    [ids, snapshot.tasksById],
  );
  const hasData = tasks.length > 0;
  const isLoading = !snapshot.initialized && !hasData;
  const isError = snapshot.fetchState.active === "error" && !hasData;

  if (isLoading) {
    return <LoadingView />;
  }

  if (isError) {
    return <ErrorView message="Failed to load tasks." />;
  }

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={tasks}
        keyExtractor={(item) => item?.task?.id ? idToString(item.task.id) : ""}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={tab === TAB_ARCHIVED ? snapshot.fetchState.archived === "loading" : snapshot.fetchState.active === "loading"}
            onRefresh={() => {
              if (tab === TAB_ARCHIVED) {
                store.refreshArchived();
              } else {
                store.refreshActive();
              }
            }}
            tintColor={theme.colors.accent}
          />
        }
        onEndReached={() => {
          if (tab === TAB_ARCHIVED) {
            store.loadMoreArchived();
          } else {
            store.loadMoreActive();
          }
        }}
        onEndReachedThreshold={0.4}
        ListHeaderComponent={
          <View style={styles.headerWrap}>
            <Text style={styles.header}>{workspaceName}</Text>
            <View style={styles.tabs}>
              <Pressable
                style={[styles.tab, tab === TAB_ACTIVE ? styles.tabActive : null]}
                onPress={() => setTab(TAB_ACTIVE)}
              >
                <Text style={[styles.tabText, tab === TAB_ACTIVE ? styles.tabTextActive : null]}>
                  Active ({snapshot.totalActive})
                </Text>
              </Pressable>
              <Pressable
                style={[styles.tab, tab === TAB_ARCHIVED ? styles.tabActive : null]}
                onPress={() => setTab(TAB_ARCHIVED)}
              >
                <Text style={[styles.tabText, tab === TAB_ARCHIVED ? styles.tabTextActive : null]}>
                  Archived ({snapshot.totalArchived})
                </Text>
              </Pressable>
            </View>
          </View>
        }
        renderItem={({ item }) => (
          <TaskRow
            title={item?.task?.title ?? "Untitled task"}
            description={item?.task?.description}
            status={item?.task?.status ?? ""}
            onPress={() => handlePress(item?.task?.id)}
          />
        )}
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>No tasks yet.</Text>
          </View>
        }
      />
    </SafeAreaView>
  );

  function handlePress(taskId?: { 0: string } | string | null) {
    const tid = idToString(taskId);
    if (!tid) return;
    navigation.navigate("Tracks", { taskId: tid, taskTitle: tasks.find((t) => idToString(t.task.id) === tid)?.task.title ?? "Task" });
  }
}

const TaskRow = ({
  title,
  description,
  status,
  onPress,
}: {
  title: string;
  description?: string | null;
  status: string;
  onPress: () => void;
}) => (
  <Pressable style={styles.card} onPress={onPress}>
    <Text style={styles.title}>{title}</Text>
    {description ? <Text style={styles.description}>{description}</Text> : null}
    <Text style={styles.meta}>Status: {status}</Text>
  </Pressable>
);

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  list: {
    padding: t.spacing.md,
    gap: t.spacing.md,
  },
  headerWrap: {
    gap: t.spacing.sm,
    marginBottom: t.spacing.md,
  },
  header: {
    color: t.colors.muted,
    textTransform: "uppercase",
    letterSpacing: 1,
    fontWeight: "600",
  },
  tabs: {
    flexDirection: "row",
    gap: t.spacing.sm,
  },
  tab: {
    paddingVertical: t.spacing.xs,
    paddingHorizontal: t.spacing.md,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
  tabActive: {
    borderColor: t.colors.accent,
    backgroundColor: t.colors.surfaceAlt,
  },
  tabText: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
    fontWeight: "600",
  },
  tabTextActive: {
    color: t.colors.accent,
  },
  card: {
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    padding: t.spacing.lg,
    gap: t.spacing.xs,
    backgroundColor: t.colors.surfaceAlt,
  },
  title: {
    color: t.colors.text,
    fontWeight: "700",
    fontSize: t.typography.sizes.lg,
  },
  description: {
    color: t.colors.text,
  },
  meta: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
    textTransform: "uppercase",
  },
  empty: {
    padding: t.spacing.xl,
    alignItems: "center",
  },
  emptyText: {
    color: t.colors.muted,
  },
}));
