import type {
  BlobUploadResp,
  DesktopAppRestartResp,
  DesktopAppUpdateApplyReq,
  DesktopAppUpdateApplyResp,
  DesktopAppUpdateAttemptResp,
  DesktopAppUpdateCheckReq,
  DesktopAppUpdateCheckResp,
  DesktopAppUpdateStateResp,
  DesktopCodexLoginRelayReq,
  DesktopConnectionInfo,
  DesktopConnectionIntent,
  DesktopDaemonRequest,
  DesktopDeepLinkToken,
  DesktopDockRecentLocalWorkspace,
  DesktopEditorSettings,
  DesktopGitBranchReq,
  DesktopGitCloneReq,
  DesktopHttpResponse,
  DesktopLinuxSandboxEnsureResp,
  DesktopLocalLinuxSandboxEnsureReq,
  DesktopMenuItemStateUpdate,
  DesktopNotificationKind,
  DesktopNotificationPermission,
  DesktopOpenFileReq,
  DesktopOpenPathReq,
  DesktopOpenWorkspaceInNewWindowReq,
  DesktopReadBinaryFileResp,
  DesktopReadFileResp,
  DesktopRecordWorkspaceVisitReq,
  DesktopRemoteDaemonUpdateReq,
  DesktopRemoteDaemonUpdateResp,
  DesktopRemotePrewarmReq,
  DesktopRemoteLinuxSandboxEnsureReq,
  DesktopRestartLocalDaemonReq,
  DesktopSaveTextFileReq,
  DesktopSetDockRecentLocalWorkspacesReq,
  DesktopSetMenuStateReq,
  DesktopSetOpenWorkspacesReq,
  DesktopSetWindowTitleReq,
  DesktopSshConnectJobStatus,
  DesktopSshConnectPollReq,
  DesktopSshHost,
  DesktopSshPathEntry,
  DesktopSshPathReq,
  DesktopSshTestReq,
  DesktopStorageBatchOp,
  DesktopStorageBatchReq,
  DesktopStorageGetReq,
  DesktopStorageNotice,
  DesktopWebviewRecoveryAutomationSnapshot,
  DesktopWebviewRecoveryFaultReq,
  DesktopWebviewRecoveryHeartbeatReq,
  DesktopWebviewRecoveryIncident,
  DesktopSyncWorkspaceAttentionReq,
  DesktopTitlebarColor,
  DesktopUploadBlobReq,
  DesktopShowSystemNotificationReq,
  SshConnectReq,
} from "../generated/desktop-ipc";

export type {
  BlobUploadResp,
  DesktopAppRestartResp,
  DesktopAppUpdateApplyResp,
  DesktopAppUpdateAttemptResp,
  DesktopAppUpdateAttemptStageResp,
  DesktopAppUpdateCheckResp,
  DesktopAppUpdateStateResp,
  DesktopCodexLoginRelayReq,
  DesktopConnectionInfo,
  DesktopConnectionIntent,
  DesktopConnectionKind,
  DesktopDeepLinkToken,
  DesktopDockRecentLocalWorkspace,
  DesktopEditorSettings,
  DesktopHttpResponse,
  DesktopLinuxSandboxEnsureResp,
  DesktopMenuItemStateUpdate,
  DesktopNotificationKind,
  DesktopNotificationPermission,
  DesktopOpenFileReq,
  DesktopOpenPathReq,
  DesktopReadBinaryFileResp,
  DesktopStorageBatchOp,
  DesktopStorageNotice,
  DesktopWebviewRecoveryAutomationSnapshot,
  DesktopWebviewRecoveryIncident,
  DesktopSshHost,
  DesktopSshPathEntry,
  DesktopTitlebarColor,
  SshConnectReq,
} from "../generated/desktop-ipc";

export type DesktopPlatform = "macos" | "windows" | "linux" | "unknown";

export type DesktopDragDropPosition = {
  x: number;
  y: number;
};

export type DesktopPhysicalSize = {
  width: number;
  height: number;
};

