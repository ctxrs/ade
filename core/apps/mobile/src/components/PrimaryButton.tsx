import { LinearGradient } from "expo-linear-gradient";
import React from "react";
import { Pressable, StyleProp, StyleSheet, Text, View, ViewStyle } from "react-native";

import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type PrimaryButtonProps = {
  label: string;
  onPress?: () => void;
  style?: StyleProp<ViewStyle>;
  icon?: React.ReactNode;
};

export function PrimaryButton({ label, onPress, style, icon }: PrimaryButtonProps) {
  return (
    <Pressable onPress={onPress} style={[styles.wrapper, style]}>
      {({ pressed }) => (
        <LinearGradient
          colors={[colors.accent, colors.accentMuted]}
          start={{ x: 0, y: 0 }}
          end={{ x: 1, y: 1 }}
          style={[styles.button, pressed ? styles.buttonPressed : null]}
        >
          <Text style={styles.label}>{label}</Text>
          {icon ? <View style={styles.iconSlot}>{icon}</View> : null}
        </LinearGradient>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    alignSelf: "stretch",
  },
  button: {
    borderRadius: spacing.buttonRadius,
    paddingVertical: 12,
    paddingHorizontal: 18,
    alignItems: "center",
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 10,
    borderWidth: 0.8,
    borderColor: colors.glassStroke,
    borderCurve: "continuous",
  },
  label: {
    color: colors.textPrimary,
    fontSize: 17,
    fontWeight: "600",
    flex: 1,
    textAlign: "left",
  },
  iconSlot: {
    alignItems: "center",
    justifyContent: "center",
  },
  buttonPressed: {
    transform: [{ scale: 0.98 }],
    opacity: 0.9,
  },
});
