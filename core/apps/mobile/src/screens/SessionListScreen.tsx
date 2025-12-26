import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import React, { useEffect, useMemo } from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { useWorkspaceCatchupSnapshot, useWorkspaceCatchupStore } from "../state/workspaceCatchupStore";
import { useSessionSupervisor } from "../state/sessionSupervisor";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";
import { idToString } from "../api/client";

const formatSessionLabel = (provider?: string, model?: string) => {
  if (!provider && !model) return "Session";
  if (provider && model) return `${provider} · ${model}`;
  return provider ?? model ?? "Session";
};

type Props = NativeStackScreenProps<RootStackParamList, "Sessions">;

export function SessionListScreen({ route, navigation }: Props): React.JSX.Element {
  const { trackId, trackLabel } = route.params;
  const { config } = useConnection();
  const store = useWorkspaceCatchupStore();
  const snapshot = useWorkspaceCatchupSnapshot();
  const supervisor = useSessionSupervisor();
  const theme = useContextTokens();

  if (!config) return <ErrorView message="Connect to a daemon first." />;

  const trackSummary = useMemo(() => {
    for (const task of Object.values(snapshot.tasksById)) {
      const found = task.tracks.find((track) => idToString(track.track.id) === trackId);
      if (found) return found;
    }
    return null;
  }, [snapshot.tasksById, trackId]);

  const sessions = useMemo(() => {
    if (!trackSummary) return [];
    return [...trackSummary.sessions].sort((a, b) =>
      String(a.session.created_at ?? "").localeCompare(String(b.session.created_at ?? "")),
    );
  }, [trackSummary]);

  useEffect(() => {
    if (sessions.length === 0) return;
    const closers = sessions.map((s) => supervisor.openSession(idToString(s.session.id)));
    return () => closers.forEach((close) => close());
  }, [sessions, supervisor]);

  if (!trackSummary && !snapshot.initialized) return <LoadingView />;
  if (!trackSummary && snapshot.initialized) return <ErrorView message="Track not found." />;

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
        ListHeaderComponent={<Text style={styles.header}>{trackLabel}</Text>}
        renderItem={({ item }) => (
          <Pressable
            style={styles.card}
            onPress={() =>
              navigation.navigate("SessionDetail", {
                sessionId: idToString(item.session.id),
                sessionTitle: formatSessionLabel(item.session.provider_id, item.session.model_id),
              })
            }
          >
            <Text style={styles.title}>{formatSessionLabel(item.session.provider_id, item.session.model_id)}</Text>
            <Text style={styles.meta}>Status: {item.session.status}</Text>
            {item.last_message_preview ? <Text style={styles.preview}>{item.last_message_preview}</Text> : null}
          </Pressable>
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
  empty: {
    padding: t.spacing.xl,
    alignItems: "center",
  },
  emptyText: {
    color: t.colors.muted,
  },
}));
