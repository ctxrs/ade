import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import React, { useState } from "react";
import { Button, ScrollView, Text, TextInput, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { useConnection } from "../state/ConnectionProvider";
import { createContextStyles, useContextTokens } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "Connection">;

export function ConnectionScreen({ navigation }: Props): React.JSX.Element {
  const { config, setConnection, clearConnection } = useConnection();
  const theme = useContextTokens();
  const [baseUrl, setBaseUrl] = useState(config?.baseUrl ?? "");
  const [token, setToken] = useState(config?.token ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const onSave = async () => {
    setError(null);
    setSaving(true);
    try {
      const trimmedUrl = baseUrl.trim();
      const trimmedToken = token.trim();
      if (!trimmedUrl || !trimmedToken) {
        setError("Base URL and API token are required.");
        return;
      }
      await setConnection({ baseUrl: trimmedUrl, token: trimmedToken });
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setSaving(false);
    }
  };

  const onClear = async () => {
    setError(null);
    setSaving(true);
    try {
      await clearConnection();
      setBaseUrl("");
      setToken("");
    } catch (e: any) {
      setError(e?.message ?? String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <SafeAreaView style={styles.safe}>
      <ScrollView contentContainerStyle={styles.container}>
        <Text style={styles.title}>Connect to Context Daemon</Text>
        <Text style={styles.subtitle}>
          Enter the HTTPS base URL (tunnel or direct) plus the API token generated from the desktop
          app’s “Enable mobile connection” flow.
        </Text>
        <View style={styles.field}>
          <Text style={styles.label}>Daemon Base URL</Text>
          <TextInput
            autoCapitalize="none"
            autoCorrect={false}
            inputMode="url"
            placeholder="https://example.devbox.com"
            value={baseUrl}
            onChangeText={setBaseUrl}
            style={styles.input}
          />
        </View>
        <View style={styles.field}>
          <Text style={styles.label}>API Token</Text>
          <TextInput
            autoCapitalize="none"
            autoCorrect={false}
            placeholder="Paste token"
            secureTextEntry
            value={token}
            onChangeText={setToken}
            style={styles.input}
          />
        </View>
        {error && <Text style={styles.error}>{error}</Text>}
        <Button title="Scan QR Code" onPress={() => navigation.navigate("QrScanner")} />
        <View style={styles.buttonRow}>
          <Button title={config ? "Update Connection" : "Connect"} onPress={onSave} disabled={saving} />
        </View>
        {config && (
          <View style={styles.buttonRow}>
            <Button title="Disconnect" onPress={onClear} disabled={saving} color={theme.colors.danger} />
          </View>
        )}
      </ScrollView>
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: {
    flex: 1,
    backgroundColor: t.colors.background,
  },
  container: {
    padding: t.spacing.xl,
    gap: t.spacing.md,
  },
  title: {
    fontSize: t.typography.sizes.xl,
    fontWeight: "700",
    color: t.colors.text,
  },
  subtitle: {
    color: t.colors.muted,
    fontSize: t.typography.sizes.md,
    lineHeight: 22,
  },
  field: {
    gap: t.spacing.xs,
  },
  label: {
    color: t.colors.text,
    fontWeight: "600",
  },
  input: {
    backgroundColor: t.colors.surfaceAlt,
    borderRadius: t.radii.md,
    borderColor: t.colors.border,
    borderWidth: 1,
    paddingHorizontal: t.spacing.md,
    paddingVertical: t.spacing.sm,
    color: t.colors.text,
  },
  error: {
    color: t.colors.danger,
    fontWeight: "600",
  },
  buttonRow: {
    marginTop: t.spacing.sm,
    gap: t.spacing.sm,
  },
}));
