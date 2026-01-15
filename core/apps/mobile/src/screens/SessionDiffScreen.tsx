import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useQuery } from "@tanstack/react-query";
import React from "react";
import { RefreshControl, ScrollView, Text } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { getSessionDiff } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "SessionDiff">;

export function SessionDiffScreen({ route }: Props): React.JSX.Element {
  const { sessionId } = route.params;
  const { config } = useConnection();
  const theme = useContextTokens();
  const query = useQuery({
    queryKey: ["sessionDiff", sessionId, config?.baseUrl],
    enabled: !!config,
    queryFn: () => getSessionDiff(config!, sessionId),
  });

  if (!config) return <ErrorView message="Connect to a daemon first." />;
  if (query.isLoading && !query.data) return <LoadingView />;
  if (query.error) return <ErrorView message={(query.error as Error).message} />;

  return (
    <SafeAreaView style={styles.safe}>
      <ScrollView
        contentContainerStyle={styles.scroll}
        refreshControl={
          <RefreshControl
            refreshing={query.isRefetching}
            onRefresh={() => void query.refetch()}
            tintColor={theme.colors.accent}
          />
        }
      >
        <Text style={styles.diff}>{query.data?.diff || "No diff available."}</Text>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  scroll: {
    padding: t.spacing.md,
  },
  diff: {
    color: t.colors.text,
    fontFamily: "Courier",
    fontSize: t.typography.sizes.sm,
    lineHeight: 18,
    backgroundColor: t.colors.surfaceAlt,
    borderRadius: t.radii.lg,
    padding: t.spacing.md,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
}));
