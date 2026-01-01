import React, { useMemo, useState } from "react";
import { Alert, Modal, Pressable, Text, TextInput, View } from "react-native";
import { useMutation } from "@tanstack/react-query";

import type { Task } from "@ctx/types";
import {
  archiveTask,
  deleteTask,
  markTaskRead,
  markTaskUnread,
  unarchiveTask,
  updateTaskTitle,
} from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import {
  useMaybeWorkspaceCatchupSnapshot,
  useMaybeWorkspaceCatchupStore,
  type WorkspaceCatchupItem,
} from "../state/workspaceCatchupStore";
import { createContextStyles, useContextTokens } from "../theme";

export function TaskActionSheet({
  open,
  taskId,
  onClose,
}: {
  open: boolean;
  taskId: string | null;
  onClose: () => void;
}): React.JSX.Element | null {
  const theme = useContextTokens();
  const { config } = useConnection();
  const store = useMaybeWorkspaceCatchupStore();
  const snapshot = useMaybeWorkspaceCatchupSnapshot();
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");

  const item: WorkspaceCatchupItem | null = useMemo(() => {
    if (!taskId) return null;
    return snapshot?.tasksById?.[taskId] ?? null;
  }, [snapshot?.tasksById, taskId]);

  const canMutate = Boolean(config && item && store);
  const isArchived = Boolean(item?.task.archived_at);

  const applyUpdate = (task: Task) => {
    store?.applyTaskUpdate(task);
  };

  const renameMutation = useMutation({
    mutationFn: async (title: string) => updateTaskTitle(config!, taskId!, title),
    onSuccess: (task) => {
      applyUpdate(task);
      setRenameOpen(false);
      onClose();
    },
  });

  const archiveMutation = useMutation({
    mutationFn: async () => {
      if (!config || !taskId) throw new Error("Not ready");
      return isArchived ? unarchiveTask(config, taskId) : archiveTask(config, taskId);
    },
    onSuccess: (task) => applyUpdate(task),
    onSettled: () => onClose(),
  });

  const markUnreadMutation = useMutation({
    mutationFn: async () => {
      if (!config || !taskId) throw new Error("Not ready");
      const nextUnread = computeUnread(item!);
      return nextUnread ? markTaskRead(config, taskId) : markTaskUnread(config, taskId);
    },
    onSuccess: (task) => applyUpdate(task),
    onSettled: () => onClose(),
  });

  const deleteMutation = useMutation({
    mutationFn: async () => {
      if (!config || !taskId) throw new Error("Not ready");
      await deleteTask(config, taskId);
      return taskId;
    },
    onSuccess: (deletedId) => {
      store?.applyTaskDelete(deletedId);
    },
    onSettled: () => onClose(),
  });

  if (!open) return null;

  return (
    <>
      <Modal visible={open && !renameOpen} transparent animationType="fade" onRequestClose={onClose}>
        <Pressable style={styles.backdrop} onPress={onClose} />
        <View style={styles.card}>
          <Text style={styles.title}>{item?.task.title ?? "Task"}</Text>
          <Pressable
            style={styles.action}
            onPress={() => {
              if (!item) return;
              setRenameDraft(String(item.task.title ?? "").trim());
              setRenameOpen(true);
            }}
            disabled={!canMutate}
          >
            <Text style={styles.actionText}>Rename</Text>
          </Pressable>
          <Pressable style={styles.action} onPress={() => archiveMutation.mutate()} disabled={!canMutate || archiveMutation.isPending}>
            <Text style={styles.actionText}>{isArchived ? "Unarchive" : "Archive"}</Text>
          </Pressable>
          <Pressable
            style={styles.action}
            onPress={() => markUnreadMutation.mutate()}
            disabled={!canMutate || markUnreadMutation.isPending}
          >
            <Text style={styles.actionText}>{item && computeUnread(item) ? "Mark read" : "Mark unread"}</Text>
          </Pressable>
          <Pressable
            style={[styles.action, styles.destructive]}
            onPress={() => {
              if (!taskId) return;
              Alert.alert("Delete task?", "This cannot be undone.", [
                { text: "Cancel", style: "cancel" },
                { text: "Delete", style: "destructive", onPress: () => deleteMutation.mutate() },
              ]);
            }}
            disabled={!canMutate || deleteMutation.isPending}
          >
            <Text style={[styles.actionText, styles.destructiveText]}>Delete</Text>
          </Pressable>
          <Pressable style={styles.action} onPress={() => {}} disabled>
            <Text style={[styles.actionText, { color: theme.colors.muted }]}>Export (coming soon)</Text>
          </Pressable>
        </View>
      </Modal>

      <Modal visible={renameOpen} transparent animationType="fade" onRequestClose={() => setRenameOpen(false)}>
        <Pressable style={styles.backdrop} onPress={() => setRenameOpen(false)} />
        <View style={styles.card}>
          <Text style={styles.title}>Rename task</Text>
          <TextInput
            value={renameDraft}
            onChangeText={setRenameDraft}
            placeholder="Task title"
            placeholderTextColor={theme.colors.muted}
            style={styles.input}
            autoFocus
          />
          {renameMutation.error ? <Text style={styles.error}>{String(renameMutation.error)}</Text> : null}
          <View style={styles.row}>
            <Pressable style={styles.button} onPress={() => setRenameOpen(false)} disabled={renameMutation.isPending}>
              <Text style={styles.buttonText}>Cancel</Text>
            </Pressable>
            <Pressable
              style={[styles.button, styles.primary]}
              onPress={() => renameMutation.mutate(renameDraft.trim())}
              disabled={!renameDraft.trim() || renameMutation.isPending}
            >
              <Text style={[styles.buttonText, styles.primaryText]}>{renameMutation.isPending ? "Saving…" : "Save"}</Text>
            </Pressable>
          </View>
        </View>
      </Modal>
    </>
  );
}

