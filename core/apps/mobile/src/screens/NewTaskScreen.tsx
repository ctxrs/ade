import { ArrowUp, ChevronDown, Image, Mic, Sparkles } from "lucide-react-native";
import React, { useState } from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";

import { Background } from "../components/Background";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type MenuPillProps = {
  label: string;
  icon?: React.ReactNode;
};

function MenuPill({ label, icon }: MenuPillProps) {
  return (
    <View style={styles.menuPill}>
      {icon ? <View style={styles.menuIcon}>{icon}</View> : null}
      <Text style={styles.menuLabel} numberOfLines={1}>
        {label}
      </Text>
      <ChevronDown size={12} color={colors.textMuted} />
    </View>
  );
}

type AttachmentPillProps = {
  label: string;
};

function AttachmentPill({ label }: AttachmentPillProps) {
  return (
    <View style={styles.attachmentPill}>
      <Image size={14} color={colors.textSecondary} />
      <Text style={styles.attachmentLabel} numberOfLines={1}>
        {label}
      </Text>
    </View>
  );
}

export function NewTaskScreen() {
  const [prompt, setPrompt] = useState("");
  const [promptHeight, setPromptHeight] = useState(20);
  const attachments: string[] = [];
  const shouldAutoFocus = process.env.EXPO_PUBLIC_SCREEN === "new-task";

  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <ScrollView contentContainerStyle={styles.container} showsVerticalScrollIndicator={false}>
        <View style={styles.card}>
          <View style={styles.promptArea}>
            {prompt.length === 0 ? (
              <Text style={styles.placeholder}>@ for context, / for commands</Text>
            ) : null}
            <TextInput
              value={prompt}
              onChangeText={setPrompt}
              autoFocus={shouldAutoFocus}
              style={[styles.promptInput, { height: promptHeight }]}
              multiline
              textAlignVertical="top"
              onContentSizeChange={(event) => {
                const next = Math.min(120, Math.max(20, event.nativeEvent.contentSize.height));
                setPromptHeight(next);
              }}
            />
          </View>

          {attachments.length > 0 ? (
            <View style={styles.attachmentsRow}>
              {attachments.map((attachment) => (
                <AttachmentPill key={attachment} label={attachment} />
              ))}
            </View>
          ) : null}

          <View style={styles.menuStack}>
            <View style={styles.harnessRow}>
              <View style={styles.harnessIcon}>
                <Sparkles size={14} color={colors.textSecondary} />
              </View>
              <View style={styles.menuSpacer} />
              <ChevronDown size={12} color={colors.textMuted} />
            </View>
            <View style={styles.menuRow}>
              <MenuPill label="gpt-5.2-codex" />
              <View style={styles.menuSpacer} />
            </View>
            <View style={styles.menuRow}>
              <MenuPill label="Default" />
              <View style={styles.menuSpacer} />
            </View>
          </View>

          <View style={styles.actionRow}>
            <View style={styles.toolsRow}>
              <View style={styles.toolButton}>
                <Image size={20} color={colors.textSecondary} />
              </View>
            </View>
            {prompt.trim().length > 0 ? (
              <View style={styles.sendButton}>
                <ArrowUp size={18} color="#ffffff" />
              </View>
            ) : (
              <View style={styles.micButton}>
                <Mic size={20} color={colors.textPrimary} />
              </View>
            )}
          </View>
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
    paddingTop: 16,
    paddingBottom: 20,
  },
  card: {
    borderRadius: spacing.composerRadius,
    backgroundColor: colors.surfaceRaised,
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: spacing.composerInnerHorizontal,
    paddingVertical: spacing.composerInnerVertical,
    gap: 12,
  },
  promptArea: {
    minHeight: 20,
  },
  placeholder: {
    position: "absolute",
    top: 2,
    left: 0,
    color: colors.textMuted,
    fontSize: 16,
  },
  promptInput: {
    minHeight: 20,
    maxHeight: 120,
    color: colors.textPrimary,
    fontSize: 16,
    lineHeight: 20,
  },
  attachmentsRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  attachmentPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: colors.surface,
    paddingHorizontal: 10,
    paddingVertical: 6,
    maxWidth: 160,
  },
  attachmentLabel: {
    color: colors.textSecondary,
    fontSize: 12,
  },
  menuStack: {
    gap: 8,
  },
  harnessRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
    paddingVertical: 6,
  },
  harnessIcon: {
    width: 20,
    height: 20,
    borderRadius: 8,
    backgroundColor: "rgba(255, 255, 255, 0.08)",
    alignItems: "center",
    justifyContent: "center",
  },
  menuRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  menuSpacer: {
    flex: 1,
  },
  menuPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    paddingHorizontal: 10,
    paddingVertical: 7,
    maxWidth: 180,
  },
  menuIcon: {
    width: 16,
    alignItems: "center",
  },
  menuLabel: {
    color: colors.textPrimary,
    fontSize: 13,
    fontWeight: "600",
  },
  actionRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  toolsRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  toolButton: {
    width: spacing.composerToolSize,
    height: spacing.composerToolSize,
    borderRadius: spacing.composerToolSize / 2,
    alignItems: "center",
    justifyContent: "center",
  },
  sendButton: {
    width: spacing.composerPrimarySize,
    height: spacing.composerPrimarySize,
    borderRadius: spacing.composerPrimarySize / 2,
    backgroundColor: colors.accent,
    alignItems: "center",
    justifyContent: "center",
  },
  micButton: {
    width: spacing.composerPrimarySize,
    height: spacing.composerPrimarySize,
    alignItems: "center",
    justifyContent: "center",
  },
});
