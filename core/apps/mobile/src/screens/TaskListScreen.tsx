import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { FlatList, Pressable, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { listTasks, type Task } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Tasks">;

export function TaskListScreen({ route, navigation }: Props): React.JSX.Element {
  const { workspaceId, workspaceName } = route.params;
  const { config } = useConnection();
  const theme = useContextTokens();

  const { data, isLoading, error, refetch, isRefetching } = useQuery({
    queryKey: ["tasks", workspaceId, config?.baseUrl],
    enabled: !!config,
    queryFn: () => listTasks(config!, workspaceId),
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
        contentContainerStyle={styles.list}
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={() => void refetch()}
            tintColor={theme.colors.accent}
          />
        }
        ListHeaderComponent={<Text style={styles.header}>{workspaceName}</Text>}
        renderItem={({ item }) => <TaskRow task={item} onPress={() => handlePress(item)} />}
        ListEmptyComponent={
          <View style={styles.empty}>
            <Text style={styles.emptyText}>No tasks yet.</Text>
          </View>
        }
      />
    </SafeAreaView>
  );

  function handlePress(task: Task) {
    navigation.navigate("Tracks", { taskId: task.id, taskTitle: task.title });
  }
}

const TaskRow = ({ task, onPress }: { task: Task; onPress: () => void }) => (
  <Pressable style={styles.card} onPress={onPress}>
    <Text style={styles.title}>{task.title}</Text>
    {task.description ? <Text style={styles.description}>{task.description}</Text> : null}
    <Text style={styles.meta}>Status: {task.status}</Text>
  </Pressable>
);

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  list: {
    padding: t.spacing.md,
    gap: t.spacing.md,
  },
  header: {
    color: t.colors.muted,
    textTransform: "uppercase",
    letterSpacing: 1,
    fontWeight: "600",
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
