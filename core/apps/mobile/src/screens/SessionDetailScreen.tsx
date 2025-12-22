import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import React, { useMemo, useState } from "react";
import {
  FlatList,
  KeyboardAvoidingView,
  Platform,
  RefreshControl,
  Text,
  TextInput,
  TouchableOpacity,
  View,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { listMessages, postMessage, type Message } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "SessionDetail">;

export function SessionDetailScreen({ route }: Props): React.JSX.Element {
  const { sessionId, sessionTitle } = route.params;
  const { config } = useConnection();
  const queryClient = useQueryClient();
  const [draft, setDraft] = useState("");
  const theme = useContextTokens();
  const { data, isLoading, error, refetch, isRefetching } = useQuery({
    queryKey: ["messages", sessionId, config?.baseUrl],
    enabled: !!config,
    queryFn: () => listMessages(config!, sessionId),
  });
  const messages = useMemo(
    () =>
      [...(data ?? [])].sort(
        (a, b) => new Date(a.created_at).getTime() - new Date(b.created_at).getTime(),
      ),
    [data],
  );

  const mutation = useMutation({
    mutationFn: (content: string) => postMessage(config!, sessionId, content),
    onSuccess: () => {
      setDraft("");
      void queryClient.invalidateQueries({ queryKey: ["messages", sessionId, config?.baseUrl] });
    },
  });

  if (!config) return <ErrorView message="Connect to a daemon first." />;
  if (isLoading && !data) return <LoadingView />;
  if (error) return <ErrorView message={(error as Error).message} />;

  return (
    <SafeAreaView style={styles.safe}>
      <KeyboardAvoidingView
        style={styles.flex}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        keyboardVerticalOffset={80}
      >
        <FlatList
          data={messages}
          keyExtractor={(item) => item.id}
          contentContainerStyle={styles.list}
          refreshControl={
            <RefreshControl
              refreshing={isRefetching || mutation.isPending}
              onRefresh={() => void refetch()}
              tintColor={theme.colors.accent}
            />
          }
          renderItem={({ item }) => <MessageBubble message={item} />}
          ListHeaderComponent={<Text style={styles.header}>{sessionTitle}</Text>}
        />
        <View style={styles.composer}>
          <TextInput
            placeholder="Send instructions..."
            placeholderTextColor={theme.colors.muted}
            style={styles.input}
            value={draft}
            onChangeText={setDraft}
            multiline
          />
          <TouchableOpacity
            style={[styles.sendButton, draft.trim() ? null : styles.sendDisabled]}
            onPress={() => mutation.mutate(draft.trim())}
            disabled={!draft.trim() || mutation.isPending}
          >
            <Text style={styles.sendLabel}>{mutation.isPending ? "..." : "Send"}</Text>
          </TouchableOpacity>
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

const MessageBubble = ({ message }: { message: Message }) => {
  const isUser = message.role === "user";
  return (
    <View style={[styles.bubble, isUser ? styles.userBubble : styles.assistantBubble]}>
      <Text style={styles.bubbleRole}>{message.role.toUpperCase()}</Text>
      <Text style={styles.bubbleContent}>{message.content}</Text>
      <Text style={styles.bubbleMeta}>{new Date(message.created_at).toLocaleString()}</Text>
    </View>
  );
};

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  flex: { flex: 1 },
  list: {
    padding: t.spacing.md,
    gap: t.spacing.md,
  },
  header: {
    color: t.colors.muted,
    fontWeight: "600",
    textTransform: "uppercase",
    marginBottom: t.spacing.xs,
  },
  composer: {
    padding: t.spacing.md,
    borderTopColor: t.colors.border,
    borderTopWidth: 1,
    backgroundColor: t.colors.surface,
    gap: t.spacing.sm,
  },
  input: {
    minHeight: 60,
    borderRadius: t.radii.md,
    borderWidth: 1,
    borderColor: t.colors.border,
    padding: t.spacing.sm,
    color: t.colors.text,
  },
  sendButton: {
    alignSelf: "flex-end",
    backgroundColor: t.colors.accent,
    paddingVertical: t.spacing.sm,
    paddingHorizontal: t.spacing.lg,
    borderRadius: t.radii.pill,
  },
  sendDisabled: {
    backgroundColor: t.colors.border,
  },
  sendLabel: {
    color: t.colors.text,
    fontWeight: "700",
  },
  bubble: {
    borderRadius: t.radii.lg,
    padding: t.spacing.md,
    gap: t.spacing.xs,
  },
  userBubble: {
    backgroundColor: t.colors.surface,
    alignSelf: "flex-end",
  },
  assistantBubble: {
    backgroundColor: t.colors.surfaceAlt,
    alignSelf: "flex-start",
  },
  bubbleRole: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
    letterSpacing: 1,
  },
  bubbleContent: {
    color: t.colors.text,
    fontSize: t.typography.sizes.md,
  },
  bubbleMeta: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
  },
}));
