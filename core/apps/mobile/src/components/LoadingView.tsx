import React from "react";
import { ActivityIndicator, View } from "react-native";

import { createContextStyles, useContextTokens } from "../theme";

export function LoadingView(): React.JSX.Element {
  const t = useContextTokens();
  return (
    <View style={styles.container}>
      <ActivityIndicator color={t.colors.accent} size="large" />
    </View>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  container: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: t.colors.background,
  },
}));
