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

const formatSessionLabel = (title?: string, provider?: string, model?: string) => {
  const trimmed = String(title ?? "").trim();
  if (trimmed) return trimmed;
  if (!provider && !model) return "Session";
  if (provider && model) return `${provider} · ${model}`;
  return provider ?? model ?? "Session";
};

type Props = NativeStackScreenProps<RootStackParamList, "Sessions">;

export function SessionListScreen({ route, navigation }: Props): React.JSX.Element {
  const { taskId, taskTitle } = route.params;
  const { config } = useConnection();
  const store = useWorkspaceCatchupStore();
  const snapshot = useWorkspaceCatchupSnapshot();
  const theme = useContextTokens();

  if (!config) return <ErrorView message="Connect to a daemon first." />;

  const sessions = useMemo(() => {
    const summary = snapshot.tasksById[taskId];
    if (!summary) return [];
    return [...summary.sessions].sort((a, b) =>
      String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")),
    );
  }, [snapshot.tasksById, taskId]);

  if (!snapshot.tasksById[taskId] && !snapshot.initialized) return <LoadingView />;
  if (!snapshot.tasksById[taskId] && snapshot.initialized) return <ErrorView message="Task not found." />;

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={sessions}
        keyExtractor={(item) => idToString(item.session.id)}
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
              style={styles.cardMain}
              onPress={() =>
                navigation.navigate("SessionDetail", {
                  sessionId: idToString(item.session.id),
                  sessionTitle: formatSessionLabel(item.session.title, item.session.provider_id, item.session.model_id),
                })
              }
            >
              <Text style={styles.title}>
                {formatSessionLabel(item.session.title, item.session.provider_id, item.session.model_id)}
              </Text>
              <Text style={styles.meta}>Status: {item.session.status}</Text>
              {item.last_message_preview ? <Text style={styles.preview}>{item.last_message_preview}</Text> : null}
            </Pressable>
            <Pressable
              onPress={() =>
                navigation.navigate("SessionDiff", {
                  sessionId: idToString(item.session.id),
                  sessionTitle: formatSessionLabel(item.session.title, item.session.provider_id, item.session.model_id),
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
            <Text style={styles.emptyText}>No sessions yet.</Text>
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
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    padding: t.spacing.lg,
    gap: t.spacing.xs,
    backgroundColor: t.colors.surfaceAlt,
  },
  cardMain: {
    gap: t.spacing.xs,
  },
  title: {
    color: t.colors.text,
    fontWeight: "700",
    fontSize: t.typography.sizes.lg,
  },
  meta: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
    textTransform: "uppercase",
  },
  preview: {
    color: t.colors.text,
  },
  diffButton: {
    alignSelf: "flex-start",
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.xs,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.accent,
    marginTop: t.spacing.xs,
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
