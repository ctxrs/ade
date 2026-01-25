import { ChevronRight, Wifi } from "lucide-react-native";
import React from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, View } from "react-native";

import { Background } from "../components/Background";
import { GlassPanel } from "../components/GlassPanel";
import { GlassPill } from "../components/GlassPill";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";
import { typography } from "../theme/typography";

type RowProps = {
  title: string;
  subtitle?: string;
  pills?: { label: string; tint?: string }[];
};

function SettingsRow({ title, subtitle, pills }: RowProps) {
  return (
    <View style={styles.row}>
      <View style={styles.rowLeft}>
        <View style={styles.rowIcon}>
          <Wifi size={16} color={colors.accent} />
        </View>
        <View style={styles.rowText}>
          <Text style={styles.rowTitle}>{title}</Text>
          {subtitle ? <Text style={styles.rowSubtitle}>{subtitle}</Text> : null}
        </View>
      </View>
      <View style={styles.rowRight}>
        {pills?.map((pill) => (
          <GlassPill
            key={pill.label}
            label={pill.label}
            tint={pill.tint ?? colors.accent}
            size="sm"
          />
        ))}
        <ChevronRight size={12} color={colors.textSecondary} />
      </View>
    </View>
  );
}

export function SettingsScreen() {
  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView contentContainerStyle={styles.container} showsVerticalScrollIndicator={false}>
        <Text style={styles.navTitle}>Settings</Text>
        <Text style={styles.title}>Settings</Text>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Account</Text>
          <GlassPanel>
            <SettingsRow title="Account" subtitle={`jane@ctx.dev \u00b7 Pro`} />
          </GlassPanel>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Daemon</Text>
          <GlassPanel>
            <SettingsRow
              title="Daemon Settings"
              subtitle={`Enabled \u00b7 Livekit \u00b7 Configured \u00b7 gpt-4.1-mini \u00b7 Adaptive \u00b7 Healthy`}
            />
          </GlassPanel>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Mobile Access</Text>
          <GlassPanel>
            <SettingsRow
              title="Mobile Access"
              subtitle="Manage remote tunnel access"
              pills={[{ label: "Pro enabled" }, { label: "Enabled" }]}
            />
          </GlassPanel>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Notifications</Text>
          <GlassPanel>
            <SettingsRow title="Notifications" subtitle="Disabled" pills={[{ label: "Pro enabled" }]} />
          </GlassPanel>
        </View>

        <View style={styles.section}>
          <Text style={styles.sectionTitle}>Model Routing</Text>
          <GlassPanel>
            <SettingsRow title="Model Routing" subtitle="Default: GPT-5" />
          </GlassPanel>
        </View>
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
  },
  navTitle: {
    color: colors.textPrimary,
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
  },
  title: {
    color: colors.textPrimary,
    fontSize: typography.title,
    fontWeight: "600",
  },
  section: {
    gap: 12,
  },
  sectionTitle: {
    color: colors.textPrimary,
    fontSize: 16,
    fontWeight: "600",
  },
  row: {
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    borderRadius: 14,
    borderWidth: 1,
    borderColor: colors.line,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: 14,
    paddingHorizontal: 14,
  },
  rowLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    flex: 1,
  },
  rowIcon: {
    width: 18,
    alignItems: "center",
  },
  rowText: {
    flex: 1,
  },
  rowTitle: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
  rowSubtitle: {
    color: colors.textMuted,
    fontSize: 12,
    marginTop: 4,
  },
  rowRight: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
});
