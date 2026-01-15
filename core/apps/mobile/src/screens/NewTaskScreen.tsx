import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useMutation, useQuery } from "@tanstack/react-query";
import React, { useMemo, useState } from "react";
import { KeyboardAvoidingView, Modal, Platform, Pressable, Text, TextInput, TouchableOpacity, View } from "react-native";
import { ChevronDown } from "lucide-react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import type { RootStackParamList } from "../navigation/types";
import { createSession, createTask, getProviderOptions, idToString, listProviders, postMessage } from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import { useWorkspaceSelection } from "../state/WorkspaceSelectionProvider";
import { useWorkbenchSelection } from "../state/WorkbenchSelectionProvider";
import { useMaybeWorkspaceActiveSnapshotStore } from "../state/workspaceActiveSnapshotStore";
import { useSessionSupervisor } from "../state/sessionSupervisor";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "NewTask">;

export function NewTaskScreen({ navigation }: Props): React.JSX.Element {
  const theme = useContextTokens();
  const { config } = useConnection();
  const { workspaceId } = useWorkspaceSelection();
  const selection = useWorkbenchSelection();
  const activeSnapshotStore = useMaybeWorkspaceActiveSnapshotStore();
  const supervisor = useSessionSupervisor();
  const [text, setText] = useState("");
  const [providerPickerOpen, setProviderPickerOpen] = useState(false);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");

  const providersQuery = useQuery({
    queryKey: ["providers"],
    enabled: Boolean(config),
    queryFn: () => listProviders(config!),
  });

  const installedProviders = useMemo(() => {
    const providers = providersQuery.data ?? [];
    return providers
      .filter((p) => p.installed && p.health === "ok")
      .map((p) => p.provider_id)
      .filter(Boolean);
  }, [providersQuery.data]);

  const effectiveProviderId = providerId || (installedProviders.includes("codex") ? "codex" : installedProviders[0] || "");

  const optionsQuery = useQuery({
    queryKey: ["providerOptions", workspaceId, effectiveProviderId],
    enabled: Boolean(config && workspaceId && effectiveProviderId),
    queryFn: () => getProviderOptions(config!, workspaceId!, effectiveProviderId),
  });

  const modelIds = useMemo(() => extractModelIds(optionsQuery.data?.models), [optionsQuery.data?.models]);
  const effectiveModelId = modelId || modelIds[0] || (effectiveProviderId ? "default" : "");

  const createMutation = useMutation({
    mutationFn: async () => {
      if (!config) throw new Error("Connect to a daemon first.");
      if (!workspaceId) throw new Error("Select a workspace first.");
      const prompt = text.trim();
      if (!prompt) throw new Error("Prompt is required.");
      const title = deriveTaskTitle(prompt);
      const task = await createTask(config, workspaceId, title, undefined, { create_default_session: false });
      const taskId = idToString(task.id);
      if (!taskId) throw new Error("Failed to create task.");
      const session = await createSession(
        config,
        taskId,
        effectiveProviderId || "codex",
        effectiveModelId || "default",
        { env_target: "worktree" },
      );
      const sessionId = idToString(session.id);
      if (!sessionId) throw new Error("Failed to create session.");
      supervisor.refreshSession(sessionId);
      await postMessage(config, sessionId, prompt, "immediate");
      supervisor.refreshSession(sessionId);
      return { taskId, sessionId };
    },
    onSuccess: ({ taskId, sessionId }) => {
      activeSnapshotStore?.refreshActive();
      selection.setSelection({ taskId, sessionId });
      navigation.goBack();
    },
  });

  return (
    <SafeAreaView style={styles.safe}>
      <KeyboardAvoidingView
        style={styles.flex}
        behavior={Platform.OS === "ios" ? "padding" : undefined}
        keyboardVerticalOffset={80}
      >
        <View style={styles.header}>
          <Text style={styles.title}>New conversation</Text>
          <TouchableOpacity onPress={() => navigation.goBack()}>
            <Text style={{ color: theme.colors.accent, fontWeight: "700" }}>Close</Text>
          </TouchableOpacity>
        </View>

        <View style={styles.body}>
          <Text style={styles.label}>Prompt</Text>
          <TextInput
            value={text}
            onChangeText={setText}
            placeholder="Describe what you want…"
            placeholderTextColor={theme.colors.muted}
            style={styles.input}
            multiline
          />

          <View style={styles.row}>
            <Pressable style={styles.picker} onPress={() => setProviderPickerOpen(true)}>
              <Text style={styles.pickerLabel}>Harness</Text>
              <View style={styles.pickerValueRow}>
                <Text style={styles.pickerValue} numberOfLines={1}>
                  {effectiveProviderId || "Select…"}
                </Text>
                <ChevronDown size={16} color={theme.colors.muted} />
              </View>
            </Pressable>
            <Pressable
              style={styles.picker}
              onPress={() => setModelPickerOpen(true)}
              disabled={!effectiveProviderId}
            >
              <Text style={styles.pickerLabel}>Model</Text>
              <View style={styles.pickerValueRow}>
                <Text style={styles.pickerValue} numberOfLines={1}>
                  {effectiveModelId || "Select…"}
                </Text>
                <ChevronDown size={16} color={theme.colors.muted} />
              </View>
            </Pressable>
          </View>

          {createMutation.error ? <Text style={styles.error}>{String(createMutation.error)}</Text> : null}
          <TouchableOpacity
            style={[styles.primaryButton, !text.trim() ? styles.primaryButtonDisabled : null]}
            onPress={() => createMutation.mutate()}
            disabled={!text.trim() || createMutation.isPending}
          >
            <Text style={styles.primaryButtonText}>{createMutation.isPending ? "Starting…" : "Start"}</Text>
          </TouchableOpacity>

          <ProviderPickerModal
            open={providerPickerOpen}
            title="Select harness"
            items={installedProviders}
            selected={effectiveProviderId}
            onClose={() => setProviderPickerOpen(false)}
            onSelect={(id) => {
              setProviderId(id);
              setModelId("");
              setProviderPickerOpen(false);
            }}
          />

          <ProviderPickerModal
            open={modelPickerOpen}
            title="Select model"
            items={modelIds.length ? modelIds : ["default"]}
            selected={effectiveModelId}
            onClose={() => setModelPickerOpen(false)}
            onSelect={(id) => {
              setModelId(id);
              setModelPickerOpen(false);
            }}
          />
        </View>
      </KeyboardAvoidingView>
    </SafeAreaView>
  );
}

