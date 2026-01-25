import { Copy, QrCode, RefreshCw, XCircle, Zap } from "lucide-react-native";
import React, { useState } from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, View } from "react-native";

import { Background } from "../components/Background";
import { Field } from "../components/Field";
import { GlassPanel } from "../components/GlassPanel";
import { GlassPill } from "../components/GlassPill";
import { GhostButton } from "../components/GhostButton";
import { PrimaryButton } from "../components/PrimaryButton";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";
import { typography } from "../theme/typography";

type StatusValueRowProps = {
  label: string;
  value: string;
};

function StatusValueRow({ label, value }: StatusValueRowProps) {
  return (
    <View style={styles.statusRow}>
      <Text style={styles.statusLabel}>{label}</Text>
      <Text style={styles.statusValue} numberOfLines={1} ellipsizeMode="middle">
        {value}
      </Text>
    </View>
  );
}

type StatusPillRowProps = {
  label: string;
  value: string;
  tint?: string;
};

function StatusPillRow({ label, value, tint = colors.accent }: StatusPillRowProps) {
  return (
    <View style={styles.statusRow}>
      <Text style={styles.statusLabel}>{label}</Text>
      <GlassPill label={value} tint={tint} size="sm" />
    </View>
  );
}

export function MobileAccessScreen() {
  const [supabaseToken, setSupabaseToken] = useState("");
  const accessEnabled = true;

  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.container}
        showsVerticalScrollIndicator={false}
      >
        <Text style={styles.title}>Mobile Access</Text>
        <Text style={styles.subtitle}>Manage remote tunnel access and pairing for this daemon.</Text>

        <GlassPanel style={styles.statusPanel}>
          <View style={styles.statusHeader}>
            <Text style={styles.sectionTitle}>Status</Text>
          </View>
          <View style={styles.statusList}>
            <StatusPillRow label="Entitlement" value="Pro enabled" tint={colors.accent} />
            <StatusPillRow
              label="Mobile access"
              value={accessEnabled ? "Enabled" : "Disabled"}
              tint={accessEnabled ? colors.accent : colors.textMuted}
            />
            <StatusPillRow label="Tunnel" value="Running" tint={colors.accent} />
            <StatusValueRow label="Public URL" value="https://atlas.ctx.dev" />
            <StatusValueRow label="Tunnel ID" value="tnl_8f3a21" />
          </View>
          <GhostButton
            label="Refresh status"
            icon={<RefreshCw size={14} color={colors.textPrimary} />}
            style={styles.refreshButton}
          />
        </GlassPanel>

        <GlassPanel style={styles.managePanel}>
          <Text style={styles.sectionTitle}>Manage Access</Text>
          <Field
            label="Supabase token"
            placeholder="Paste token"
            value={supabaseToken}
            onChangeText={setSupabaseToken}
            secureTextEntry
            textContentType="password"
          />
          <Text style={styles.helperText}>Required to enable or disable managed mobile access.</Text>
          {accessEnabled ? (
            <GhostButton
              label="Disable Mobile Access"
              trailingIcon={<XCircle size={16} color={colors.textPrimary} />}
              style={styles.fullWidth}
            />
          ) : (
            <PrimaryButton
              label="Enable Mobile Access"
              icon={<Zap size={16} color={colors.textPrimary} />}
            />
          )}
        </GlassPanel>

        <GlassPanel style={styles.qrPanel}>
          <Text style={styles.sectionTitle}>Pairing Window</Text>
          <StatusValueRow label="Expires" value="in 4m 12s" />
          <View style={styles.qrRow}>
            <View style={styles.qrBox}>
              <QrCode size={120} color={colors.textSecondary} />
            </View>
            <Text style={styles.qrHelp}>
              Scan this QR code with another ctx mobile device to pair securely.
            </Text>
          </View>
          <GhostButton
            label="Copy pairing payload"
            icon={<Copy size={14} color={colors.textPrimary} />}
            style={styles.fullWidth}
          />
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
  subtitle: {
    color: colors.textMuted,
    fontSize: 15,
  },
  sectionTitle: {
    color: colors.textPrimary,
    fontSize: 16,
    fontWeight: "600",
  },
  statusPanel: {
    gap: 12,
  },
  statusHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  statusList: {
    gap: 12,
  },
  statusRow: {
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
  statusLabel: {
    color: colors.textSecondary,
    fontSize: 15,
    flex: 1,
    marginRight: 12,
  },
  statusValue: {
    color: colors.textPrimary,
    fontSize: 15,
    textAlign: "right",
    flex: 1,
    marginLeft: 12,
  },
  managePanel: {
    gap: 12,
  },
  helperText: {
    color: colors.textMuted,
    fontSize: 12,
  },
  refreshButton: {
    alignSelf: "flex-start",
  },
  fullWidth: {
    alignSelf: "stretch",
  },
  qrPanel: {
    gap: 12,
  },
  qrRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  qrBox: {
    width: 180,
    height: 180,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    alignItems: "center",
    justifyContent: "center",
  },
  qrHelp: {
    color: colors.textMuted,
    fontSize: 12,
    flex: 1,
  },
});
