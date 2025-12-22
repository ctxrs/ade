import React from "react";
import { Text, View } from "react-native";

import { createContextStyles } from "../theme";

type Props = {
  message: string;
};

export function ErrorView({ message }: Props): React.JSX.Element {
  return (
    <View style={styles.container}>
      <Text style={styles.title}>Something went wrong</Text>
      <Text style={styles.message}>{message}</Text>
    </View>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  container: {
    flex: 1,
    padding: t.spacing.xl,
    backgroundColor: t.colors.background,
    gap: t.spacing.sm,
    alignItems: "center",
    justifyContent: "center",
  },
  title: {
    color: t.colors.danger,
    fontSize: t.typography.sizes.lg,
    fontWeight: "700",
  },
  message: {
    color: t.colors.text,
    textAlign: "center",
  },
}));