function ProviderPickerModal({
  open,
  title,
  items,
  selected,
  onClose,
  onSelect,
}: {
  open: boolean;
  title: string;
  items: string[];
  selected: string;
  onClose: () => void;
  onSelect: (id: string) => void;
}): React.JSX.Element {
  const theme = useContextTokens();
  return (
    <Modal visible={open} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.modalBackdrop} onPress={onClose} />
      <View style={styles.modalCard}>
        <Text style={styles.modalTitle}>{title}</Text>
        {items.map((id) => {
          const active = id === selected;
          return (
            <Pressable key={id} style={styles.modalRow} onPress={() => onSelect(id)}>
              <Text style={styles.modalRowText}>{id}</Text>
              {active ? <Text style={{ color: theme.colors.accent, fontWeight: "900" }}>✓</Text> : null}
            </Pressable>
          );
        })}
      </View>
    </Modal>
  );
}

function deriveTaskTitle(_prompt: string): string {
  return "New Task";
}

function extractModelIds(models: any): string[] {
  if (!models) return [];
  if (Array.isArray(models)) {
    const out = models
      .map((m) => (typeof m === "string" ? m : typeof m?.id === "string" ? m.id : ""))
      .map((s) => String(s).trim())
      .filter(Boolean);
    return Array.from(new Set(out));
  }
  if (typeof models === "object") {
    const choices = (models as any).choices;
    if (Array.isArray(choices)) return extractModelIds(choices);
    return Object.keys(models).filter((k) => k && k !== "choices");
  }
  return [];
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  flex: { flex: 1 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: t.spacing.lg,
    paddingVertical: t.spacing.md,
    borderBottomWidth: 1,
    borderBottomColor: t.colors.border,
    backgroundColor: t.colors.surface,
  },
  title: {
    color: t.colors.text,
    fontWeight: "800",
    fontSize: t.typography.sizes.lg,
  },
  body: {
    padding: t.spacing.lg,
    gap: t.spacing.sm,
  },
  label: {
    color: t.colors.muted,
    fontWeight: "700",
    textTransform: "uppercase",
    fontSize: t.typography.sizes.xs,
  },
  input: {
    minHeight: 140,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    padding: t.spacing.md,
    color: t.colors.text,
    backgroundColor: t.colors.surfaceAlt,
  },
  row: { flexDirection: "row", gap: t.spacing.sm },
  picker: {
    flex: 1,
    borderWidth: 1,
    borderColor: t.colors.border,
    borderRadius: t.radii.lg,
    backgroundColor: t.colors.surfaceAlt,
    padding: t.spacing.md,
    gap: 6,
  },
  pickerLabel: { color: t.colors.muted, fontWeight: "800", textTransform: "uppercase", fontSize: t.typography.sizes.xs },
  pickerValueRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: t.spacing.sm },
  pickerValue: { color: t.colors.text, fontWeight: "800", flex: 1 },
  primaryButton: {
    marginTop: t.spacing.md,
    backgroundColor: t.colors.accent,
    borderRadius: t.radii.pill,
    paddingVertical: t.spacing.md,
    alignItems: "center",
  },
  primaryButtonDisabled: { backgroundColor: t.colors.border },
  primaryButtonText: { color: t.colors.text, fontWeight: "900" },
  error: { color: t.colors.danger, fontWeight: "800" },
  modalBackdrop: { position: "absolute", left: 0, top: 0, right: 0, bottom: 0, backgroundColor: "rgba(0,0,0,0.6)" },
  modalCard: {
    marginTop: 140,
    marginHorizontal: t.spacing.lg,
    padding: t.spacing.lg,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surface,
    gap: t.spacing.sm,
  },
  modalTitle: { color: t.colors.text, fontWeight: "900", fontSize: t.typography.sizes.lg },
  modalRow: { paddingVertical: t.spacing.sm, flexDirection: "row", justifyContent: "space-between", gap: t.spacing.sm },
  modalRowText: { color: t.colors.text, fontWeight: "800" },
}));