function computeUnread(item: WorkspaceCatchupItem): boolean {
  const lastAssistant = Date.parse(item.task.last_assistant_message_at ?? "") || null;
  const seen = Date.parse(item.task.assistant_seen_at ?? "") || null;
  return Boolean(lastAssistant !== null && (seen === null || lastAssistant > seen));
}

const styles: Record<string, any> = createContextStyles((t) => ({
  backdrop: { position: "absolute", left: 0, top: 0, right: 0, bottom: 0, backgroundColor: "rgba(0,0,0,0.6)" },
  card: {
    marginTop: 140,
    marginHorizontal: t.spacing.lg,
    padding: t.spacing.lg,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surface,
    gap: t.spacing.sm,
  },
  title: { color: t.colors.text, fontWeight: "900", fontSize: t.typography.sizes.lg },
  action: {
    paddingVertical: t.spacing.sm,
    borderTopWidth: 1,
    borderTopColor: t.colors.border,
  },
  actionText: { color: t.colors.text, fontWeight: "800", fontSize: t.typography.sizes.md },
  destructive: {},
  destructiveText: { color: t.colors.danger },
  input: {
    borderWidth: 1,
    borderColor: t.colors.border,
    borderRadius: t.radii.md,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    backgroundColor: t.colors.surfaceAlt,
    color: t.colors.text,
  },
  error: { color: t.colors.danger, fontWeight: "700" },
  row: { flexDirection: "row", justifyContent: "flex-end", gap: t.spacing.sm, marginTop: t.spacing.sm },
  button: {
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    borderRadius: t.radii.pill,
    borderWidth: 1,
    borderColor: t.colors.border,
  },
  primary: { backgroundColor: t.colors.accent, borderColor: t.colors.accent },
  buttonText: { color: t.colors.text, fontWeight: "800" },
  primaryText: { color: t.colors.text },
}));