export type DesktopViewGeometry = {
  scaleFactor: number;
  devicePixelRatio: number;
  webviewPosition: DesktopDragDropPosition;
  webviewSize: DesktopPhysicalSize;
  windowInnerPosition: DesktopDragDropPosition;
  windowOuterPosition: DesktopDragDropPosition;
  windowInnerSize: DesktopPhysicalSize;
  windowOuterSize: DesktopPhysicalSize;
  screenWidth: number;
  screenHeight: number;
  innerWidth: number;
  innerHeight: number;
};

export type DesktopDragDropEvent =
  | {
      type: "enter";
      paths: string[];
      position: DesktopDragDropPosition;
    }
  | {
      type: "over";
      position: DesktopDragDropPosition;
    }
  | {
      type: "drop";
      paths: string[];
      position: DesktopDragDropPosition;
    }
  | {
      type: "leave";
    };

const DESKTOP_DRAG_DROP_TEST_EVENT = "ctx:desktop-drag-drop-test";

type TauriGlobals = {
  __TAURI_INTERNALS__?: unknown;
  __TAURI__?: unknown;
};

export const isDesktopApp = (): boolean => {
  try {
    const g = globalThis as typeof globalThis & TauriGlobals;
    return Boolean(g.__TAURI_INTERNALS__ || g.__TAURI__);
  } catch {
    return false;
  }
};

export const getDesktopPlatform = async (): Promise<DesktopPlatform> => {
  if (!isDesktopApp()) return "unknown";
  try {
    const mod = await import("@tauri-apps/plugin-os");
    const value = await mod.platform();
    switch (value) {
      case "macos":
        return "macos";
      case "windows":
        return "windows";
      case "linux":
        return "linux";
      default:
        return "unknown";
    }
  } catch {
    return "unknown";
  }
};

export const openExternalLink = async (href: string): Promise<boolean> => {
  if (!href) return false;
  if (isDesktopApp()) {
    try {
      const mod = await import("@tauri-apps/plugin-shell");
      await mod.open(href);
      return true;
    } catch {
      return false;
    }
  }
  try {
    const win = window.open(href, "_blank", "noopener,noreferrer");
    return Boolean(win);
  } catch {
    return false;
  }
};

const invoke = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
  const mod = await import("@tauri-apps/api/core");
  try {
    return await mod.invoke<T>(cmd, args);
  } catch (err: unknown) {
    if (err instanceof Error) throw err;
    if (typeof err === "string") throw new Error(err.trim() || `desktop invoke failed: ${cmd}`);
    if (err && typeof err === "object") {
      const withMessage = err as { message?: unknown };
      if (typeof withMessage.message === "string") {
        const message = withMessage.message.trim();
        if (message) throw new Error(message);
      }
      try {
        const encoded = JSON.stringify(err);
        if (encoded.trim()) throw new Error(encoded);
      } catch {
        // ignore serialization failures
      }
    }
    throw new Error(`desktop invoke failed: ${cmd}`);
  }
};

const invokeDesktopReq = async <TReq, TResp>(cmd: string, req: TReq): Promise<TResp> =>
  invoke<TResp>(cmd, { req });

const DESKTOP_SSH_CONNECT_POLL_MS = 500;
const DESKTOP_SSH_CONNECT_TIMEOUT_MS = 4 * 60_000;

const sleep = (ms: number) =>
  new Promise<void>((resolve) => {
    globalThis.setTimeout(resolve, ms);
  });

const consumeDesktopSshConnectJob = async (jobId: string) => {
  try {
    const req: DesktopSshConnectPollReq = { job_id: jobId, consume: true };
    await invokeDesktopReq<DesktopSshConnectPollReq, DesktopSshConnectJobStatus>(
      "desktop_connect_ssh_poll",
      req,
    );
  } catch {
    // Ignore cleanup failures so the primary connect result surfaces cleanly.
  }
};

export const desktopListen = async <T>(event: string, handler: (payload: T) => void): Promise<() => void> => {
  const mod = await import("@tauri-apps/api/event");
  const unlisten = await mod.listen<T>(event, (e) => handler(e.payload));
  return () => {
    try {
      unlisten();
    } catch {
      // ignore
    }
  };
};

