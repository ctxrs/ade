import { BlurView } from "expo-blur";
import React from "react";
import { StyleSheet, Text } from "react-native";

import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type GlassPillProps = {
  label: string;
  tint?: string;
  size?: "sm" | "md";
};

export function GlassPill({ label, tint = colors.accent, size = "md" }: GlassPillProps) {
  const borderTint = withAlpha(tint, 0.5);
  const pillStyle = size === "sm" ? styles.pillSmall : styles.pill;
  const labelStyle = size === "sm" ? styles.labelSmall : styles.label;
  return (
    <BlurView
      intensity={50}
      tint="systemUltraThinMaterialDark"
      style={[pillStyle, { borderColor: borderTint }]}
    >
      <Text style={[labelStyle, { color: tint }]}>{label}</Text>
    </BlurView>
  );
}

function withAlpha(color: string, opacity: number) {
  if (color.startsWith("rgba")) {
    return color;
  }
  if (color.startsWith("rgb")) {
    const [r, g, b] = color.replace(/[^\d,]/g, "").split(",").map(Number);
    return `rgba(${r}, ${g}, ${b}, ${opacity})`;
  }
  if (color.startsWith("#")) {
    const value = color.slice(1);
    const hex = value.length === 3 ? value.split("").map((c) => c + c).join("") : value;
    const bigint = parseInt(hex, 16);
    const r = (bigint >> 16) & 255;
    const g = (bigint >> 8) & 255;
    const b = bigint & 255;
    return `rgba(${r}, ${g}, ${b}, ${opacity})`;
  }
  return color;
}

const styles = StyleSheet.create({
  pill: {
    borderRadius: spacing.pillRadius,
    paddingHorizontal: 10,
    paddingVertical: 6,
    borderWidth: 0.8,
    overflow: "hidden",
  },
  pillSmall: {
    borderRadius: spacing.pillRadius,
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderWidth: 0.8,
    overflow: "hidden",
  },
  label: {
    fontSize: 12,
    fontWeight: "600",
  },
  labelSmall: {
    fontSize: 11,
    fontWeight: "600",
  },
});
