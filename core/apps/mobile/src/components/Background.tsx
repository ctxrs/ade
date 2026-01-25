import { LinearGradient } from "expo-linear-gradient";
import React from "react";
import { StyleSheet, View } from "react-native";
import Svg, { Defs, RadialGradient, Rect, Stop } from "react-native-svg";

import { colors } from "../theme/colors";

export function Background() {
  return (
    <View pointerEvents="none" style={StyleSheet.absoluteFill}>
      <LinearGradient
        colors={[colors.background, colors.backgroundDeep]}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 1 }}
        style={StyleSheet.absoluteFill}
      />
      <Svg height="100%" width="100%" style={StyleSheet.absoluteFill}>
        <Defs>
          <RadialGradient
            id="ctxGlow"
            cx="85%"
            cy="90%"
            rx="60%"
            ry="60%"
            fx="85%"
            fy="90%"
          >
            <Stop offset="0%" stopColor={colors.surfaceRaised} stopOpacity={0.6} />
            <Stop offset="100%" stopColor={colors.backgroundDeep} stopOpacity={0} />
          </RadialGradient>
        </Defs>
        <Rect width="100%" height="100%" fill="url(#ctxGlow)" />
      </Svg>
    </View>
  );
}