export const desktopListenForDragDrop = async (
  handler: (event: DesktopDragDropEvent) => void,
): Promise<(() => void) | null> => {
  const cleanup = new Set<() => void>();
  const onTestEvent = (event: Event) => {
    if (!(event instanceof CustomEvent)) return;
    handler(event.detail as DesktopDragDropEvent);
  };
  window.addEventListener(DESKTOP_DRAG_DROP_TEST_EVENT, onTestEvent as EventListener);
  cleanup.add(() => {
    window.removeEventListener(DESKTOP_DRAG_DROP_TEST_EVENT, onTestEvent as EventListener);
  });
  try {
    const unlisten = await desktopListen<DesktopDragDropEvent>(DESKTOP_DRAG_DROP_TEST_EVENT, handler);
    cleanup.add(() => {
      try {
        unlisten();
      } catch {
        // ignore
      }
    });
  } catch {
    // Ignore missing event-bus support in environments that only expose DOM test events.
  }
  try {
    const mod = await import("@tauri-apps/api/webview");
    const unlisten = await mod.getCurrentWebview().onDragDropEvent((event) => handler(event.payload as DesktopDragDropEvent));
    cleanup.add(() => {
      try {
        unlisten();
      } catch {
        // ignore
      }
    });
  } catch {
    // Ignore missing Tauri drag/drop support in environments that don't expose the desktop webview API.
  }
  return () => {
    for (const dispose of cleanup) {
      try {
        dispose();
      } catch {
        // ignore
      }
    }
  };
};

export const desktopGetViewGeometry = async (): Promise<DesktopViewGeometry> => {
  if (!isDesktopApp()) {
    throw new Error("desktop view geometry is only available inside the desktop app");
  }
  const [webviewMod, windowMod] = await Promise.all([
    import("@tauri-apps/api/webview"),
    import("@tauri-apps/api/window"),
  ]);
  const webview = webviewMod.getCurrentWebview();
  const currentWindow = windowMod.getCurrentWindow();
  const [
    webviewPosition,
    webviewSize,
    windowInnerPosition,
    windowOuterPosition,
    windowInnerSize,
    windowOuterSize,
    scaleFactor,
  ] = await Promise.all([
    webview.position(),
    webview.size(),
    currentWindow.innerPosition(),
    currentWindow.outerPosition(),
    currentWindow.innerSize(),
    currentWindow.outerSize(),
    currentWindow.scaleFactor(),
  ]);
  return {
    scaleFactor,
    devicePixelRatio: window.devicePixelRatio > 0 ? window.devicePixelRatio : 1,
    webviewPosition: {
      x: webviewPosition.x,
      y: webviewPosition.y,
    },
    webviewSize: {
      width: webviewSize.width,
      height: webviewSize.height,
    },
    windowInnerPosition: {
      x: windowInnerPosition.x,
      y: windowInnerPosition.y,
    },
    windowOuterPosition: {
      x: windowOuterPosition.x,
      y: windowOuterPosition.y,
    },
    windowInnerSize: {
      width: windowInnerSize.width,
      height: windowInnerSize.height,
    },
    windowOuterSize: {
      width: windowOuterSize.width,
      height: windowOuterSize.height,
    },
    screenWidth: window.screen.width,
    screenHeight: window.screen.height,
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
  };
};

export const desktopGetConnection = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_get_connection");

export const desktopDisconnect = async (): Promise<void> =>
  invoke<void>("desktop_disconnect");

export const desktopConnectLocal = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_local");

export const desktopRestartLocalDaemon = async (): Promise<DesktopConnectionInfo> =>
  invokeDesktopReq<DesktopRestartLocalDaemonReq, DesktopConnectionInfo>(
    "desktop_restart_local_daemon",
    { confirm: true },
  );

