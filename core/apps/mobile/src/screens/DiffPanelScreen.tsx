import { ChevronDown, GitBranch } from "lucide-react-native";
import React from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, View } from "react-native";

import { Background } from "../components/Background";
import { GlassPanel } from "../components/GlassPanel";
import { GlassPill } from "../components/GlassPill";
import { PillButton } from "../components/PillButton";
import { colors } from "../theme/colors";

const gitStatusSummary = [
  " M core/apps/ios-native/Sources/App/RootView.swift",
  " A core/apps/ios-native/Sources/App/UITestOverrides.swift",
].join("\n");

const diffFiles = [
  {
    path: "core/apps/ios-native/Sources/App/RootView.swift",
    added: 4,
    isNew: false,
    diffLines: [
      "diff --git a/core/apps/ios-native/Sources/App/RootView.swift b/core/apps/ios-native/Sources/App/RootView.swift",
      "index 2b4e9b4..a83e3d1 100644",
      "--- a/core/apps/ios-native/Sources/App/RootView.swift",
      "+++ b/core/apps/ios-native/Sources/App/RootView.swift",
      "@@ -10,6 +10,10 @@ struct RootView: View {",
      " @State private var didBootstrap = false",
      "",
      " var body: some View {",
      "+    let uiTestScreen = UITestOverrides.screen",
      "+    if let uiTestScreen {",
      "+        UITestScreenOverrideView(screen: uiTestScreen)",
      "+    }",
    ],
  },
  {
    path: "core/apps/ios-native/Sources/App/UITestOverrides.swift",
    added: 2,
    isNew: true,
    diffLines: [
      "diff --git a/core/apps/ios-native/Sources/App/UITestOverrides.swift b/core/apps/ios-native/Sources/App/UITestOverrides.swift",
      "new file mode 100644",
      "index 0000000..1d2f3a4",
      "--- /dev/null",
      "+++ b/core/apps/ios-native/Sources/App/UITestOverrides.swift",
      "@@ -0,0 +1,4 @@",
      "+import Foundation",
      "+enum UITestOverrides {}",
    ],
  },
];

function diffLineColor(line: string) {
  if (line.startsWith("+") && !line.startsWith("+++")) {
    return colors.diffAdded;
  }
  if (line.startsWith("-") && !line.startsWith("---")) {
    return colors.diffRemoved;
  }
  return colors.textPrimary;
}

function DiffTextBlock({ lines }: { lines: string[] }) {
  return (
    <View style={styles.diffBlock}>
      <Text style={styles.diffText}>
        {lines.map((line, index) => (
          <Text key={`${index}-${line}`} style={[styles.diffLine, { color: diffLineColor(line) }]}>
            {line}
            {index < lines.length - 1 ? "\n" : ""}
          </Text>
        ))}
      </Text>
    </View>
  );
}

export function DiffPanelScreen() {
  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView contentContainerStyle={styles.container} showsVerticalScrollIndicator={false}>
        <View style={styles.navRow}>
          <Text style={styles.navButton}>Done</Text>
        </View>
        <View style={styles.headerRow}>
          <View style={styles.headerLeft}>
            <GitBranch size={18} color={colors.accent} />
            <Text style={styles.headerTitle}>Diff</Text>
          </View>
          <PillButton label="Refresh" />
        </View>

        <GlassPanel style={styles.statusCardContent}>
          <View style={styles.statusHeader}>
            <Text style={styles.statusTitle}>Git Status</Text>
            <GlassPill label="2 files" tint={colors.accent} />
          </View>
          <Text style={styles.statusBody}>{gitStatusSummary}</Text>
        </GlassPanel>

        <GlassPanel>
          <View style={styles.fileList}>
            {diffFiles.map((file) => (
              <View key={file.path} style={styles.fileRow}>
                <View style={styles.fileRowContent}>
                  <ChevronDown size={14} color={colors.textSecondary} />
                  <Text style={styles.filePath} numberOfLines={1}>
                    {file.path}
                  </Text>
                  <View style={styles.changeBadge}>
                    {file.isNew ? <Text style={styles.changeText}>New</Text> : null}
                    <Text style={styles.changeText}>+{file.added}</Text>
                  </View>
                </View>
                <DiffTextBlock lines={file.diffLines} />
              </View>
            ))}
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
    paddingHorizontal: 16,
    paddingTop: 16,
    paddingBottom: 40,
    gap: 16,
  },
  navRow: {
    flexDirection: "row",
    justifyContent: "flex-end",
  },
  navButton: {
    color: colors.textPrimary,
    fontSize: 13,
    fontWeight: "600",
  },
  headerRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  headerLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  headerTitle: {
    color: colors.textPrimary,
    fontSize: 17,
    fontWeight: "600",
  },
  statusCardContent: {
    gap: 10,
  },
  statusHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    marginBottom: 10,
  },
  statusTitle: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
  statusBody: {
    color: colors.textSecondary,
    fontSize: 12,
    lineHeight: 18,
    fontFamily: "Menlo",
  },
  fileList: {
    gap: 12,
  },
  fileRow: {
    gap: 10,
    paddingHorizontal: 12,
    paddingVertical: 12,
    borderRadius: 16,
    borderWidth: 0.8,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.5)",
    borderCurve: "continuous",
  },
  fileRowContent: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    flex: 1,
  },
  filePath: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
    flex: 1,
    marginRight: 10,
  },
  changeBadge: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    marginLeft: "auto",
  },
  changeText: {
    color: colors.diffAdded,
    fontSize: 11,
    fontWeight: "600",
  },
  diffText: {
    fontFamily: "Menlo",
    fontSize: 12,
    lineHeight: 16,
    includeFontPadding: false,
  },
  diffLine: {
    color: colors.textPrimary,
  },
  diffBlock: {
    marginTop: 0,
  },
});
