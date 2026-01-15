import "react-native-get-random-values";
import { NavigationContainer } from "@react-navigation/native";
import { createNativeStackNavigator } from "@react-navigation/native-stack";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import * as Notifications from "expo-notifications";
import React, { useEffect } from "react";
import { TouchableOpacity, Text, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { ConnectionScreen } from "./src/screens/ConnectionScreen";
import { WorkspaceListScreen } from "./src/screens/WorkspaceListScreen";
import { TaskListScreen } from "./src/screens/TaskListScreen";
import { SessionListScreen } from "./src/screens/SessionListScreen";
import { SessionDetailScreen } from "./src/screens/SessionDetailScreen";
import { SessionDiffScreen } from "./src/screens/SessionDiffScreen";
import { DiagnosticsScreen } from "./src/screens/DiagnosticsScreen";
import { SettingsScreen } from "./src/screens/SettingsScreen";
import { QrScannerScreen } from "./src/screens/QrScannerScreen";
import { WorkbenchScreen } from "./src/screens/WorkbenchScreen";
import { NewTaskScreen } from "./src/screens/NewTaskScreen";
import type { RootStackParamList } from "./src/navigation/types";
import { ConnectionProvider, useConnection } from "./src/state/ConnectionProvider";
import { WorkspaceCatchupProvider, useWorkspaceCatchupStore } from "./src/state/workspaceCatchupStore";
import { SessionSupervisorProvider, useSessionSupervisor } from "./src/state/sessionSupervisor";
import { WorkspaceSelectionProvider, useWorkspaceSelection } from "./src/state/WorkspaceSelectionProvider";
import { WorkbenchSelectionProvider } from "./src/state/WorkbenchSelectionProvider";
import { LoadingView } from "./src/components/LoadingView";
import { tokens } from "./src/theme";

const Stack = createNativeStackNavigator<RootStackParamList>();
const queryClient = new QueryClient();

Notifications.setNotificationHandler({
  handleNotification: async () => ({
    shouldPlaySound: true,
    shouldSetBadge: false,
    shouldShowAlert: true,
    shouldShowBanner: true,
    shouldShowList: true,
  }),
});

export default function App(): React.JSX.Element {
  return (
    <SafeAreaProvider>
      <QueryClientProvider client={queryClient}>
        <ConnectionProvider>
          <WorkspaceSelectionProvider>
            <WorkbenchSelectionProvider>
              <SessionSupervisorProvider>
                <WorkspaceDataProviders>
                  <NavigationContainer>
                    <RootNavigator />
                  </NavigationContainer>
                </WorkspaceDataProviders>
              </SessionSupervisorProvider>
            </WorkbenchSelectionProvider>
          </WorkspaceSelectionProvider>
        </ConnectionProvider>
      </QueryClientProvider>
    </SafeAreaProvider>
  );
}

function WorkspaceDataProviders({ children }: { children: React.ReactNode }): React.JSX.Element {
  const { workspaceId } = useWorkspaceSelection();

  return (
    <WorkspaceCatchupProvider workspaceId={workspaceId}>
      {workspaceId ? <WorkspaceCatchupBinding /> : null}
      {children}
    </WorkspaceCatchupProvider>
  );
}

function WorkspaceCatchupBinding(): null {
  const supervisor = useSessionSupervisor();
  const store = useWorkspaceCatchupStore();

  useEffect(() => {
    supervisor.bindWorkspaceCatchupStore(store);
    return () => supervisor.bindWorkspaceCatchupStore(null);
  }, [supervisor, store]);

  return null;
}

function RootNavigator(): React.JSX.Element {
  const { loading, config } = useConnection();

  if (loading) return <LoadingView />;

  return (
    <Stack.Navigator
      screenOptions={{
        headerStyle: { backgroundColor: tokens.colors.surface },
        headerTintColor: tokens.colors.text,
      }}
    >
      {!config ? (
        <>
          <Stack.Screen name="Connection" component={ConnectionScreen} options={{ headerShown: false }} />
          <Stack.Screen
            name="QrScanner"
            component={QrScannerScreen}
            options={{ presentation: "fullScreenModal", title: "Scan QR" }}
          />
        </>
      ) : (
        <>
          <Stack.Screen name="Workbench" component={WorkbenchScreen} options={{ headerShown: false }} />
          <Stack.Screen name="NewTask" component={NewTaskScreen} options={{ presentation: "modal", headerShown: false }} />
          <Stack.Screen
            name="Workspaces"
            component={WorkspaceListScreen}
            options={({ navigation }) => ({
              headerRight: () => (
                <View style={{ flexDirection: "row", gap: 12 }}>
                  <TouchableOpacity onPress={() => navigation.navigate("Diagnostics")}>
                    <Text style={{ color: tokens.colors.accent, fontWeight: "600" }}>Diagnostics</Text>
                  </TouchableOpacity>
                  <TouchableOpacity onPress={() => navigation.navigate("Settings")}>
                    <Text style={{ color: tokens.colors.accent, fontWeight: "600" }}>Settings</Text>
                  </TouchableOpacity>
                  <TouchableOpacity onPress={() => navigation.navigate("Connection")}>
                    <Text style={{ color: tokens.colors.accent, fontWeight: "600" }}>Connection</Text>
                  </TouchableOpacity>
                </View>
              ),
            })}
          />
          <Stack.Screen name="Tasks" component={TaskListScreen} options={({ route }) => ({ title: route.params.workspaceName })} />
          <Stack.Screen name="Sessions" component={SessionListScreen} options={({ route }) => ({ title: route.params.taskTitle })} />
          <Stack.Screen name="SessionDetail" component={SessionDetailScreen} options={({ route }) => ({ title: route.params.sessionTitle })} />
          <Stack.Screen name="SessionDiff" component={SessionDiffScreen} options={({ route }) => ({ title: `${route.params.sessionTitle} diff` })} />
          <Stack.Screen name="Diagnostics" component={DiagnosticsScreen} />
          <Stack.Screen name="Settings" component={SettingsScreen} />
          <Stack.Screen name="Connection" component={ConnectionScreen} options={{ presentation: "modal" }} />
          <Stack.Screen
            name="QrScanner"
            component={QrScannerScreen}
            options={{ presentation: "fullScreenModal", title: "Scan QR" }}
          />
        </>
      )}
    </Stack.Navigator>
  );
}
