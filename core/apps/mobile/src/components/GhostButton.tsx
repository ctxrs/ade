import { BlurView } from "expo-blur";
import React from "react";
import { Pressable, StyleProp, StyleSheet, Text, View, ViewStyle } from "react-native";

import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type GhostButtonProps = {
  label: string;
  onPress?: () => void;
  style?: StyleProp<ViewStyle>;
  icon?: React.ReactNode;
  trailingIcon?: React.ReactNode;
};

export function GhostButton({ label, onPress, style, icon, trailingIcon }: GhostButtonProps) {
  return (
    <Pressable onPress={onPress}>
      {({ pressed }) => (
        <BlurView
          intensity={50}
          tint="systemUltraThinMaterialDark"
          style={[styles.button, style, pressed ? styles.buttonPressed : null]}
        >
          <View style={styles.content}>
            {icon ? <View style={styles.iconSlot}>{icon}</View> : null}
            <Text style={[styles.label, trailingIcon ? styles.labelTrailing : null]}>{label}</Text>
            {trailingIcon ? <View style={styles.trailingSlot}>{trailingIcon}</View> : null}
          </View>
        </BlurView>
      )}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    borderRadius: spacing.fieldRadius,
    paddingVertical: 10,
    paddingHorizontal: 16,
    borderWidth: 1,
    borderColor: colors.line,
    borderCurve: "continuous",
  },
  content: {
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  iconSlot: {
    alignItems: "center",
    justifyContent: "center",
  },
  label: {
    color: colors.textPrimary,
    fontSize: 15,
    fontWeight: "600",
  },
  labelTrailing: {
    flex: 1,
  },
  trailingSlot: {
    alignItems: "center",
    justifyContent: "center",
  },
  buttonPressed: {
    transform: [{ scale: 0.98 }],
    opacity: 0.9,
  },
});
