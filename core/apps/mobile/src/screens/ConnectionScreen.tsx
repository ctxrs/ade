import { CircleArrowRight, QrCode, Server } from "lucide-react-native";
import React, { useState } from "react";
import { Platform, SafeAreaView, ScrollView, StyleSheet, Switch, Text, View } from "react-native";

import { Background } from "../components/Background";
import { Field } from "../components/Field";
import { GlassPanel } from "../components/GlassPanel";
import { GlassPill } from "../components/GlassPill";
import { GhostButton } from "../components/GhostButton";
import { PrimaryButton } from "../components/PrimaryButton";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";
import { typography } from "../theme/typography";

export function ConnectionScreen() {
  const [daemonURL, setDaemonURL] = useState("http://127.0.0.1:4410");
  const [accessToken, setAccessToken] = useState("");
  const [rememberDevice, setRememberDevice] = useState(true);

  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView contentContainerStyle={styles.container} showsVerticalScrollIndicator={false}>
        <View style={styles.heroBlock}>
          <Text style={styles.title}>ctx</Text>
          <Text style={styles.subtitle}>Connect to your daemon to start work.</Text>
          <Text style={styles.body}>Secure, local-first sessions with native streaming and diagnostics.</Text>
        </View>

        <GlassPanel style={styles.panel}>
          <View style={styles.panelHeader}>
            <Text style={styles.panelTitle}>Connection</Text>
            <GlassPill label="Local network" tint={colors.accent} />
          </View>

          <Field
            label="Daemon URL"
            placeholder="https://your-daemon.local"
            value={daemonURL}
            onChangeText={setDaemonURL}
            keyboardType="url"
          />
          <Field
            label="Access Token"
            placeholder="Paste token"
            value={accessToken}
            onChangeText={setAccessToken}
            secureTextEntry
            textContentType="password"
          />

          <View style={styles.toggleRow}>
            <Text style={styles.toggleLabel}>Remember this device</Text>
            <Switch
              value={rememberDevice}
              onValueChange={setRememberDevice}
              trackColor={{ false: colors.line, true: colors.accent }}
              thumbColor={colors.textPrimary}
              style={styles.toggleControl}
            />
          </View>

          <PrimaryButton
            label="Connect to ctx"
            icon={<CircleArrowRight size={18} color={colors.textPrimary} />}
          />

          <GhostButton
            label="Scan QR instead"
            icon={<QrCode size={16} color={colors.textPrimary} />}
            style={[styles.secondaryButton, styles.fullWidthButton]}
          />
        </GlassPanel>

        <GlassPanel style={styles.recentPanelContent}>
          <View style={styles.recentHeader}>
            <Text style={styles.sectionTitle}>Recent connections</Text>
            <Text style={styles.sectionAction}>Clear</Text>
          </View>
          <View style={styles.recentList}>
            <View style={styles.recentRow}>
              <View style={styles.recentIcon}>
                <Server size={16} color={colors.textSecondary} />
              </View>
              <View style={styles.recentInfo}>
                <Text style={styles.recentTitle}>127.0.0.1</Text>
                <Text style={styles.recentSubtitle}>http://127.0.0.1:4410 \u00b7 Token 3ae9e7...</Text>
              </View>
              <GlassPill label="6 hr. ago" tint={colors.accent} />
            </View>
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
    paddingHorizontal: spacing.screenPaddingWide,
    paddingTop: 28,
    paddingBottom: 40,
    gap: 24,
  },
  heroBlock: {
    gap: 10,
  },
  title: {
    color: colors.textPrimary,
    fontSize: typography.heroTitle,
    fontWeight: "600",
    fontFamily: Platform.select({ ios: "SF Pro Rounded" }),
  },
  subtitle: {
    color: colors.textSecondary,
    fontSize: typography.subtitle,
    fontWeight: "500",
    lineHeight: 24,
  },
  body: {
    color: colors.textMuted,
    fontSize: 15,
    lineHeight: 20,
  },
  panel: {
    gap: 18,
  },
  panelHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  panelTitle: {
    color: colors.textPrimary,
    fontSize: 17,
    fontWeight: "600",
    lineHeight: 22,
  },
  toggleRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingVertical: 2,
  },
  toggleLabel: {
    color: colors.textSecondary,
    fontSize: 17,
    lineHeight: 22,
  },
  toggleControl: {
    transform: [{ translateY: -1 }],
  },
  secondaryButton: {
    marginTop: 2,
  },
  fullWidthButton: {
    alignSelf: "stretch",
  },
  sectionTitle: {
    color: colors.textPrimary,
    fontSize: 17,
    fontWeight: "600",
    lineHeight: 22,
  },
  sectionAction: {
    color: colors.textSecondary,
    fontSize: typography.caption,
    fontWeight: "600",
    lineHeight: 16,
  },
  recentPanelContent: {
    gap: 12,
  },
  recentHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  recentList: {
    gap: 10,
  },
  recentRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 12,
    paddingVertical: 12,
    borderRadius: 16,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
  },
  recentIcon: {
    width: 36,
    height: 36,
    borderRadius: 18,
    backgroundColor: colors.surfaceRaised,
    alignItems: "center",
    justifyContent: "center",
  },
  recentInfo: {
    flex: 1,
    gap: 4,
  },
  recentTitle: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
    lineHeight: 20,
  },
  recentSubtitle: {
    color: colors.textMuted,
    fontSize: 12,
    lineHeight: 16,
  },
});
