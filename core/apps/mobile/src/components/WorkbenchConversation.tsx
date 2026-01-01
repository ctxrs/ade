import { useMutation } from "@tanstack/react-query";
import React, { useMemo, useState } from "react";
import { FlatList, Image, KeyboardAvoidingView, Platform, Pressable, Text, TextInput, View } from "react-native";
import { Calculator, Mic, Plus, SendHorizontal } from "lucide-react-native";
import * as ImagePicker from "expo-image-picker";

import type { Message, MessageAttachment } from "@ctx/types";
import { idToString, postMessage } from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import { useOpenSession, useSessionEntry, useSessionSupervisor } from "../state/sessionSupervisor";
import { createContextStyles, useContextTokens } from "../theme";
import { IconButton } from "./IconButton";

type Props = {
  sessionId: string;
  onOpenOptions: () => void;
  onPressDictation: () => void;
};

export function WorkbenchConversation({ sessionId, onOpenOptions, onPressDictation }: Props): React.JSX.Element {
  const theme = useContextTokens();
  const { config } = useConnection();
  const supervisor = useSessionSupervisor();
  const entry = useSessionEntry(sessionId);
  const [draft, setDraft] = useState("");
  const [attachments, setAttachments] = useState<MessageAttachment[]>([]);

  useOpenSession(sessionId);

  const messages = useMemo(() => entry?.messages ?? [], [entry?.messages]);
  const listData = useMemo(() => [...messages].reverse(), [messages]);
  const sending = useMemo(() => Boolean(entry?.queue?.length), [entry?.queue?.length]);

  const mutation = useMutation({
    mutationFn: (content: string) => postMessage(config!, sessionId, content, "immediate", attachments),
    onSuccess: () => {
      setDraft("");
      setAttachments([]);
      supervisor.refreshSession(sessionId);
    },
  });

  const onPickImage = async () => {
    const perm = await ImagePicker.requestMediaLibraryPermissionsAsync();
    if (!perm.granted) return;
    const res = await ImagePicker.launchImageLibraryAsync({
      mediaTypes: ImagePicker.MediaTypeOptions.Images,
      allowsMultipleSelection: false,
      quality: 0.9,
      base64: true,
    });
    if (res.canceled) return;
    const asset = res.assets[0];
    if (!asset?.base64) return;
    const mime = asset.mimeType || "image/*";
    const name = asset.fileName || "image";
    setAttachments((prev) => [
      ...prev,
      { kind: "image", mime_type: mime, data_base64: asset.base64!, name },
    ]);
  };

  if (!config) {
    return (
      <View style={styles.empty}>
        <Text style={styles.emptyText}>Connect to a daemon first.</Text>
      </View>
    );
  }

  if (!entry) {
    return (
      <View style={styles.empty}>
        <Text style={styles.emptyText}>Loading…</Text>
      </View>
    );
  }

  return (
    <KeyboardAvoidingView
      style={styles.flex}
      behavior={Platform.OS === "ios" ? "padding" : undefined}
      keyboardVerticalOffset={80}
    >
      <FlatList
        data={listData}
        keyExtractor={(m) => idToString(m.id)}
        inverted
        keyboardDismissMode="interactive"
        keyboardShouldPersistTaps="handled"
        contentContainerStyle={styles.list}
        renderItem={({ item }) => <MessageRow message={item} />}
      />

      {attachments.length ? (
        <View style={styles.attachmentsRow}>
          <View style={styles.attachmentsThumbs}>
            {attachments.map((a, idx) => {
              if (a.kind !== "image") return null;
              const uri = `data:${a.mime_type};base64,${a.data_base64}`;
              return (
                <Pressable
                  key={`${idx}-${a.name}`}
                  onPress={() => setAttachments((prev) => prev.filter((_, i) => i !== idx))}
                  style={styles.thumbWrap}
                >
                  <Image source={{ uri }} style={styles.thumb} />
                  <View style={styles.thumbRemove}>
                    <Text style={styles.thumbRemoveText}>×</Text>
                  </View>
                </Pressable>
              );
            })}
          </View>
        </View>
      ) : null}

      <View style={styles.composer}>
        <IconButton
          icon={<Plus color={theme.colors.text} size={18} />}
          onPress={onPickImage}
          size={36}
          accessibilityLabel="Add attachment"
        />
        <IconButton
          icon={<Calculator color={theme.colors.text} size={18} />}
          onPress={onOpenOptions}
          size={36}
          accessibilityLabel="Composer options"
        />
        <View style={styles.inputWrap}>
          <TextInput
            placeholder="Send instructions…"
            placeholderTextColor={theme.colors.muted}
            style={styles.input}
            value={draft}
            onChangeText={setDraft}
            multiline
          />
        </View>
        <IconButton
          icon={<Mic color={theme.colors.text} size={18} />}
          onPress={onPressDictation}
          size={36}
          accessibilityLabel="Dictation"
        />
        <IconButton
          icon={<SendHorizontal color={draft.trim() ? theme.colors.text : theme.colors.muted} size={18} />}
          onPress={() => mutation.mutate(draft.trim())}
          disabled={!draft.trim() || mutation.isPending || sending}
          size={36}
          accessibilityLabel="Send message"
        />
      </View>
    </KeyboardAvoidingView>
  );
}

