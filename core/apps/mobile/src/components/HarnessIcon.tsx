import React from "react";
import { Image, StyleSheet, View } from "react-native";

import { colors } from "../theme/colors";

const harnessSources: Record<string, number> = {
  aider: require("../../assets/harness/harness_aider.png"),
  amp: require("../../assets/harness/harness_amp.png"),
  auggie: require("../../assets/harness/harness_auggie.png"),
  cagent: require("../../assets/harness/harness_cagent.png"),
  charm: require("../../assets/harness/harness_charm.png"),
  codex: require("../../assets/harness/harness_codex.png"),
  codebuff: require("../../assets/harness/harness_codebuff.png"),
  cody: require("../../assets/harness/harness_cody.png"),
  claude: require("../../assets/harness/harness_claude.png"),
  cline: require("../../assets/harness/harness_cline.png"),
  continue: require("../../assets/harness/harness_continue.png"),
  copilot: require("../../assets/harness/harness_copilot.png"),
  cursor: require("../../assets/harness/harness_cursor.png"),
  droid: require("../../assets/harness/harness_droid.png"),
  gemini: require("../../assets/harness/harness_gemini.png"),
  goose: require("../../assets/harness/harness_goose.png"),
  junie: require("../../assets/harness/harness_junie.png"),
  kilo: require("../../assets/harness/harness_kilo.png"),
  kimi: require("../../assets/harness/harness_kimi.png"),
  kiro: require("../../assets/harness/harness_kiro.png"),
  mistral: require("../../assets/harness/harness_mistral.png"),
  opencode: require("../../assets/harness/harness_opencode.png"),
  openhands: require("../../assets/harness/harness_openhands.png"),
  qwen: require("../../assets/harness/harness_qwen.png"),
  rovo: require("../../assets/harness/harness_rovo.png"),
  "swe-agent": require("../../assets/harness/harness_swe-agent.png"),
};

const invertedInDark = new Set(["auggie", "codex", "copilot", "cursor", "droid", "opencode"]);

type HarnessIconProps = {
  providerId?: string;
  size?: number;
  radius?: number;
  backgroundOpacity?: number;
};

export function HarnessIcon({
  providerId,
  size = 16,
  radius = 6,
  backgroundOpacity = 0.04,
}: HarnessIconProps) {
  const source = providerId ? harnessSources[providerId] : undefined;
  const tint = providerId && invertedInDark.has(providerId) ? colors.textPrimary : undefined;
  const containerStyle = {
    width: size,
    height: size,
    borderRadius: radius,
    backgroundColor: `rgba(255, 255, 255, ${backgroundOpacity})`,
  };

  return (
    <View style={[styles.container, containerStyle]}>
      {source ? (
        <Image source={source} style={[styles.image, { tintColor: tint }]} />
      ) : (
        <View style={[styles.fallback, { borderRadius: radius }]} />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    alignItems: "center",
    justifyContent: "center",
  },
  image: {
    width: "100%",
    height: "100%",
    resizeMode: "contain",
  },
  fallback: {
    width: "100%",
    height: "100%",
    backgroundColor: "rgba(255, 255, 255, 0.1)",
    borderWidth: 1,
    borderColor: "rgba(18, 18, 18, 0.8)",
  },
});