export const desktopConnectSsh = async (req: SshConnectReq): Promise<DesktopConnectionInfo> => {
  const jobId = String(await invokeDesktopReq<SshConnectReq, string>("desktop_connect_ssh_begin", req)).trim();
  if (!jobId) {
    throw new Error("desktop_connect_ssh_begin returned empty job id");
  }

  const startedAt = Date.now();
  while (Date.now() - startedAt < DESKTOP_SSH_CONNECT_TIMEOUT_MS) {
    const snapshot = await invokeDesktopReq<DesktopSshConnectPollReq, DesktopSshConnectJobStatus>(
      "desktop_connect_ssh_poll",
      { job_id: jobId, consume: false },
    );
    const status = String(snapshot.status || "").trim().toLowerCase();
    if (status === "succeeded") {
      await consumeDesktopSshConnectJob(jobId);
      if (!snapshot.info) {
        throw new Error("desktop_connect_ssh succeeded without connection info");
      }
      return snapshot.info;
    }
    if (status === "failed") {
      await consumeDesktopSshConnectJob(jobId);
      throw new Error(String(snapshot.error || "desktop_connect_ssh failed"));
    }
    await sleep(DESKTOP_SSH_CONNECT_POLL_MS);
  }

  await consumeDesktopSshConnectJob(jobId);
  throw new Error("desktop_connect_ssh timed out waiting for completion");
};

export const desktopUpdateRemoteDaemon = async (channel?: string): Promise<DesktopRemoteDaemonUpdateResp> =>
  invokeDesktopReq<DesktopRemoteDaemonUpdateReq, DesktopRemoteDaemonUpdateResp>(
    "desktop_update_remote_daemon",
    {
      confirm: true,
      ...(channel ? { channel } : {}),
    },
  );

export const desktopCheckAppUpdate = async (channel?: string): Promise<DesktopAppUpdateCheckResp> =>
  invokeDesktopReq<DesktopAppUpdateCheckReq, DesktopAppUpdateCheckResp>(
    "desktop_check_app_update",
    channel ? { channel } : {},
  );

export const desktopGetAppUpdateState = async (channel?: string): Promise<DesktopAppUpdateStateResp> =>
  invokeDesktopReq<DesktopAppUpdateCheckReq, DesktopAppUpdateStateResp>(
    "desktop_get_app_update_state",
    channel ? { channel } : {},
  );

export const desktopApplyAppUpdate = async (
  channel?: string,
  downloadId?: string,
): Promise<DesktopAppUpdateApplyResp> =>
  invokeDesktopReq<DesktopAppUpdateApplyReq, DesktopAppUpdateApplyResp>(
    "desktop_apply_app_update",
    {
      confirm: true,
      ...(channel ? { channel } : {}),
      ...(downloadId ? { download_id: downloadId } : {}),
    },
  );

export const desktopRestartApp = async (): Promise<DesktopAppRestartResp> =>
  invoke<DesktopAppRestartResp>("desktop_restart_app");

export const desktopGetLastAppUpdateAttempt = async (): Promise<DesktopAppUpdateAttemptResp | null> =>
  invoke<DesktopAppUpdateAttemptResp | null>("desktop_get_last_app_update_attempt");

export const desktopListSshHosts = async (): Promise<DesktopSshHost[]> =>
  invoke<DesktopSshHost[]>("desktop_list_ssh_hosts");

export const desktopTestSsh = async (req: { host: string; user?: string | null; password_once?: string | null }): Promise<void> =>
  invokeDesktopReq<DesktopSshTestReq, void>("desktop_test_ssh", req);

export const desktopKickoffRemotePrewarm = async (req: DesktopRemotePrewarmReq): Promise<void> =>
  invokeDesktopReq<DesktopRemotePrewarmReq, void>("desktop_kickoff_remote_prewarm", req);

export const desktopEnsureLocalLinuxSandboxReady = async (
  req?: DesktopLocalLinuxSandboxEnsureReq,
): Promise<DesktopLinuxSandboxEnsureResp> =>
  invokeDesktopReq<DesktopLocalLinuxSandboxEnsureReq, DesktopLinuxSandboxEnsureResp>(
    "desktop_ensure_local_linux_sandbox_ready",
    req ?? {},
  );

export const desktopEnsureRemoteLinuxSandboxReady = async (
  req?: DesktopRemoteLinuxSandboxEnsureReq,
): Promise<DesktopLinuxSandboxEnsureResp> =>
  invokeDesktopReq<DesktopRemoteLinuxSandboxEnsureReq, DesktopLinuxSandboxEnsureResp>(
    "desktop_ensure_remote_linux_sandbox_ready",
    req ?? {},
  );