function MessageRow({ message }: { message: Message }): React.JSX.Element {
  const isUser = message.role === "user";
  return (
    <View style={[styles.msgRow, isUser ? styles.msgRowUser : styles.msgRowAssistant]}>
      <View style={[styles.bubble, isUser ? styles.bubbleUser : styles.bubbleAssistant]}>
        <Text style={styles.msgText}>{message.content}</Text>
      </View>
    </View>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  flex: { flex: 1 },
  list: {
    flexGrow: 1,
    justifyContent: "flex-end",
    paddingHorizontal: t.spacing.lg,
    paddingTop: t.spacing.lg,
    paddingBottom: t.spacing.md,
    gap: t.spacing.md,
  },
  msgRow: {
    flexDirection: "row",
  },
  msgRowUser: { justifyContent: "flex-end" },
  msgRowAssistant: { justifyContent: "flex-start" },
  bubble: {
    maxWidth: "88%",
    borderRadius: t.radii.lg,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
  bubbleUser: { backgroundColor: t.colors.surfaceAlt },
  bubbleAssistant: {
    backgroundColor: "transparent",
    borderColor: "transparent",
    borderWidth: 0,
    paddingHorizontal: 0,
    paddingVertical: 0,
  },
  msgText: {
    color: t.colors.text,
    fontSize: t.typography.sizes.md,
    lineHeight: t.typography.sizes.md + 6,
  },
  composer: {
    flexDirection: "row",
    alignItems: "flex-end",
    gap: t.spacing.sm,
    padding: t.spacing.md,
    borderTopWidth: 1,
    borderTopColor: t.colors.border,
    backgroundColor: t.colors.surface,
  },
  inputWrap: { flex: 1 },
  input: {
    minHeight: 40,
    maxHeight: 140,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    backgroundColor: t.colors.surfaceAlt,
    color: t.colors.text,
  },
  empty: { flex: 1, alignItems: "center", justifyContent: "center", padding: t.spacing.lg },
  emptyText: { color: t.colors.muted },
  attachmentsRow: {
    paddingHorizontal: t.spacing.lg,
    paddingTop: t.spacing.md,
    paddingBottom: t.spacing.sm,
    borderTopWidth: 1,
    borderTopColor: t.colors.border,
    backgroundColor: t.colors.surface,
  },
  attachmentsThumbs: { flexDirection: "row", gap: t.spacing.sm },
  thumbWrap: { width: 44, height: 44, borderRadius: t.radii.md, overflow: "hidden" },
  thumb: { width: 44, height: 44 },
  thumbRemove: {
    position: "absolute",
    right: 4,
    top: 4,
    width: 18,
    height: 18,
    borderRadius: 9,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(0,0,0,0.7)",
  },
  thumbRemoveText: { color: "white", fontWeight: "900", fontSize: 14, lineHeight: 14 },
}));
