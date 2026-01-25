import { ArrowUp, ChevronDown, Copy, Image, Mic, MoreHorizontal, Sparkles } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { SafeAreaView, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import Markdown from "react-native-markdown-display";

import { Background } from "../components/Background";
import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type ChatMessage = {
  id: string;
  role: "assistant" | "user";
  text: string;
  status?: string;
  copyable?: boolean;
};

const sampleMessages: ChatMessage[] = [
  {
    id: "assistant-0",
    role: "assistant" as const,
    text: "Welcome to ctx. What would you like to build today?",
  },
  {
    id: "user-1",
    role: "user" as const,
    text:
      "A native chat screen with SwiftUI components.\nMake the user message bubble full-width and easy to copy...\n",
    copyable: true,
  },
  {
    id: "assistant-2",
    role: "assistant" as const,
    text: "Great. I will set up message bubbles and a composer that feels like ChatGPT.",
    status: "Completed · 42s",
  },
];

export function ChatScreen() {
  const [composer, setComposer] = useState("");
  const markdownStyle = useMemo(
    () => ({
      body: {
        color: colors.textPrimary,
        fontSize: 17,
        lineHeight: 22,
      },
      heading1: {
        color: colors.textPrimary,
        fontSize: 22,
        lineHeight: 28,
        fontWeight: "600",
      },
      text: {
        color: colors.textPrimary,
      },
      code_block: {
        backgroundColor: colors.surfaceRaised,
        borderColor: colors.line,
        borderWidth: 1,
        borderRadius: 12,
        padding: 12,
        fontFamily: "Menlo",
        fontSize: 14,
        color: colors.textPrimary,
      },
      fence: {
        backgroundColor: colors.surfaceRaised,
        borderColor: colors.line,
        borderWidth: 1,
        borderRadius: 12,
        padding: 12,
        fontFamily: "Menlo",
        fontSize: 14,
        color: colors.textPrimary,
      },
      code_inline: {
        backgroundColor: colors.surfaceRaised,
        borderRadius: 6,
        paddingHorizontal: 6,
        paddingVertical: 2,
        fontFamily: "Menlo",
        fontSize: 13,
        color: colors.textPrimary,
      },
      bullet_list: {
        marginTop: 8,
        marginBottom: 8,
      },
    }),
    []
  );

  return (
    <SafeAreaView style={styles.safeArea}>
      <Background />
      <View style={styles.container}>
        <ScrollView contentContainerStyle={styles.messages} showsVerticalScrollIndicator={false}>
          {sampleMessages.map((message) =>
            message.role === "user" ? (
              <View key={message.id} style={styles.userMessageRow}>
                <View style={styles.userBubble}>
                  <Text style={styles.userText}>{message.text}</Text>
                  {"copyable" in message && message.copyable ? (
                    <View style={styles.userCopyButton}>
                      <Copy size={14} color={colors.textMuted} />
                    </View>
                  ) : null}
                </View>
              </View>
            ) : (
              <View key={message.id} style={styles.assistantRow}>
                <Markdown style={markdownStyle}>{message.text}</Markdown>
                {"status" in message && message.status ? (
                  <View style={styles.statusRow}>
                    <Text style={styles.statusText}>{message.status}</Text>
                    <Text style={styles.statusText}>·</Text>
                    <Copy size={12} color={colors.textSecondary} />
                  </View>
                ) : null}
              </View>
            )
          )}
        </ScrollView>

        <View style={styles.composer}>
          <View style={styles.composerCard}>
            <View style={styles.composerInputWrap}>
              {composer.length === 0 ? (
                <Text style={styles.composerPlaceholder}>@ for context, / for commands</Text>
              ) : null}
              <TextInput
                value={composer}
                onChangeText={setComposer}
                style={styles.composerInput}
                multiline
              />
            </View>
            <View style={styles.composerActions}>
              <View style={styles.actionLeft}>
                <View style={styles.harnessIcon}>
                  <Sparkles size={14} color={colors.textSecondary} />
                </View>
                <View style={styles.modelPill}>
                  <Text style={styles.modelLabel}>default</Text>
                  <ChevronDown size={12} color={colors.textMuted} />
                </View>
              </View>
              <View style={styles.actionRight}>
                <View style={styles.iconButton}>
                  <MoreHorizontal size={20} color={colors.textSecondary} />
                </View>
                <View style={styles.iconButton}>
                  <Image size={20} color={colors.textSecondary} />
                </View>
                {composer.length > 0 ? (
                  <View style={styles.sendButton}>
                    <ArrowUp size={18} color="#ffffff" />
                  </View>
                ) : (
                  <View style={styles.iconButton}>
                    <Mic size={20} color={colors.textSecondary} />
                  </View>
                )}
              </View>
            </View>
          </View>
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
  messages: {
    paddingHorizontal: 16,
    paddingTop: spacing.messageTopPadding + 8,
    paddingBottom: spacing.messageBottomPadding,
    gap: spacing.messageSpacing,
  },
  assistantRow: {
    paddingVertical: 2,
  },
  userMessageRow: {
    alignItems: "stretch",
    width: "100%",
  },
  userBubble: {
    borderRadius: 12,
    backgroundColor: colors.bubbleUser,
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: 14,
    paddingVertical: 12,
    maxWidth: 960,
    width: "100%",
    position: "relative",
  },
  userText: {
    color: colors.textPrimary,
    fontSize: 17,
    lineHeight: 22,
    paddingRight: 26,
  },
  userCopyButton: {
    position: "absolute",
    top: 6,
    right: 6,
    width: 24,
    height: 24,
    borderRadius: 6,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "rgba(255, 255, 255, 0.05)",
  },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    marginTop: 6,
  },
  statusText: {
    color: colors.textSecondary,
    fontSize: 13,
    fontWeight: "700",
  },
  composer: {
    paddingHorizontal: 12,
    paddingBottom: 12,
  },
  composerCard: {
    borderRadius: 32,
    backgroundColor: colors.surfaceRaised,
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: spacing.composerInnerHorizontal,
    paddingVertical: 16,
    gap: 12,
  },
  composerInputWrap: {
    minHeight: 20,
  },
  composerPlaceholder: {
    position: "absolute",
    top: 2,
    left: 0,
    color: colors.textMuted,
    fontSize: 16,
  },
  composerInput: {
    color: colors.textPrimary,
    fontSize: 17,
    lineHeight: 22,
    minHeight: 48,
  },
  composerActions: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  actionLeft: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  actionRight: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  harnessIcon: {
    width: 20,
    height: 20,
    borderRadius: 8,
    backgroundColor: "rgba(255, 255, 255, 0.08)",
    alignItems: "center",
    justifyContent: "center",
  },
  modelPill: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    borderRadius: 12,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    paddingHorizontal: 10,
    paddingVertical: 7,
  },
  modelLabel: {
    color: colors.textPrimary,
    fontSize: 12,
    fontWeight: "600",
  },
  iconButton: {
    width: spacing.composerToolSize,
    height: spacing.composerToolSize,
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
});
