export type DesktopConnectionKind = "none" | "local" | "ssh";

export type DesktopPlatform = "macos" | "windows" | "linux" | "unknown";

export type DesktopTitlebarColor = {
  r: number;
  g: number;
  b: number;
  a?: number;
};

export type DesktopConnectionInfo = {
  kind: DesktopConnectionKind;
  base_url?: string | null;
  token?: string | null;
};

export type SshConnectReq = {
  host: string;
  user?: string | null;
  remote_port?: number | null;
  start_remote?: boolean;
  remote_data_dir?: string | null;
  remote_ctx_bin?: string | null;
};

export type DesktopHttpResponse = {
  status: number;
  body: string;
  content_type?: string | null;
};

export type DesktopStorageBatchOp =
  | { kind: "set"; key: string; value: unknown }
  | { kind: "delete"; key: string };

export type DesktopDeepLinkToken = {
  token: string;
  expires_at_ms: number;
};

export type DesktopStorageNotice =
  | {
      kind: "ui_state_reset";
      reason: "schema_mismatch" | "invalid_ui_state_db";
    };

export type DesktopSshHost = {
  host: string;
  user?: string | null;
  host_name?: string | null;
  port?: number | null;
};

export type DesktopSshPathEntry = {
  name: string;
  path: string;
};

export type DesktopOpenFileReq = {
  worktree_id: string;
  path: string;
  line?: number | null;
  col?: number | null;
};

export type DesktopOpenPathReq = {
  path: string;
  line?: number | null;
  col?: number | null;
};

export type DesktopEditorSettings = {
  target:
    | "system"
    | "vscode"
    | "vscode_insiders"
    | "cursor"
    | "windsurf"
    | "antigravity"
    | "idea"
    | "pycharm"
    | "xcode"
    | "android_studio"
    | "custom";
  custom_command?: string | null;
  remote_authority?: string | null;
};

export type DesktopCodexLoginRelayReq = {
  login_id: string;
  callback_url: string;
  completion_token: string;
};

export type DesktopMenuItemStateUpdate = {
  id: string;
  enabled?: boolean;
  checked?: boolean;
};

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
  return mod.invoke<T>(cmd, args);
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

export const desktopGetConnection = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_get_connection");

export const desktopDisconnect = async (): Promise<void> =>
  invoke<void>("desktop_disconnect");

export const desktopConnectLocal = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_local");

export const desktopConnectSsh = async (req: SshConnectReq): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_ssh", { req });

export const desktopListSshHosts = async (): Promise<DesktopSshHost[]> =>
  invoke<DesktopSshHost[]>("desktop_list_ssh_hosts");

export const desktopTestSsh = async (req: { host: string; user?: string | null }): Promise<void> =>
  invoke<void>("desktop_test_ssh", { req });

export const desktopKickoffRemotePrewarm = async (req: {
  host: string;
  user?: string | null;
  remote_port?: number | null;
  remote_data_dir?: string | null;
}): Promise<void> =>
  invoke<void>("desktop_kickoff_remote_prewarm", { req });

export const desktopListSshPaths = async (req: { host: string; user?: string | null; path?: string | null }): Promise<DesktopSshPathEntry[]> =>
  invoke<DesktopSshPathEntry[]>("desktop_list_ssh_paths", { req });

export const desktopGetGitBranch = async (req: { path: string }): Promise<string | null> =>
  invoke<string | null>("desktop_get_git_branch", { req });

export const desktopPickFolder = async (): Promise<string | null> =>
  invoke<string | null>("desktop_pick_folder");

export const desktopGitClone = async (repo_url: string, dest_parent: string): Promise<string> =>
  invoke<string>("desktop_git_clone", { repo_url, dest_parent });

export const desktopSaveTextFile = async (args: {
  suggested_name?: string | null;
  contents: string;
}): Promise<string | null> =>
  invoke<string | null>("desktop_save_text_file", args);

export const desktopReadFile = async (args: {
  path: string;
  line?: number | null;
  col?: number | null;
}): Promise<{ path: string; text: string }> =>
  invoke<{ path: string; text: string }>("desktop_read_file", args);

export const desktopGetDeepLinkToken = async (): Promise<DesktopDeepLinkToken> =>
  invoke<DesktopDeepLinkToken>("desktop_get_deep_link_token");

export const desktopGetVersion = async (): Promise<string> => {
  const mod = await import("@tauri-apps/api/app");
  return mod.getVersion();
};

export const desktopOpenFile = async (req: DesktopOpenFileReq): Promise<void> =>
  invoke<void>("desktop_open_file", { req });

export const desktopOpenPath = async (req: DesktopOpenPathReq): Promise<void> =>
  invoke<void>("desktop_open_path", { req });

export const desktopGetEditorSettings = async (): Promise<DesktopEditorSettings> =>
  invoke<DesktopEditorSettings>("desktop_get_editor_settings");

export const desktopUpdateEditorSettings = async (
  settings: DesktopEditorSettings,
): Promise<DesktopEditorSettings> =>
  invoke<DesktopEditorSettings>("desktop_update_editor_settings", { settings });

export const desktopDaemonRequest = async (req: {
  method: string;
  path: string;
  body?: string | null;
  headers?: Array<[string, string]>;
}): Promise<DesktopHttpResponse> =>
  invoke<DesktopHttpResponse>("desktop_daemon_request", { req });

export const desktopStartCodexLoginRelay = async (req: DesktopCodexLoginRelayReq): Promise<boolean> =>
  invoke<boolean>("desktop_start_codex_login_relay", { req });

export const desktopStorageGet = async (key: string): Promise<unknown | null> =>
  invoke<unknown | null>("desktop_storage_get", { key });

export const desktopStorageBatch = async (ops: DesktopStorageBatchOp[]): Promise<void> =>
  invoke<void>("desktop_storage_batch", { ops });

export const desktopStorageConsumeNotice = async (): Promise<DesktopStorageNotice | null> =>
  invoke<DesktopStorageNotice | null>("desktop_storage_consume_notice");

export const desktopUploadBlob = async (args: {
  bytes: number[];
  mime_type: string;
  name?: string | null;
}): Promise<unknown> =>
  invoke<unknown>("desktop_upload_blob", args);

export const desktopSetOpenWorkspaces = async (workspace_ids: string[]): Promise<void> =>
  invoke<void>("desktop_set_open_workspaces", { workspace_ids });

export const desktopOpenWorkspaceInNewWindow = async (workspace_id: string): Promise<void> =>
  invoke<void>("desktop_open_workspace_in_new_window", { workspace_id });

export const desktopSetTitlebarColor = async (color: DesktopTitlebarColor): Promise<void> =>
  invoke<void>("desktop_set_titlebar_color", { color });

export const desktopSetMenuState = async (items: DesktopMenuItemStateUpdate[]): Promise<void> =>
  invoke<void>("desktop_set_menu_state", { items });
