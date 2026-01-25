import { Folder, Search, X } from "lucide-react-native";
import React from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";

import { Background } from "../components/Background";
import { GlassPill } from "../components/GlassPill";
import { GhostButton } from "../components/GhostButton";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";
import { typography } from "../theme/typography";

const workspaces = [
  { name: "Atlas", detail: "ios-native-ui", status: "Connected" },
  { name: "Context Monorepo", detail: "workbench-shell", status: "Idle" },
  { name: "Remote Devbox", detail: "ctx-linux", status: "Sleeping" },
  { name: "Docs Sandbox", detail: "ctx-docs", status: "Active" },
];

export function WorkspaceSelectorScreen() {
  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <View style={styles.container}>
        <View style={styles.topBar}>
          <View style={styles.closeButton}>
            <X size={16} color={colors.textSecondary} />
          </View>
        </View>

        <Text style={styles.title}>Select workspace</Text>

        <View style={styles.searchField}>
          <Search size={14} color={colors.textSecondary} />
          <TextInput
            placeholder="Search workspaces"
            placeholderTextColor={colors.textSecondary}
            style={styles.searchInput}
            autoCapitalize="none"
            autoCorrect={false}
          />
        </View>

        <View style={styles.listArea}>
          <ScrollView
            contentContainerStyle={styles.scrollContent}
            showsVerticalScrollIndicator={false}
            style={styles.scroll}
          >
            <View style={styles.list}>
              {workspaces.map((workspace, index) => (
                <View
                  key={workspace.name}
                  style={[styles.card, index === 0 ? styles.cardSelected : null]}
                >
                  <View style={styles.cardRow}>
                    <View style={styles.cardIcon}>
                      <Folder size={18} color={colors.accent} />
                    </View>
                    <View style={styles.cardText}>
                      <Text style={styles.cardTitle}>{workspace.name}</Text>
                      <Text style={styles.cardSubtitle}>{workspace.detail}</Text>
                    </View>
                    <GlassPill
                      label={workspace.status}
                      tint={workspace.status === "Connected" ? colors.accent : colors.textSecondary}
                    />
                  </View>
                </View>
              ))}
            </View>
          </ScrollView>
          <GhostButton label="Cancel" style={styles.cancelButton} />
        </View>
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  safeArea: {
    flex: 1,
    backgroundColor: colors.backgroundDeep,
  },
  container: {
    flex: 1,
    paddingHorizontal: spacing.screenPadding,
    paddingBottom: 20,
    gap: 18,
  },
  topBar: {
    flexDirection: "row",
    paddingTop: 0,
    alignItems: "center",
    justifyContent: "flex-end",
  },
  closeButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  title: {
    color: colors.textPrimary,
    fontSize: typography.title,
    fontWeight: "600",
    marginTop: 0,
  },
  searchField: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    borderRadius: 14,
    paddingHorizontal: 12,
    paddingVertical: 12,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    borderWidth: 1,
    borderColor: colors.line,
  },
  searchInput: {
    color: colors.textPrimary,
    fontSize: 16,
    flex: 1,
  },
  listArea: {
    flex: 1,
    gap: 18,
  },
  list: {
    gap: 12,
  },
  scroll: {
    flex: 1,
  },
  scrollContent: {
    flexGrow: 1,
    gap: 18,
  },
  cancelButton: {
    marginTop: 18,
  },
  card: {
    padding: 14,
    borderRadius: 18,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
  },
  cardSelected: {
    borderColor: "rgba(55, 148, 255, 0.6)",
    borderWidth: 1.2,
    backgroundColor: "rgba(37, 37, 38, 0.8)",
  },
  cardRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  cardIcon: {
    width: 44,
    height: 44,
    borderRadius: 12,
    backgroundColor: colors.surfaceRaised,
    alignItems: "center",
    justifyContent: "center",
  },
  cardText: {
    flex: 1,
    gap: 4,
  },
  cardTitle: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
  cardSubtitle: {
    color: colors.textMuted,
    fontSize: 12,
  },
});