export const desktopListSshPaths = async (req: DesktopSshPathReq): Promise<DesktopSshPathEntry[]> =>
  invokeDesktopReq<DesktopSshPathReq, DesktopSshPathEntry[]>("desktop_list_ssh_paths", req);

export const desktopGetGitBranch = async (req: DesktopGitBranchReq): Promise<string | null> =>
  invokeDesktopReq<DesktopGitBranchReq, string | null>("desktop_get_git_branch", req);

export const desktopPickFolder = async (): Promise<string | null> =>
  invoke<string | null>("desktop_pick_folder");

export const desktopGitClone = async (repo_url: string, dest_parent: string): Promise<string> =>
  invokeDesktopReq<DesktopGitCloneReq, string>("desktop_git_clone", { repo_url, dest_parent });

export const desktopSaveTextFile = async (args: DesktopSaveTextFileReq): Promise<string | null> =>
  invokeDesktopReq<DesktopSaveTextFileReq, string | null>("desktop_save_text_file", args);

export const desktopReadFile = async (args: DesktopOpenPathReq): Promise<DesktopReadFileResp> =>
  invokeDesktopReq<DesktopOpenPathReq, DesktopReadFileResp>("desktop_read_file", args);

export const desktopReadBinaryFile = async (args: DesktopOpenPathReq): Promise<DesktopReadBinaryFileResp> =>
  invokeDesktopReq<DesktopOpenPathReq, DesktopReadBinaryFileResp>(
    "desktop_read_binary_file",
    args,
  );

export const desktopGetDeepLinkToken = async (): Promise<DesktopDeepLinkToken> =>
  invoke<DesktopDeepLinkToken>("desktop_get_deep_link_token");

export const desktopGetVersion = async (): Promise<string> => {
  const mod = await import("@tauri-apps/api/app");
  return mod.getVersion();
};

export const desktopOpenFile = async (req: DesktopOpenFileReq): Promise<void> =>
  invokeDesktopReq<DesktopOpenFileReq, void>("desktop_open_file", req);

export const desktopOpenPath = async (req: DesktopOpenPathReq): Promise<void> =>
  invokeDesktopReq<DesktopOpenPathReq, void>("desktop_open_path", req);

export const desktopGetEditorSettings = async (): Promise<DesktopEditorSettings> =>
  invoke<DesktopEditorSettings>("desktop_get_editor_settings");

export const desktopUpdateEditorSettings = async (
  settings: DesktopEditorSettings,
): Promise<DesktopEditorSettings> =>
  invokeDesktopReq<DesktopEditorSettings, DesktopEditorSettings>(
    "desktop_update_editor_settings",
    settings,
  );

export const desktopDaemonRequest = async (req: DesktopDaemonRequest): Promise<DesktopHttpResponse> =>
  invokeDesktopReq<DesktopDaemonRequest, DesktopHttpResponse>("desktop_daemon_request", req);

export const desktopStartCodexLoginRelay = async (req: DesktopCodexLoginRelayReq): Promise<boolean> =>
  invokeDesktopReq<DesktopCodexLoginRelayReq, boolean>("desktop_start_codex_login_relay", req);

export const desktopStorageGet = async (key: string): Promise<unknown | null> =>
  invokeDesktopReq<DesktopStorageGetReq, unknown | null>("desktop_storage_get", { key });

export const desktopStorageBatch = async (ops: DesktopStorageBatchOp[]): Promise<void> =>
  invokeDesktopReq<DesktopStorageBatchReq, void>("desktop_storage_batch", { ops });

export const desktopStorageConsumeNotice = async (): Promise<DesktopStorageNotice | null> =>
  invoke<DesktopStorageNotice | null>("desktop_storage_consume_notice");

export const desktopWebviewRecoveryHeartbeat = async (
  req: DesktopWebviewRecoveryHeartbeatReq,
): Promise<void> =>
  invokeDesktopReq<DesktopWebviewRecoveryHeartbeatReq, void>(
    "desktop_webview_recovery_heartbeat",
    req,
  );

export const desktopWebviewRecoveryConsumeIncidents = async (): Promise<DesktopWebviewRecoveryIncident[]> =>
  invoke<DesktopWebviewRecoveryIncident[]>("desktop_webview_recovery_consume_incidents");

