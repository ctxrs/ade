import React, { useEffect, useMemo, useState } from "react";
import { Modal, Pressable, Text, View } from "react-native";
import { useMutation, useQuery } from "@tanstack/react-query";
import { ChevronDown } from "lucide-react-native";

import { createSession, getProviderOptions, idToString, listProviders } from "../api/client";
import type { ConnectionConfig } from "../api/client";
import { createContextStyles, useContextTokens } from "../theme";

export type WorkbenchComposerOptionsSheetProps = {
  open: boolean;
  onClose: () => void;
  conn: ConnectionConfig;
  workspaceId: string;
  taskId: string;
  currentProviderId?: string | null;
  currentModelId?: string | null;
  onCreated: (result: { sessionId: string; providerId: string; modelId: string }) => void;
};

export function WorkbenchComposerOptionsSheet(props: WorkbenchComposerOptionsSheetProps): React.JSX.Element {
  const theme = useContextTokens();
  const { open, onClose, conn, workspaceId, taskId } = props;
  const [providerPickerOpen, setProviderPickerOpen] = useState(false);
  const [modelPickerOpen, setModelPickerOpen] = useState(false);
  const [providerId, setProviderId] = useState<string>("");
  const [modelId, setModelId] = useState<string>("");

  const providersQuery = useQuery({
    queryKey: ["providers"],
    enabled: open,
    queryFn: () => listProviders(conn),
  });

  const installedProviders = useMemo(() => {
    const providers = providersQuery.data ?? [];
    return providers
      .filter((p) => p.installed && p.health === "ok")
      .map((p) => p.provider_id)
      .filter(Boolean);
  }, [providersQuery.data]);

  const effectiveProviderId = useMemo(() => {
    const current = String(props.currentProviderId ?? "").trim();
    if (providerId) return providerId;
    if (current && installedProviders.includes(current)) return current;
    return installedProviders.includes("codex") ? "codex" : installedProviders[0] || "";
  }, [providerId, installedProviders, props.currentProviderId]);

  const optionsQuery = useQuery({
    queryKey: ["providerOptions", workspaceId, effectiveProviderId],
    enabled: open && Boolean(workspaceId && effectiveProviderId),
    queryFn: () => getProviderOptions(conn, workspaceId, effectiveProviderId),
  });

  const modelIds = useMemo(() => extractModelIds(optionsQuery.data?.models), [optionsQuery.data?.models]);

  const effectiveModelId = useMemo(() => {
    const current = String(props.currentModelId ?? "").trim();
    if (modelId) return modelId;
    if (current && modelIds.includes(current)) return current;
    return modelIds[0] || (effectiveProviderId ? "default" : "");
  }, [modelId, modelIds, props.currentModelId, effectiveProviderId]);

  useEffect(() => {
    if (!open) return;
    setProviderId("");
    setModelId("");
  }, [open]);

  const createMutation = useMutation({
    mutationFn: async () => {
      const prov = String(effectiveProviderId || "").trim();
      const model = String(effectiveModelId || "").trim();
      if (!prov) throw new Error("Select a harness first.");
      if (!model) throw new Error("Select a model first.");

      const session = await createSession(conn, taskId, prov, model, { env_target: "worktree" });
      const sessionId = idToString(session.id);
      if (!sessionId) throw new Error("Failed to create session.");
      return { sessionId, providerId: prov, modelId: model };
    },
    onSuccess: (result) => {
      props.onCreated(result);
      onClose();
    },
  });

  return (
    <Modal visible={open} transparent animationType="slide" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose} />
      <View style={styles.sheet}>
        <View style={styles.sheetHeader}>
          <Text style={styles.sheetTitle}>Session options</Text>
          <Pressable onPress={onClose}>
            <Text style={styles.sheetClose}>Close</Text>
          </Pressable>
        </View>

        <Text style={styles.hint}>
          Changing harness/model creates a new session, then switches you to it.
        </Text>

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

        {createMutation.error ? <Text style={styles.error}>{String(createMutation.error)}</Text> : null}

        <Pressable
          style={[styles.primaryButton, createMutation.isPending ? styles.primaryButtonDisabled : null]}
          onPress={() => createMutation.mutate()}
          disabled={createMutation.isPending}
        >
          <Text style={styles.primaryButtonText}>{createMutation.isPending ? "Creating…" : "Create new session"}</Text>
        </Pressable>

        <PickerModal
          open={providerPickerOpen}
          title="Select harness"
          items={installedProviders}
          selected={effectiveProviderId}
          loading={providersQuery.isLoading}
          onClose={() => setProviderPickerOpen(false)}
          onSelect={(id) => {
            setProviderId(id);
            setModelId("");
            setProviderPickerOpen(false);
          }}
        />

        <PickerModal
          open={modelPickerOpen}
          title="Select model"
          items={modelIds.length ? modelIds : ["default"]}
          selected={effectiveModelId}
          loading={optionsQuery.isLoading}
          onClose={() => setModelPickerOpen(false)}
          onSelect={(id) => {
            setModelId(id);
            setModelPickerOpen(false);
          }}
        />
      </View>
    </Modal>
  );
}

