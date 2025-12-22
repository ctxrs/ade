import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { listTracks, type Track } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Tracks">;

export function TrackListScreen({ route, navigation }: Props): React.JSX.Element {
  const { taskId, taskTitle } = route.params;
  const { config } = useConnection();
  const theme = useContextTokens();
  const { data, isLoading, error, refetch, isRefetching } = useQuery({
    queryKey: ["tracks", taskId, config?.baseUrl],
    enabled: !!config,
    queryFn: () => listTracks(config!, taskId),
  });

  if (!config) return <ErrorView message="Connect to a daemon first." />;
  if (isLoading && !data) return <LoadingView />;
  if (error) return <ErrorView message={(error as Error).message} />;

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={data}
        keyExtractor={(item) => item.id}
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={() => void refetch()}
            tintColor={theme.colors.accent}
          />
        }
        ListHeaderComponent={<Text style={styles.header}>{taskTitle}</Text>}
        renderItem={({ item }) => (
          <View style={styles.card}>
            <Pressable
              onPress={() => navigation.navigate("Sessions", { trackId: item.id, trackLabel: item.label })}
            >
              <Text style={styles.title}>{item.label || "Track"}</Text>
              <Text style={styles.meta}>Status: {item.status}</Text>
            </Pressable>
            <Pressable
              onPress={() => navigation.navigate("TrackDiff", { trackId: item.id, trackLabel: item.label })}
              style={styles.diffButton}
            >
              <Text style={styles.diffText}>View Diff</Text>
            </Pressable>
          </View>
        )}
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>No tracks for this task.</Text>
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
