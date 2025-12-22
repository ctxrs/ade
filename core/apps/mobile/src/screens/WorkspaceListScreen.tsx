import { useQuery } from "@tanstack/react-query";
import React from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { listWorkspaces, type Workspace } from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";
import { LoadingView } from "../components/LoadingView";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";

type Props = NativeStackScreenProps<RootStackParamList, "Workspaces">;

export function WorkspaceListScreen({ navigation }: Props): React.JSX.Element {
  const { config } = useConnection();
  const theme = useContextTokens();
  const { data, isLoading, refetch, isRefetching, error } = useQuery({
    queryKey: ["workspaces", config?.baseUrl],
    enabled: !!config,
    queryFn: () => listWorkspaces(config!),
  });

  if (!config) {
    return <ErrorView message="Connect to a daemon first." />;
  }

  if (isLoading && !data) {
    return <LoadingView />;
  }

  if (error) {
    return <ErrorView message={(error as Error).message} />;
  }

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={data}
        keyExtractor={(item) => item.id}
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={() => void refetch()}
            tintColor={theme.colors.accent}
          />
        }
        contentContainerStyle={styles.list}
        renderItem={({ item }) => (
          <Pressable
            style={styles.card}
            onPress={() =>
              navigation.navigate("Tasks", { workspaceId: item.id, workspaceName: item.name })
            }
          >
            <Text style={styles.name}>{item.name}</Text>
            <Text style={styles.path}>{item.root_path}</Text>
          </Pressable>
        )}
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>No workspaces found.</Text>
          </View>
        }
      />
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: {
    flex: 1,
    backgroundColor: t.colors.background,
  },
  list: {
    padding: t.spacing.md,
    gap: t.spacing.md,
  },
  card: {
    padding: t.spacing.lg,
    borderRadius: t.radii.lg,
    backgroundColor: t.colors.surfaceAlt,
    borderColor: t.colors.border,
    borderWidth: 1,
    gap: t.spacing.xs,
  },
  name: {
    fontSize: t.typography.sizes.lg,
    color: t.colors.text,
    fontWeight: "700",
  },
  path: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.sm,
  },
  empty: {
    padding: t.spacing.xl,
    alignItems: "center",
  },
  emptyText: {
    color: t.colors.muted,
  },
}));
