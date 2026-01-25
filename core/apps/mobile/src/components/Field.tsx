import React from "react";
import { StyleSheet, Text, TextInput, TextInputProps, View } from "react-native";

import { colors } from "../theme/colors";
import { spacing } from "../theme/spacing";

type FieldProps = {
  label: string;
  placeholder: string;
  value?: string;
  defaultValue?: string;
  secureTextEntry?: boolean;
  onChangeText?: TextInputProps["onChangeText"];
  autoCapitalize?: TextInputProps["autoCapitalize"];
  autoCorrect?: boolean;
  keyboardType?: TextInputProps["keyboardType"];
  textContentType?: TextInputProps["textContentType"];
};

export function Field({
  label,
  placeholder,
  value,
  defaultValue,
  secureTextEntry,
  onChangeText,
  autoCapitalize = "none",
  autoCorrect = false,
  keyboardType,
  textContentType,
}: FieldProps) {
  return (
    <View style={styles.wrapper}>
      <Text style={styles.label}>{label}</Text>
      <TextInput
        placeholder={placeholder}
        placeholderTextColor={colors.textMuted}
        value={value}
        defaultValue={defaultValue}
        onChangeText={onChangeText}
        secureTextEntry={secureTextEntry}
        autoCapitalize={autoCapitalize}
        autoCorrect={autoCorrect}
        keyboardType={keyboardType}
        textContentType={textContentType}
        style={styles.input}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  wrapper: {
    gap: 8,
  },
  label: {
    color: colors.textSecondary,
    fontSize: 13,
    fontWeight: "600",
  },
  input: {
    borderRadius: spacing.fieldRadius,
    borderWidth: 1,
    borderColor: colors.line,
    paddingHorizontal: 12,
    paddingVertical: 12,
    color: colors.textPrimary,
    backgroundColor: "rgba(37, 37, 38, 0.6)",
  },
});
