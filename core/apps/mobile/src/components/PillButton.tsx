import React from "react";
import { Pressable, StyleProp, StyleSheet, Text, View, ViewStyle } from "react-native";

import { colors } from "../theme/colors";

type PillButtonProps = {
  label: string;
  onPress?: () => void;
  style?: StyleProp<ViewStyle>;
};

export function PillButton({ label, onPress, style }: PillButtonProps) {
  return (
    <Pressable onPress={onPress} style={style}>
      {({ pressed }) => (
        <View style={[styles.button, pressed ? styles.buttonPressed : null]}>
          <Text style={styles.label}>{label}</Text>
        </View>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    borderRadius: 10,
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderWidth: 1,
    borderColor: colors.line,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
    borderCurve: "continuous",
    alignItems: "center",
    justifyContent: "center",
  },
  label: {
    color: colors.textPrimary,
    fontSize: 11,
    lineHeight: 13,
    fontWeight: "600",
  },
  buttonPressed: {
    transform: [{ scale: 0.98 }],
    opacity: 0.9,
  },
});
