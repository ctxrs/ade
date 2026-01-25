import { StatusBar } from "expo-status-bar";
import React from "react";

import { ConnectionScreen } from "./src/screens/ConnectionScreen";
import { DiffPanelScreen } from "./src/screens/DiffPanelScreen";
import { DiagnosticsScreen } from "./src/screens/DiagnosticsScreen";
import { MobileAccessScreen } from "./src/screens/MobileAccessScreen";
import { NewTaskScreen } from "./src/screens/NewTaskScreen";
import { SettingsScreen } from "./src/screens/SettingsScreen";
import { TaskListScreen } from "./src/screens/TaskListScreen";
import { WorkspaceSelectorScreen } from "./src/screens/WorkspaceSelectorScreen";
import { ChatScreen } from "./src/screens/ChatScreen";

export default function App() {
  const screen = process.env.EXPO_PUBLIC_SCREEN ?? "connection";

  const content =
    screen === "task-list" ? (
      <TaskListScreen />
    ) : screen === "new-task" ? (
      <NewTaskScreen />
    ) : screen === "diff-panel" ? (
      <DiffPanelScreen />
    ) : screen === "chat" ? (
      <ChatScreen />
    ) : screen === "settings" ? (
      <SettingsScreen />
    ) : screen === "mobile-access" ? (
      <MobileAccessScreen />
    ) : screen === "diagnostics" ? (
      <DiagnosticsScreen />
    ) : screen === "workspace-selector" ? (
      <WorkspaceSelectorScreen />
    ) : (
      <ConnectionScreen />
    );

  return (
    <>
      <StatusBar style="light" />
      {content}
    </>
  );
}
