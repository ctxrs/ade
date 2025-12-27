import React from "react";
import { Pressable, type PressableProps, View } from "react-native";
import { useContextTokens } from "../theme";

export type IconButtonProps = Omit<PressableProps, "children"> & {
  icon: React.ReactNode;
  size?: number;
};

export function IconButton({ icon, style, size = 36, ...props }: IconButtonProps): React.JSX.Element {
  const theme = useContextTokens();
  return (
    <Pressable
      {...props}
      hitSlop={10}
      style={[
        {
          width: size,
          height: size,
          borderRadius: size / 2,
          alignItems: "center",
          justifyContent: "center",
        },
        style as any,
      ]}
    >
      <View pointerEvents="none" style={{ opacity: props.disabled ? 0.5 : 1 }}>
        {icon}
      </View>
    </Pressable>
  );
}
