import * as BackgroundFetch from "expo-background-fetch";
import { BackgroundFetchResult } from "expo-background-fetch";
import Constants from "expo-constants";
import * as Notifications from "expo-notifications";
import * as TaskManager from "expo-task-manager";
import { Platform } from "react-native";

import { listTasks, listWorkspaces } from "../api/client";
import { loadConnectionConfig } from "../state/connectionStorage";

const TASK_NAME = "contextQueueWatcher";
const isExpoGo = Constants.appOwnership === "expo";

if (isExpoGo) {
  void BackgroundFetch.unregisterTaskAsync(TASK_NAME).catch((err) =>
    console.warn("[queueWatcher] failed to unregister Expo Go task", err),
  );
}

TaskManager.defineTask(TASK_NAME, async () => {
  if (isExpoGo) return BackgroundFetchResult.NoData;
  const conn = await loadConnectionConfig();
  if (!conn) return BackgroundFetchResult.NoData;
  try {
    const workspaces = await listWorkspaces(conn);
    let needsAttention = 0;
    for (const workspace of workspaces) {
      const tasks = await listTasks(conn, workspace.id);
      for (const task of tasks) {
        const status = (task.status || "").toLowerCase();
        if (status.includes("await") || status.includes("needs") || status.includes("blocked")) {
          needsAttention += 1;
        }
      }
    }
    if (needsAttention > 0) {
      const soundSetting = Platform.select<boolean | undefined>({
        ios: true,
        android: true,
        default: undefined,
      });
      await Notifications.scheduleNotificationAsync({
        content: {
          title: "ctx needs your input",
          body: `${needsAttention} task${needsAttention === 1 ? "" : "s"} awaiting approval/input.`,
          sound: soundSetting,
        },
        trigger: null,
      });
      return BackgroundFetchResult.NewData;
    }
    return BackgroundFetchResult.NoData;
  } catch (err) {
    console.warn("[queueWatcher] failed", err);
    return BackgroundFetchResult.Failed;
  }
});

export const registerQueueWatcher = async (): Promise<void> => {
  if (isExpoGo) {
    console.info("[queueWatcher] Skipping background registration inside Expo Go");
    return;
  }
  await Notifications.requestPermissionsAsync();
  const alreadyRegistered = await TaskManager.isTaskRegisteredAsync(TASK_NAME);
  if (alreadyRegistered) return;
  if (Platform.OS === "android") {
    await Notifications.setNotificationChannelAsync("ctx-alerts", {
      name: "ctx alerts",
      importance: Notifications.AndroidImportance.HIGH,
    });
  }
  await BackgroundFetch.registerTaskAsync(TASK_NAME, {
    minimumInterval: 60 * 5,
    stopOnTerminate: false,
    startOnBoot: true,
  });
};

export const unregisterQueueWatcher = async (): Promise<void> => {
  const isRegistered = await TaskManager.isTaskRegisteredAsync(TASK_NAME);
  if (isRegistered) {
    await BackgroundFetch.unregisterTaskAsync(TASK_NAME);
  }
};

export const isQueueWatcherRegistered = async (): Promise<boolean> => {
  if (isExpoGo) return false;
  return TaskManager.isTaskRegisteredAsync(TASK_NAME);
};
