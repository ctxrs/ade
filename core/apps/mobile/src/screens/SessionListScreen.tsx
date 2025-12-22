import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { listSessionsForTrack, type Session } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Sessions">;

export function SessionListScreen({ route, navigation }: Props): React.JSX.Element {
  const { trackId, trackLabel } = route.params;
  const { config } = useConnection();
  const theme = useContextTokens();

  const { data, isLoading, error, refetch, isRefetching } = useQuery({
    queryKey: ["sessions", trackId, config?.baseUrl],
    enabled: !!config,
    queryFn: () => listSessionsForTrack(config!, trackId),
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
        ListHeaderComponent={<Text style={styles.header}>{trackLabel || "Track"}</Text>}
        renderItem={({ item }) => (
          <Pressable
            style={styles.card}
            onPress={() =>
              navigation.navigate("SessionDetail", {
                sessionId: item.id,
                sessionTitle: `${item.provider_id} · ${item.model_id}`,
              })
            }
          >
            <Text style={styles.title}>{item.provider_id}</Text>
            <Text style={styles.meta}>Model: {item.model_id}</Text>
            <Text style={styles.meta}>Status: {item.status}</Text>
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
    backgroundColor: t.colors.surfaceAlt,
    padding: t.spacing.lg,
    gap: t.spacing.xs,
  },
  title: {
    color: t.colors.text,
    fontSize: t.typography.sizes.lg,
    fontWeight: "700",
  },
  meta: {
    color: t.colors.muted,
  },
  empty: {
    padding: t.spacing.xl,
    alignItems: "center",
  },
  emptyText: {
    color: t.colors.muted,
  },
}));