function PickerModal({
  open,
  title,
  items,
  selected,
  loading,
  onClose,
  onSelect,
}: {
  open: boolean;
  title: string;
  items: string[];
  selected: string;
  loading: boolean;
  onClose: () => void;
  onSelect: (id: string) => void;
}): React.JSX.Element {
  const theme = useContextTokens();
  return (
    <Modal visible={open} transparent animationType="fade" onRequestClose={onClose}>
      <Pressable style={styles.backdrop} onPress={onClose} />
      <View style={styles.modalCard}>
        <Text style={styles.modalTitle}>{title}</Text>
        {loading ? <Text style={styles.modalHint}>Loading…</Text> : null}
        {items.map((id) => {
          const active = id === selected;
          return (
            <Pressable key={id} style={styles.modalRow} onPress={() => onSelect(id)}>
              <Text style={styles.modalRowText} numberOfLines={1}>
                {id}
              </Text>
              {active ? <Text style={{ color: theme.colors.accent, fontWeight: "900" }}>✓</Text> : null}
            </Pressable>
          );
        })}
      </View>
    </Modal>
  );
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
  backdrop: { position: "absolute", left: 0, top: 0, right: 0, bottom: 0, backgroundColor: "rgba(0,0,0,0.6)" },
  sheet: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    borderTopLeftRadius: t.radii.lg,
    borderTopRightRadius: t.radii.lg,
    borderTopWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surface,
    padding: t.spacing.lg,
    gap: t.spacing.sm,
  },
  sheetHeader: { flexDirection: "row", alignItems: "center", justifyContent: "space-between" },
  sheetTitle: { color: t.colors.text, fontWeight: "900", fontSize: t.typography.sizes.lg },
  sheetClose: { color: t.colors.accent, fontWeight: "800" },
  hint: { color: t.colors.muted },
  picker: {
    padding: t.spacing.md,
    borderRadius: t.radii.lg,
    borderWidth: 1,
    borderColor: t.colors.border,
    backgroundColor: t.colors.surfaceAlt,
    gap: 6,
  },
  pickerLabel: { color: t.colors.muted, fontWeight: "700", textTransform: "uppercase", fontSize: t.typography.sizes.xs },
  pickerValueRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: t.spacing.sm },
  pickerValue: { flex: 1, color: t.colors.text, fontWeight: "800" },
  primaryButton: {
    marginTop: t.spacing.sm,
    backgroundColor: t.colors.accent,
    paddingVertical: t.spacing.md,
    borderRadius: t.radii.lg,
    alignItems: "center",
  },
  primaryButtonDisabled: { opacity: 0.7 },
  primaryButtonText: { color: "white", fontWeight: "900" },
  error: { color: t.colors.danger, fontWeight: "700" },
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
  modalHint: { color: t.colors.muted },
  modalRow: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingVertical: t.spacing.sm },
  modalRowText: { color: t.colors.text, fontWeight: "700", flex: 1 },
}));
