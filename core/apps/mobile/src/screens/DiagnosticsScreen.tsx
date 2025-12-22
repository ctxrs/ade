import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { FlatList, RefreshControl, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { getDiagnostics, listProviders } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Diagnostics">;

export function DiagnosticsScreen(_: Props): React.JSX.Element {
  const { config } = useConnection();
  const theme = useContextTokens();

  const providersQuery = useQuery({
    queryKey: ["providers", config?.baseUrl],
    enabled: !!config,
    queryFn: () => listProviders(config!),
  });
  const diagnosticsQuery = useQuery({
    queryKey: ["diagnostics", config?.baseUrl],
    enabled: !!config,
    queryFn: () => getDiagnostics(config!),
  });

  if (!config) return <ErrorView message="Connect to a daemon first." />;
  if ((providersQuery.isLoading && !providersQuery.data) || (diagnosticsQuery.isLoading && !diagnosticsQuery.data)) {
    return <LoadingView />;
  }
  if (providersQuery.error) return <ErrorView message={(providersQuery.error as Error).message} />;
  if (diagnosticsQuery.error) return <ErrorView message={(diagnosticsQuery.error as Error).message} />;

  const { data: providers } = providersQuery;
  const { data: diagnostics } = diagnosticsQuery;

  return (
    <SafeAreaView style={styles.safe}>
      <FlatList
        data={providers}
        keyExtractor={(item) => item.provider_id}
        refreshControl={
          <RefreshControl
            refreshing={providersQuery.isRefetching || diagnosticsQuery.isRefetching}
            onRefresh={() => {
              void providersQuery.refetch();
              void diagnosticsQuery.refetch();
            }}
            tintColor={theme.colors.accent}
          />
        }
        contentContainerStyle={styles.list}
        ListHeaderComponent={
          <View style={styles.banner}>
            <Text style={styles.bannerTitle}>Daemon</Text>
            <Text style={styles.bannerText}>
              {diagnostics?.daemon.version} · PID {diagnostics?.daemon.pid} · Data root {diagnostics?.daemon.data_root}
            </Text>
          </View>
        }
        renderItem={({ item }) => (
          <View style={styles.card}>
            <Text style={styles.cardTitle}>{item.provider_id}</Text>
            <Text style={[styles.status, item.health === "ok" ? styles.statusOk : styles.statusBad]}>
              {item.health}
            </Text>
            {item.version ? <Text style={styles.meta}>Version: {item.version}</Text> : null}
            {item.detected_path ? <Text style={styles.meta}>Path: {item.detected_path}</Text> : null}
            {item.diagnostics.map((line: string, idx: number) => (
              <Text key={idx} style={styles.diagnostic}>
                • {line}
              </Text>
            ))}
          </View>
        )}
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
  banner: {
    marginBottom: t.spacing.md,
    padding: t.spacing.lg,
    backgroundColor: t.colors.surfaceAlt,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    gap: t.spacing.xs,
  },
  bannerTitle: {
    color: t.colors.text,
    fontSize: t.typography.sizes.md,
    fontWeight: "700",
  },
  bannerText: {
    color: t.colors.muted,
  },
  card: {
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surface,
    padding: t.spacing.lg,
    gap: t.spacing.xs,
  },
  cardTitle: {
    color: t.colors.text,
    fontSize: t.typography.sizes.lg,
    fontWeight: "700",
  },
  status: {
    textTransform: "uppercase",
    fontWeight: "600",
  },
  statusOk: {
    color: t.colors.success,
  },
  statusBad: {
    color: t.colors.danger,
  },
  meta: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
  },
  diagnostic: {
    color: t.colors.text,
  },
}));
