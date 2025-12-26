import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import React, { useMemo } from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { useWorkspaceCatchupSnapshot, useWorkspaceCatchupStore } from "../state/workspaceCatchupStore";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";
import { idToString } from "../api/client";

type Props = NativeStackScreenProps<RootStackParamList, "Tracks">;

export function TrackListScreen({ route, navigation }: Props): React.JSX.Element {
  const { taskId, taskTitle } = route.params;
  const { config } = useConnection();
  const store = useWorkspaceCatchupStore();
  const snapshot = useWorkspaceCatchupSnapshot();
  const theme = useContextTokens();

  if (!config) return <ErrorView message="Connect to a daemon first." />;

  const summary = snapshot.tasksById[taskId];
  const tracks = useMemo(() => summary?.tracks ?? [], [summary?.tracks]);
  const hasData = tracks.length > 0;

  if (!summary && !snapshot.initialized) return <LoadingView />;
  if (!summary && snapshot.initialized) return <ErrorView message="Task not found." />;

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={tracks}
        keyExtractor={(item) => idToString(item.track.id)}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={snapshot.fetchState.active === "loading"}
            onRefresh={() => store.refreshActive()}
            tintColor={theme.colors.accent}
          />
        }
        ListHeaderComponent={<Text style={styles.header}>{taskTitle}</Text>}
        renderItem={({ item }) => (
          <View style={styles.card}>
            <Pressable
              onPress={() =>
                navigation.navigate("Sessions", {
                  trackId: idToString(item.track.id),
                  trackLabel: item.track.label || "Track",
                })
              }
            >
              <Text style={styles.title}>{item.track.label || "Track"}</Text>
              <Text style={styles.meta}>Status: {item.track.status}</Text>
              <Text style={styles.meta}>Sessions: {item.sessions.length}</Text>
            </Pressable>
            <Pressable
              onPress={() =>
                navigation.navigate("TrackDiff", {
                  trackId: idToString(item.track.id),
                  trackLabel: item.track.label || "Track",
                })
              }
              style={styles.diffButton}
            >
              <Text style={styles.diffText}>View Diff</Text>
            </Pressable>
          </View>
        )}
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>{hasData ? "" : "No tracks for this task."}</Text>
          </View>
        }
      />
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  list: {
    padding: t.spacing.md,
    gap: t.spacing.md,
  },
  header: {
    color: t.colors.muted,
    fontWeight: "600",
    textTransform: "uppercase",
  },
  card: {
    borderWidth: 1,
    borderColor: t.colors.border,
    borderRadius: t.radii.lg,
    padding: t.spacing.lg,
    backgroundColor: t.colors.surfaceAlt,
    gap: t.spacing.sm,
  },
  title: {
    color: t.colors.text,
    fontWeight: "700",
    fontSize: t.typography.sizes.lg,
  },
  meta: {
    color: t.colors.muted,
  },
  diffButton: {
    alignSelf: "flex-start",
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.xs,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.accent,
  },
  diffText: {
    color: t.colors.accent,
    fontWeight: "600",
  },
  empty: {
    padding: t.spacing.xl,
    alignItems: "center",
  },
  emptyText: {
    color: t.colors.muted,
  },
}));
