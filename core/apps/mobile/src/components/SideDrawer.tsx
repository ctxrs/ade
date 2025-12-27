import React, { useEffect, useMemo, useRef } from "react";
import { Animated, Pressable, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useContextTokens } from "../theme";

export type SideDrawerProps = {
  open: boolean;
  width?: number;
  onRequestClose: () => void;
  drawer: React.ReactNode;
  children: React.ReactNode;
};

export function SideDrawer({
  open,
  width = 320,
  onRequestClose,
  drawer,
  children,
}: SideDrawerProps): React.JSX.Element {
  const theme = useContextTokens();
  const insets = useSafeAreaInsets();
  const translateX = useRef(new Animated.Value(open ? 0 : -width)).current;
  const backdropOpacity = useRef(new Animated.Value(open ? 1 : 0)).current;

  useEffect(() => {
    Animated.parallel([
      Animated.timing(translateX, {
        toValue: open ? 0 : -width,
        duration: open ? 220 : 180,
        useNativeDriver: true,
      }),
      Animated.timing(backdropOpacity, {
        toValue: open ? 1 : 0,
        duration: open ? 220 : 180,
        useNativeDriver: true,
      }),
    ]).start();
  }, [open, translateX, backdropOpacity, width]);

  const pointerEvents = open ? "auto" : "none";
  const overlayColor = useMemo(() => {
    return theme.colors.background;
  }, [theme.colors.background]);

  return (
    <View style={{ flex: 1 }}>
      {children}
      <Animated.View
        pointerEvents={pointerEvents}
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          right: 0,
          bottom: 0,
          opacity: backdropOpacity,
        }}
      >
        <Pressable
          onPress={onRequestClose}
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            backgroundColor: overlayColor,
            opacity: 0.55,
          }}
        />
      </Animated.View>

      <Animated.View
        style={{
          position: "absolute",
          left: 0,
          top: 0,
          bottom: 0,
          width,
          transform: [{ translateX }],
          paddingTop: insets.top,
          backgroundColor: theme.colors.surface,
          borderRightWidth: 1,
          borderRightColor: theme.colors.border,
        }}
      >
        {drawer}
      </Animated.View>
    </View>
  );
}

