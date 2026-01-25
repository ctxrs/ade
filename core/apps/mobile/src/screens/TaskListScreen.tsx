import { ChevronDown, ChevronRight, Pencil, Search, Settings } from "lucide-react-native";
import React from "react";
import { ActivityIndicator, SafeAreaView, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";

import { Background } from "../components/Background";
import { TaskHarnessStack } from "../components/TaskHarnessStack";
import { colors } from "../theme/colors";

const tasks = [
  {
    title: "Polish drawer list",
    subtitle: "Refining the workbench drawer spacing.",
    time: "570d",
    progress: true,
    providerIds: ["codex"],
  },
  {
    title: "Fix diff rendering",
    subtitle: "Diff parser crashed on renamed file.",
    time: "570d",
    dot: colors.error,
    providerIds: ["claude"],
  },
  {
    title: "Sync settings copy",
    subtitle: "Settings copy updated and approved.",
    time: "570d",
    dot: colors.accent,
    providerIds: ["codex"],
  },
];

const archivedCount = 1;

export function TaskListScreen() {
  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <View style={styles.container}>
        <View style={styles.searchRow}>
          <View style={styles.searchField}>
            <Search size={17} color={colors.textSecondary} />
            <TextInput
              placeholder="Search Tasks"
              placeholderTextColor={colors.textSecondary}
              style={styles.searchInput}
            />
          </View>
          <View style={styles.composeButton}>
            <Pencil size={17} color={colors.textPrimary} />
          </View>
        </View>
        <View style={styles.divider} />

        <ScrollView contentContainerStyle={styles.scrollContent} showsVerticalScrollIndicator={false}>
          <View style={styles.listSection}>
            <View style={styles.sectionHeader}>
              <Text style={styles.sectionTitle}>ACTIVE</Text>
            </View>

            <View style={styles.list}>
              {tasks.map((task) => (
                <View
                  key={`${task.title}-${task.time}`}
                  style={[styles.row, task.progress ? styles.rowActive : null]}
                >
                  <TaskHarnessStack providerIds={task.providerIds} />
                  <View style={styles.rowBody}>
                    <Text style={styles.rowTitle} numberOfLines={1}>
                      {task.title}
                    </Text>
                    {task.subtitle ? (
                      <Text style={styles.rowSubtitle} numberOfLines={1}>
                        {task.subtitle}
                      </Text>
                    ) : null}
                  </View>
                  <View style={styles.rowMeta}>
                    <Text style={styles.rowTime}>{task.time}</Text>
                    {task.progress ? (
                      <ActivityIndicator size="small" color={colors.accent} style={styles.rowSpinner} />
                    ) : null}
                    {task.dot ? <View style={[styles.statusDot, { backgroundColor: task.dot }]} /> : null}
                  </View>
                </View>
              ))}
            </View>

            <View style={styles.archivedRow}>
              <Text style={styles.sectionTitle}>ARCHIVED</Text>
              <View style={styles.archivedMeta}>
                <Text style={styles.archivedCount}>{archivedCount}</Text>
                <ChevronDown size={12} color={colors.textSecondary} style={styles.archivedChevron} />
              </View>
            </View>
          </View>
        </ScrollView>

        <View style={styles.divider} />
        <View style={styles.workspaceRow}>
          <View style={styles.workspaceInner}>
            <Settings size={16} color={colors.accent} />
            <Text style={styles.workspaceLabel}>Atlas</Text>
          </View>
          <ChevronRight size={12} color={colors.textSecondary} />
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
  },
  searchRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingHorizontal: 16,
    paddingTop: 16,
    paddingBottom: 12,
  },
  scrollContent: {
    padding: 16,
    gap: 16,
  },
  searchField: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 10,
    backgroundColor: "rgba(255, 255, 255, 0.04)",
    borderWidth: 1,
    borderColor: colors.line,
  },
  searchInput: {
    color: colors.textPrimary,
    flex: 1,
    fontSize: 13,
  },
  composeButton: {
    width: 40,
    height: 40,
    borderRadius: 12,
    backgroundColor: "rgba(255, 255, 255, 0.04)",
    borderWidth: 1,
    borderColor: colors.line,
    alignItems: "center",
    justifyContent: "center",
  },
  sectionHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  sectionTitle: {
    color: colors.textMuted,
    fontSize: 12,
    letterSpacing: 0.6,
    fontWeight: "600",
  },
  listSection: {
    gap: 16,
  },
  list: {
    gap: 2,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingVertical: 6,
    paddingHorizontal: 8,
    borderRadius: 10,
  },
  rowActive: {
    backgroundColor: "rgba(255, 255, 255, 0.06)",
    borderWidth: 1,
    borderColor: colors.line,
  },
  rowBody: {
    flex: 1,
  },
  rowTitle: {
    color: colors.textPrimary,
    fontSize: 12.5,
    fontWeight: "400",
  },
  rowSubtitle: {
    color: colors.textMuted,
    fontSize: 11,
    marginTop: 4,
  },
  rowMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  rowSpinner: {
    transform: [{ scale: 0.7 }],
  },
  statusDot: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  rowTime: {
    color: colors.textSecondary,
    fontSize: 12,
    textAlign: "right",
    minWidth: 28,
  },
  divider: {
    height: StyleSheet.hairlineWidth,
    backgroundColor: "rgba(255, 255, 255, 0.08)",
  },
  archivedRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  archivedMeta: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  archivedCount: {
    color: colors.textSecondary,
    fontSize: 12,
  },
  archivedChevron: {
    transform: [{ rotate: "-90deg" }],
  },
  workspaceRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 16,
    paddingVertical: 12,
  },
  workspaceInner: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  workspaceLabel: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
});
