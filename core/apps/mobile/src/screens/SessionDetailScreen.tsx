import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useMutation } from "@tanstack/react-query";
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

import { idToString, postMessage } from "../api/client";
import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { useOpenSession, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { LoadingView } from "../components/LoadingView";
import { ErrorView } from "../components/ErrorView";
import { createContextStyles, useContextTokens } from "../theme";
import type { Message } from "@ctx/types";

type Props = NativeStackScreenProps<RootStackParamList, "SessionDetail">;

export function SessionDetailScreen({ route }: Props): React.JSX.Element {
  const { sessionId, sessionTitle } = route.params;
  const { config } = useConnection();
  const supervisor = useSessionSupervisor();
  const entry = useSessionEntry(sessionId);
  const [draft, setDraft] = useState("");
  const theme = useContextTokens();

  useOpenSession(sessionId);

  const messages = useMemo(() => entry?.messages ?? [], [entry?.messages]);

  const mutation = useMutation({
    mutationFn: (content: string) => postMessage(config!, sessionId, content, "immediate"),
    onSuccess: () => {
      setDraft("");
      supervisor.refreshSession(sessionId);
    },
  });

  if (!config) return <ErrorView message="Connect to a daemon first." />;
  if (!entry || (entry.loading && messages.length === 0)) return <LoadingView />;
  if (entry.error) return <ErrorView message={entry.error} />;

  return (
    <SafeAreaView style={styles.safe}>
      <KeyboardAvoidingView
        style={styles.flex}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        keyboardVerticalOffset={80}
      >
        <FlatList
          data={messages}
          keyExtractor={(item) => idToString(item.id)}
          contentContainerStyle={styles.list}
          refreshControl={
            <RefreshControl
              refreshing={mutation.isPending}
              onRefresh={() => supervisor.refreshSession(sessionId)}
              tintColor={theme.colors.accent}
            />
          }
          ListHeaderComponent={
            <View style={styles.headerWrap}>
              <Text style={styles.header}>{sessionTitle}</Text>
              {entry.hasMoreTurns ? (
                <TouchableOpacity style={styles.moreButton} onPress={() => supervisor.loadMoreTurns(sessionId)}>
                  <Text style={styles.moreText}>Load earlier turns</Text>
                </TouchableOpacity>
              ) : null}
            </View>
          }
          renderItem={({ item }) => <MessageBubble message={item} />}
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
  headerWrap: {
    gap: t.spacing.sm,
    marginBottom: t.spacing.xs,
  },
  header: {
    color: t.colors.muted,
    fontWeight: "600",
    textTransform: "uppercase",
  },
  moreButton: {
    alignSelf: "flex-start",
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.xs,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
  moreText: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.xs,
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
