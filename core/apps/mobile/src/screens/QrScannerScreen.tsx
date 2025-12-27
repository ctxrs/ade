import { CameraView, useCameraPermissions } from "expo-camera";
import type { BarcodeScanningResult } from "expo-camera";
import type { BarCodeEvent } from "expo-barcode-scanner";
import React, { useEffect, useMemo, useState } from "react";
import { Alert, Platform, StyleSheet, Text, TouchableOpacity, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useConnection } from "../state/ConnectionProvider";
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
      await setConnection(profile);
      Alert.alert("Connection updated", `Connected to ${profile.baseUrl}`);
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
        Scan the QR code from the desktop “Enable mobile connection” dialog. It should embed the daemon HTTPS
        URL and API token.
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

const extractProfile = (data: any) => {
  if (!data) return null;
  const profile = data.connection_profile ?? data;
  const connection = profile.connection ?? {};
  const auth = profile.auth ?? {};
  const baseUrl =
    connection.base_url || connection.baseUrl || connection.url || connection.baseURL || profile.baseUrl;
  const token = auth.api_token || auth.token || profile.api_token || profile.token;
  if (!baseUrl || !token) return null;
  return { baseUrl: String(baseUrl), token: String(token) };
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
