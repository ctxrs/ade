import { Copy, RefreshCw } from "lucide-react-native";
import React from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, View } from "react-native";

import { Background } from "../components/Background";
import { GlassPanel } from "../components/GlassPanel";
import { GlassPill } from "../components/GlassPill";
import { GhostButton } from "../components/GhostButton";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";
import { typography } from "../theme/typography";

const sampleJson = `{\n  \"daemonVersion\": \"0.6.1\",\n  \"lastSync\": \"2024-07-01T12:00:00Z\",\n  \"workspaces\": 3,\n  \"sessions\": 14\n}`;

const daemonRows = [
  { label: "Daemon URL", value: "http://127.0.0.1:4399" },
  { label: "Version", value: "0.6.1" },
  { label: "PID", value: "12345" },
  { label: "Data root", value: "/Users/example-user/.ctx" },
  { label: "Auth required", value: "Yes" },
];

const platformRows = [
  { label: "OS", value: "macOS 14.6" },
  { label: "Arch", value: "arm64" },
];

const providerRows = [
  { name: "Codex", status: "Ok", tint: colors.accent, version: "0.18.1", notes: ["ok"] },
];

const logRows = [
  { name: "daemon.log", detail: "120kb · modified 2h ago" },
  { name: "ios-client.log", detail: "42kb · modified 2h ago" },
];

type KeyValueRowProps = {
  label: string;
  value: string;
};

function KeyValueRow({ label, value }: KeyValueRowProps) {
  return (
    <View style={styles.row}>
      <Text style={styles.rowLabel}>{label}</Text>
      <Text style={styles.rowValue}>{value}</Text>
    </View>
  );
}

type ProviderRowProps = {
  name: string;
  status: string;
  tint: string;
  version: string;
  notes: string[];
};

function ProviderRow({ name, status, tint, version, notes }: ProviderRowProps) {
  return (
    <View style={styles.providerRow}>
      <View style={styles.providerHeader}>
        <Text style={styles.providerTitle}>{name}</Text>
        <GlassPill label={status} tint={tint} size="sm" />
      </View>
      <Text style={styles.providerMeta}>{version}</Text>
      {notes.map((note) => (
        <Text key={note} style={styles.providerNote}>
          {note}
        </Text>
      ))}
    </View>
  );
}

type LogRowProps = {
  name: string;
  detail: string;
};

function LogRow({ name, detail }: LogRowProps) {
  return (
    <View style={styles.providerRow}>
      <Text style={styles.providerTitle}>{name}</Text>
      <Text style={styles.providerMeta}>{detail}</Text>
    </View>
  );
}

export function DiagnosticsScreen() {
  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.container}
        showsVerticalScrollIndicator={false}
      >
        <Text style={styles.title}>Diagnostics</Text>

        <View style={styles.actionRow}>
          <GhostButton
            label="Refresh"
            icon={<RefreshCw size={14} color={colors.textPrimary} />}
          />
          <GhostButton
            label="Copy JSON"
            icon={<Copy size={14} color={colors.textPrimary} />}
          />
        </View>

        <GlassPanel style={styles.section}>
          <Text style={styles.sectionTitle}>Daemon</Text>
          <View style={styles.sectionBody}>
            {daemonRows.map((row) => (
              <KeyValueRow key={row.label} label={row.label} value={row.value} />
            ))}
          </View>
        </GlassPanel>

        <GlassPanel style={styles.section}>
          <Text style={styles.sectionTitle}>Platform</Text>
          <View style={styles.sectionBody}>
            {platformRows.map((row) => (
              <KeyValueRow key={row.label} label={row.label} value={row.value} />
            ))}
          </View>
        </GlassPanel>

        <GlassPanel style={styles.section}>
          <Text style={styles.sectionTitle}>Providers</Text>
          <View style={styles.sectionBody}>
            {providerRows.map((provider) => (
              <ProviderRow key={provider.name} {...provider} />
            ))}
          </View>
        </GlassPanel>

        <GlassPanel style={styles.section}>
          <Text style={styles.sectionTitle}>Logs</Text>
          <View style={styles.sectionBody}>
            {logRows.map((row) => (
              <LogRow key={row.name} {...row} />
            ))}
          </View>
        </GlassPanel>

        <GlassPanel style={styles.section}>
          <Text style={styles.sectionTitle}>Diagnostics JSON</Text>
          <View style={styles.codeContainer}>
            <ScrollView horizontal showsHorizontalScrollIndicator={false}>
              <Text style={styles.codeBlock}>{sampleJson}</Text>
            </ScrollView>
          </View>
        </GlassPanel>
      </ScrollView>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safeArea: {
    flex: 1,
    backgroundColor: colors.backgroundDeep,
  },
  container: {
    paddingHorizontal: spacing.screenPadding,
    paddingTop: 8,
    paddingBottom: 40,
    gap: 20,
    flexGrow: 1,
  },
  scroll: {
    flex: 1,
  },
  title: {
    color: colors.textPrimary,
    fontSize: typography.title,
    fontWeight: "600",
  },
  actionRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  section: {
    gap: 12,
  },
  sectionTitle: {
    color: colors.textPrimary,
    fontSize: 16,
    fontWeight: "600",
  },
  sectionBody: {
    gap: 12,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 14,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
  },
  rowLabel: {
    color: colors.textSecondary,
    fontSize: 15,
  },
  rowValue: {
    color: colors.textPrimary,
    fontSize: 15,
    textAlign: "right",
    flex: 1,
    marginLeft: 12,
    flexShrink: 1,
  },
  providerRow: {
    gap: 6,
    paddingHorizontal: 14,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
  },
  providerHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  providerTitle: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
  providerMeta: {
    color: colors.textMuted,
    fontSize: 12,
  },
  providerNote: {
    color: colors.textSecondary,
    fontSize: 12,
  },
  codeBlock: {
    color: colors.textSecondary,
    fontSize: 12,
    fontFamily: "Menlo",
    lineHeight: 18,
  },
  codeContainer: {
    backgroundColor: "rgba(37, 37, 38, 0.35)",
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    padding: 8,
    minHeight: 160,
  },
});
