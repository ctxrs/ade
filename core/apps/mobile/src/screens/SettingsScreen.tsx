import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import Constants from "expo-constants";
import React, { useCallback, useEffect, useState } from "react";
import { Switch, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { RootStackParamList } from "../navigation/types";
import { isQueueWatcherRegistered, registerQueueWatcher, unregisterQueueWatcher } from "../background/queueWatcher";
import { createContextStyles } from "../theme";
import { useConnection } from "../state/ConnectionProvider";
import { syncDeviceRegistration } from "../state/deviceRegistration";

type Props = NativeStackScreenProps<RootStackParamList, "Settings">;

export function SettingsScreen(_: Props): React.JSX.Element {
  const [queueWatcherEnabled, setQueueWatcherEnabled] = useState(false);
  const [deviceStatus, setDeviceStatus] = useState<string | null>(null);
  const [syncingDevice, setSyncingDevice] = useState(false);
  const { config } = useConnection();
  const isExpoGo = Constants.appOwnership === "expo";

  useEffect(() => {
    (async () => {
      setQueueWatcherEnabled(await isQueueWatcherRegistered());
    })();
  }, []);

  const ensureRegistration = useCallback(async (): Promise<boolean> => {
    if (!config) {
      setDeviceStatus("Connect to a daemon to register this device.");
      return false;
    }
    setSyncingDevice(true);
    setDeviceStatus("Registering device…");
    try {
      await syncDeviceRegistration(config);
      setDeviceStatus("Device registered.");
      return true;
    } catch (err: any) {
      setDeviceStatus(err?.message ?? "Failed to register device.");
      return false;
    } finally {
      setSyncingDevice(false);
    }
  }, [config]);

  useEffect(() => {
    if (config && queueWatcherEnabled) {
      void ensureRegistration();
    }
  }, [config, queueWatcherEnabled, ensureRegistration]);

  const toggle = async () => {
    if (isExpoGo) {
      setDeviceStatus("Expo Go can’t enable background alerts; install a dev build to use this switch.");
      return;
    }
    if (queueWatcherEnabled) {
      await unregisterQueueWatcher();
      setQueueWatcherEnabled(false);
    } else {
      if (!config) {
        setDeviceStatus("Connect to a daemon before enabling alerts.");
        return;
      }
      await registerQueueWatcher();
      const ok = await ensureRegistration();
      if (ok) {
        setQueueWatcherEnabled(true);
      } else {
        await unregisterQueueWatcher();
      }
    }
  };

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.row}>
        <View style={styles.textCol}>
          <Text style={styles.title}>Background approvals alert</Text>
          <Text style={styles.caption}>
            Polls the daemon every few minutes and sends a local notification when tasks need your input.
          </Text>
        </View>
        <Switch
          value={queueWatcherEnabled}
          onValueChange={() => void toggle()}
          disabled={!config || syncingDevice || isExpoGo}
        />
      </View>
      {deviceStatus && <Text style={styles.status}>{deviceStatus}</Text>}
    </SafeAreaView>
  );
}

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: {
    flex: 1,
    backgroundColor: t.colors.background,
    padding: t.spacing.xl,
  },
  row: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: t.spacing.lg,
  },
  textCol: {
    flex: 1,
    gap: t.spacing.xs,
  },
  title: {
    color: t.colors.text,
    fontSize: t.typography.sizes.lg,
    fontWeight: "600",
  },
  caption: {
    color: t.colors.muted,
  },
  status: {
    marginTop: t.spacing.md,
    color: t.colors.muted,
  },
}));
