import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { colors } from "../theme/colors";
import { HarnessIcon } from "./HarnessIcon";

type TaskHarnessStackProps = {
  providerIds?: string[];
  maxVisible?: number;
  size?: number;
  overlap?: number;
};

export function TaskHarnessStack({
  providerIds = [],
  maxVisible = 2,
  size = 16,
  overlap = -6,
}: TaskHarnessStackProps) {
  const visible = providerIds.slice(0, maxVisible);
  const overflow = providerIds.length - visible.length;

  return (
    <View style={styles.container}>
      {visible.map((providerId, index) => (
        <View key={`${providerId}-${index}`} style={index === 0 ? null : { marginLeft: overlap }}>
          <HarnessIcon providerId={providerId} size={size} radius={6} backgroundOpacity={0.04} />
        </View>
      ))}
      {overflow > 0 ? (
        <View style={[styles.overflowPill, { marginLeft: overlap, height: size }]}>
          <Text style={styles.overflowText}>+{overflow}</Text>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flexDirection: "row",
    alignItems: "center",
  },
  overflowPill: {
    minWidth: 16,
    paddingHorizontal: 6,
    paddingVertical: 2,
    borderRadius: 999,
    backgroundColor: "rgba(37, 37, 38, 0.7)",
    borderWidth: 1,
    borderColor: colors.line,
    alignItems: "center",
    justifyContent: "center",
  },
  overflowText: {
    color: colors.textSecondary,
    fontSize: 10,
    fontWeight: "600",
  },
});