export const desktopTriggerWebviewRecoveryFault = async (
  req: DesktopWebviewRecoveryFaultReq,
): Promise<void> =>
  invokeDesktopReq<DesktopWebviewRecoveryFaultReq, void>(
    "desktop_trigger_webview_recovery_fault",
    req,
  );

export const desktopGetWebviewRecoveryAutomationSnapshot = async (): Promise<DesktopWebviewRecoveryAutomationSnapshot> =>
  invoke<DesktopWebviewRecoveryAutomationSnapshot>(
    "desktop_get_webview_recovery_automation_snapshot",
  );

export const desktopUploadBlob = async (args: DesktopUploadBlobReq): Promise<BlobUploadResp> =>
  invokeDesktopReq<DesktopUploadBlobReq, BlobUploadResp>("desktop_upload_blob", args);

export const desktopSetOpenWorkspaces = async (workspace_ids: string[]): Promise<void> =>
  invokeDesktopReq<DesktopSetOpenWorkspacesReq, void>("desktop_set_open_workspaces", {
    workspace_ids,
  });

export const desktopOpenLauncherInNewWindow = async (): Promise<void> =>
  invoke<void>("desktop_open_launcher_in_new_window");

export const desktopOpenWorkspaceInNewWindow = async (workspace_id: string): Promise<void> =>
  invokeDesktopReq<DesktopOpenWorkspaceInNewWindowReq, void>(
    "desktop_open_workspace_in_new_window",
    { workspace_id },
  );

export const desktopOpenWorkspaceSetupInNewWindow = async (): Promise<void> =>
  invoke<void>("desktop_open_workspace_setup_in_new_window");

export const desktopSetDockRecentLocalWorkspaces = async (
  entries: DesktopDockRecentLocalWorkspace[],
): Promise<void> =>
  invokeDesktopReq<DesktopSetDockRecentLocalWorkspacesReq, void>(
    "desktop_set_dock_recent_local_workspaces",
    { entries },
  );

export const desktopRecordWorkspaceVisit = async (
  workspace_id: string,
  workspace_label: string,
): Promise<void> => {
  if (!isDesktopApp()) return;
  const req: DesktopRecordWorkspaceVisitReq = { workspace_id, workspace_label };
  await invokeDesktopReq<DesktopRecordWorkspaceVisitReq, void>(
    "desktop_record_workspace_visit",
    req,
  );
};

export const desktopSetTitlebarColor = async (color: DesktopTitlebarColor): Promise<void> =>
  invokeDesktopReq<DesktopTitlebarColor, void>("desktop_set_titlebar_color", color);

export const desktopSetMenuState = async (items: DesktopMenuItemStateUpdate[]): Promise<void> =>
  invokeDesktopReq<DesktopSetMenuStateReq, void>("desktop_set_menu_state", { items });

export const desktopSetWindowTitle = async (title: string): Promise<void> => {
  if (!isDesktopApp()) return;
  const req: DesktopSetWindowTitleReq = { title };
  await invokeDesktopReq<DesktopSetWindowTitleReq, void>("desktop_set_window_title", req);
};

export const desktopGetNotificationPermission = async (): Promise<DesktopNotificationPermission> =>
  invoke<DesktopNotificationPermission>("desktop_get_notification_permission");

export const desktopRequestNotificationPermission = async (): Promise<DesktopNotificationPermission> =>
  invoke<DesktopNotificationPermission>("desktop_request_notification_permission");

export const desktopShowSystemNotification = async (
  req: DesktopShowSystemNotificationReq,
): Promise<void> =>
  invokeDesktopReq<DesktopShowSystemNotificationReq, void>(
    "desktop_show_system_notification",
    req,
  );

export const desktopSyncWorkspaceAttention = async (
  req: DesktopSyncWorkspaceAttentionReq,
): Promise<void> =>
  invokeDesktopReq<DesktopSyncWorkspaceAttentionReq, void>(
    "desktop_sync_workspace_attention",
    req,
  );

export const desktopClearWindowAttention = async (): Promise<void> =>
  invoke<void>("desktop_clear_window_attention");
