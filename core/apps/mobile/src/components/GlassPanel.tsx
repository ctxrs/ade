import { BlurView } from "expo-blur";
import React from "react";
import { Platform, StyleProp, StyleSheet, View, ViewStyle } from "react-native";

import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type GlassPanelProps = {
  children: React.ReactNode;
  style?: StyleProp<ViewStyle>;
  containerStyle?: StyleProp<ViewStyle>;
  padding?: number;
  radius?: number;
  intensity?: number;
};

export function GlassPanel({
  children,
  style,
  containerStyle,
  padding = spacing.panelPadding,
  radius = spacing.panelRadius,
  intensity = 50,
}: GlassPanelProps) {
  return (
    <View style={[styles.shadow, { borderRadius: radius }, containerStyle]}>
      <BlurView
        intensity={intensity}
        tint="systemUltraThinMaterialDark"
        style={[
          styles.panel,
          {
            padding,
            borderRadius: radius,
            borderColor: colors.glassStroke,
          },
          style,
        ]}
      >
        {children}
      </BlurView>
    </View>
  );
}

const styles = StyleSheet.create({
  shadow: {
    shadowColor: colors.shadow,
    shadowOpacity: 1,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: Platform.select({ android: 6, default: 0 }),
  },
  panel: {
    borderWidth: 0.8,
    borderCurve: "continuous",
    overflow: "hidden",
  },
});
