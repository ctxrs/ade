const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const { waitForTauri } = require("./helpers/tauri.cjs");
const {
  tauriInvoke,
  waitForDesktopAppReady,
} = require("./helpers/container_lifecycle.cjs");

const permissionSetupMessage = [
  "macOS notification permission for bundle identifier rs.ctx.desktop is not granted.",
  "Prepare the Mac mini by launching the signed ctx.app once from the desktop session and allowing notifications in the native prompt,",
  "or grant notifications for ctx in System Settings > Notifications before running ctx-mac-nightly unattended.",
].join(" ");

const requireTauriValue = async (command, payload = {}) => {
  const response = await tauriInvoke(command, payload);
  if (response.error) {
    throw new Error(`${command} failed: ${response.error}`);
  }
  return response.value;
};

const boolish = (value) => {
  const normalized = String(value || "").trim().toLowerCase();
  return normalized === "1" || normalized === "true" || normalized === "yes";
};

const isCi = () =>
  boolish(process.env.CI) ||
  boolish(process.env.BUILDKITE) ||
  boolish(process.env.CTX_AUTOMATION_REQUIRE_PREGRANTED_NOTIFICATIONS);

const requireSignedAppPath = () => {
  if (!boolish(process.env.CTX_AUTOMATION_EXPECT_SIGNED_APP)) {
    return;
  }
  if (process.platform !== "darwin") {
    throw new Error("signed notification smoke requires macOS");
  }
  const appPath = path.resolve(String(process.env.CTX_DESKTOP_APP_PATH || "").trim());
  if (!appPath || !appPath.endsWith(".app")) {
    throw new Error(`CTX_DESKTOP_APP_PATH must point to a signed .app bundle, got '${appPath}'`);
  }
  const executablePath = path.join(appPath, "Contents", "MacOS", "ctx");
  if (!fs.existsSync(executablePath)) {
    throw new Error(`signed app executable missing: ${executablePath}`);
  }
  const codesign = spawnSync("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=2", appPath], {
    encoding: "utf8",
  });
  if (codesign.status !== 0) {
    throw new Error(
      `signed app codesign verification failed for ${appPath}\n${codesign.stdout || ""}${codesign.stderr || ""}`,
    );
  }
};

const requireNotificationPermission = async () => {
  let permission = await requireTauriValue("desktop_get_notification_permission");
  if (permission === "granted") {
    return;
  }
  if (permission === "default" && !isCi()) {
    permission = await requireTauriValue("desktop_request_notification_permission");
    if (permission === "granted") {
      return;
    }
  }
  throw new Error(`${permissionSetupMessage} Current permission: ${String(permission)}`);
};

const summarizeDeliveredSnapshot = (snapshot, { deepLink, title }) => {
  const delivered = Array.isArray(snapshot?.delivered) ? snapshot.delivered : [];
  const entries = delivered.map((entry) => ({
    deepLinkMatches: entry?.deepLink === deepLink,
    identifier: String(entry?.identifier || ""),
    titleMatches: entry?.title === title,
  }));
  return {
    deliveredCount: delivered.length,
    matchingCount: entries.filter((entry) => entry.titleMatches && entry.deepLinkMatches && entry.identifier).length,
    notificationIdentifiers: entries.map((entry) => entry.identifier).filter(Boolean),
  };
};

const summarizeAutomationSnapshot = (snapshot, { expectedTaskId }) => {
  const deepLink = String(snapshot?.deepLink || "");
  return {
    deepLinkContainsExpectedTask: deepLink.includes(expectedTaskId),
    hasBody: typeof snapshot?.body === "string" && snapshot.body.length > 0,
    hasDeepLink: deepLink.length > 0,
    hasTitle: typeof snapshot?.title === "string" && snapshot.title.length > 0,
  };
};

const waitForDeliveredNotification = async ({ deepLink, title, timeoutMs = 30000 }) => {
  let lastSnapshot = null;
  let matched = null;
  try {
    await browser.waitUntil(async () => {
      lastSnapshot = await requireTauriValue("desktop_get_delivered_notification_automation_snapshot");
      const delivered = Array.isArray(lastSnapshot?.delivered) ? lastSnapshot.delivered : [];
      matched = delivered.find((entry) => entry?.title === title && entry?.deepLink === deepLink) || null;
      return Boolean(matched?.identifier);
    }, {
      timeout: timeoutMs,
      interval: 500,
      timeoutMsg: "delivered notification did not appear",
    });
  } catch (error) {
    const summary = summarizeDeliveredSnapshot(lastSnapshot, { deepLink, title });
    throw new Error(`delivered notification did not appear; lastSummary=${JSON.stringify(summary)}`, { cause: error });
  }
  return matched;
};

describe("signed macOS notification smoke", () => {
  it("delivers a real system notification from a signed app bundle", async () => {
    if (process.platform !== "darwin") {
      throw new Error("signed macOS notification smoke requires macOS");
    }
    if (String(process.env.CTX_AUTOMATION_SIMULATE_SYSTEM_NOTIFICATIONS || "").trim() === "1") {
      throw new Error("signed notification smoke must not run with CTX_AUTOMATION_SIMULATE_SYSTEM_NOTIFICATIONS=1");
    }

    requireSignedAppPath();

    await browser.url("tauri://localhost");
    await waitForTauri();
    await waitForDesktopAppReady();
    await requireNotificationPermission();
    await requireTauriValue("desktop_clear_notification_automation_snapshot");

    const nonce = `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    const title = `ctx notification smoke ${nonce}`;
    const body = `Delivered notification smoke ${nonce}`;
    await requireTauriValue("desktop_show_system_notification", {
      req: {
        kind: "turn_completed",
        title,
        body,
        workspace_id: `workspace-${nonce}`,
        task_id: `task-${nonce}`,
        session_id: `session-${nonce}`,
      },
    });

    const automationSnapshot = await requireTauriValue("desktop_get_notification_automation_snapshot");
    const deepLink = String(automationSnapshot?.deepLink || "");
    if (!deepLink.includes(`task-${nonce}`)) {
      const summary = summarizeAutomationSnapshot(automationSnapshot, { expectedTaskId: `task-${nonce}` });
      throw new Error(`notification automation snapshot missing expected deep link; summary=${JSON.stringify(summary)}`);
    }

    const delivered = await waitForDeliveredNotification({ deepLink, title });
    await requireTauriValue("desktop_clear_delivered_notification_automation_snapshot", {
      req: {
        identifiers: [delivered.identifier],
      },
    });

    await requireTauriValue("desktop_simulate_last_notification_click");
  });
});
