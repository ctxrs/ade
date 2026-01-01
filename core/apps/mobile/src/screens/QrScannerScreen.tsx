import Constants from "expo-constants";
import { CameraView, useCameraPermissions } from "expo-camera";
import type { BarcodeScanningResult } from "expo-camera";
import type { BarCodeEvent } from "expo-barcode-scanner";
import React, { useEffect, useMemo, useState } from "react";
import { Buffer } from "buffer";
import { Alert, Platform, StyleSheet, Text, TouchableOpacity, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { pairMobileDevice, type ConnectionConfig } from "../api/client";
import { useConnection } from "../state/ConnectionProvider";
import { loadOrCreateDeviceIdentity } from "../state/deviceIdentity";
import { decryptPayload } from "../utils/e2ee";
import { getSecureConnectionContext } from "../utils/secureConnection";
import type { RootStackParamList } from "../navigation/types";
import { createContextStyles } from "../theme";

type Props = NativeStackScreenProps<RootStackParamList, "QrScanner">;

export function QrScannerScreen({ navigation }: Props): React.JSX.Element {
  const [cameraPermission, requestCameraPermission] = useCameraPermissions();
  const [webPermission, setWebPermission] = useState<"granted" | "denied" | "prompt">("prompt");
  const [scanned, setScanned] = useState(false);
  const { setConnection, config } = useConnection();
  const isWeb = Platform.OS === "web";
  const webScannerModule = useMemo(() => {
    if (!isWeb) return null;
    try {
      return require("expo-barcode-scanner");
    } catch (err) {
      console.warn("[QrScanner] Failed to load web barcode scanner", err);
      return null;
    }
  }, [isWeb]);

  useEffect(() => {
    let cancelled = false;
    if (isWeb) {
      (async () => {
        try {
          const { status } = await webScannerModule?.BarCodeScanner.requestPermissionsAsync();
          if (!cancelled) {
            setWebPermission(status === "granted" ? "granted" : "denied");
          }
        } catch (err) {
          console.warn("[QrScanner] Failed to request web permissions", err);
          if (!cancelled) setWebPermission("denied");
        }
      })();
      return () => {
        cancelled = true;
      };
    }
    if (!cameraPermission?.granted) {
      requestCameraPermission().catch((err) =>
        console.warn("[QrScanner] Failed to request camera permissions", err),
      );
    }
  }, [cameraPermission?.granted, isWeb, requestCameraPermission, webScannerModule]);

  const wasConnected = Boolean(config);

  const handleScan = async (data: string | undefined) => {
    if (!data) return;
    if (scanned) return;
    setScanned(true);
    try {
      const parsed = JSON.parse(data);
      const profile = extractProfile(parsed);
      if (!profile) throw new Error("QR code is missing connection info.");
      let nextConfig: ConnectionConfig;
      if (profile.mode === "e2ee") {
        const identity = await loadOrCreateDeviceIdentity();
        const envelope = await pairMobileDevice(profile.baseUrl, {
          pairing_token: profile.pairingToken,
          device_id: identity.deviceId,
          device_label: Constants.deviceName ?? Platform.OS,
          platform: Platform.OS,
          public_key: identity.publicKey,
          app_version: Constants.expoConfig?.version,
        });
        if (envelope.device_id !== identity.deviceId) {
          throw new Error("Pairing response device mismatch.");
        }
        const { key } = await getSecureConnectionContext(identity.deviceId, profile.daemonPublicKey);
        const plaintext = decryptPayload(key, identity.deviceId, envelope.seq, envelope);
        const ack = JSON.parse(decodeText(plaintext)) as { paired?: boolean };
        if (!ack?.paired) {
          throw new Error("Pairing was not accepted.");
        }
        nextConfig = {
          baseUrl: profile.baseUrl,
          deviceId: identity.deviceId,
          daemonPublicKey: profile.daemonPublicKey,
        };
      } else {
        nextConfig = { baseUrl: profile.baseUrl, token: profile.token };
      }
      await setConnection(nextConfig);
      Alert.alert("Connection updated", `Connected to ${nextConfig.baseUrl}`);
      if (wasConnected) {
        navigation.goBack();
      } else {
        navigation.reset({ index: 0, routes: [{ name: "Workbench" }] });
      }
    } catch (err: any) {
      Alert.alert("Scan failed", err?.message ?? String(err), [
        {
          text: "Try again",
          onPress: () => setScanned(false),
        },
      ]);
    }
  };

  const permissionDenied = isWeb ? webPermission === "denied" : cameraPermission?.granted === false;
  if (permissionDenied) {
    return (
      <SafeAreaView style={styles.safe}>
        <View style={styles.center}>
          <Text style={styles.text}>Camera access denied. Enable it in Settings to scan QR codes.</Text>
          <TouchableOpacity onPress={() => navigation.goBack()} style={styles.button}>
            <Text style={styles.buttonLabel}>Go back</Text>
          </TouchableOpacity>
        </View>
      </SafeAreaView>
    );
  }

  return (
    <SafeAreaView style={styles.safe}>
      <View style={styles.scannerContainer}>
        {renderScanner()}
      </View>
      <Text style={styles.instructions}>
        Scan the QR code from the desktop “Mobile Access” settings. It should embed the tunnel URL and pairing
        token.
      </Text>
    </SafeAreaView>
  );

  function renderScanner(): React.JSX.Element {
    if (isWeb) {
      const WebScanner = webScannerModule?.BarCodeScanner;
      if (!WebScanner) {
        return (
          <View style={styles.center}>
            <Text style={styles.text}>QR scanning is not available in the web preview.</Text>
          </View>
        );
      }
      if (webPermission !== "granted") {
        return (
          <View style={styles.center}>
            <Text style={styles.text}>Requesting camera access…</Text>
          </View>
        );
      }
      return (
        <WebScanner
          onBarCodeScanned={(result: BarCodeEvent) => handleScan(result.data)}
          style={StyleSheet.absoluteFillObject}
        />
      );
    }
    if (!cameraPermission?.granted) {
      return (
        <View style={styles.center}>
          <Text style={styles.text}>Requesting camera access…</Text>
        </View>
      );
    }
    return (
      <CameraView
        style={StyleSheet.absoluteFillObject}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={(result: BarcodeScanningResult) => handleScan(result.data)}
      />
    );
  }
}

const decodeText = (bytes: Uint8Array): string => {
  if (typeof TextDecoder !== "undefined") {
    return new TextDecoder().decode(bytes);
  }
  return Buffer.from(bytes).toString("utf-8");
};

type ParsedQr =
  | { mode: "e2ee"; baseUrl: string; pairingToken: string; daemonPublicKey: string }
  | { mode: "legacy"; baseUrl: string; token: string };

const extractProfile = (data: any): ParsedQr | null => {
  if (!data) return null;
  if (data.type === "context_mobile_e2ee") {
    const baseUrl = data.base_url || data.baseUrl;
    const pairingToken = data.pairing_token || data.pairingToken;
    const daemonPublicKey = data.daemon_public_key || data.daemonPublicKey;
    if (!baseUrl || !pairingToken || !daemonPublicKey) return null;
    return {
      mode: "e2ee",
      baseUrl: String(baseUrl),
      pairingToken: String(pairingToken),
      daemonPublicKey: String(daemonPublicKey),
    };
  }
  const profile = data.connection_profile ?? data;
  const connection = profile.connection ?? {};
  const auth = profile.auth ?? {};
  const baseUrl =
    connection.base_url || connection.baseUrl || connection.url || connection.baseURL || profile.baseUrl;
  const token = auth.api_token || auth.token || profile.api_token || profile.token;
  if (!baseUrl || !token) return null;
  return { mode: "legacy", baseUrl: String(baseUrl), token: String(token) };
};

const styles: Record<string, any> = createContextStyles((t) => ({
  safe: { flex: 1, backgroundColor: t.colors.background },
  scannerContainer: {
    flex: 1,
  },
  instructions: {
    padding: t.spacing.md,
    textAlign: "center",
    color: t.colors.text,
    backgroundColor: t.colors.background,
  },
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    padding: t.spacing.xl,
  },
  text: {
    color: t.colors.text,
    textAlign: "center",
  },
  button: {
    marginTop: t.spacing.md,
    paddingHorizontal: t.spacing.lg,
    paddingVertical: t.spacing.sm,
    backgroundColor: t.colors.accent,
    borderRadius: t.radii.pill,
  },
  buttonLabel: {
    color: t.colors.text,
    fontWeight: "700",
  },
}));
