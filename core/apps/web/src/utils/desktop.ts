export type DesktopConnectionKind = "none" | "local" | "ssh";

export type DesktopConnectionInfo = {
  kind: DesktopConnectionKind;
  base_url?: string | null;
  token?: string | null;
};

export type DesktopDaemonLaunchMode = "host" | "container";

export type DesktopLastConnection =
  | {
      kind: "local";
    }
  | {
      kind: "ssh";
      host: string;
      user?: string | null;
      remote_port: number;
      start_remote?: boolean;
      remote_data_dir?: string | null;
    };

export type DesktopDaemonSettings = {
  auto_start: boolean;
  launch_mode?: DesktopDaemonLaunchMode | null;
  docker_passthrough?: boolean | null;
  last_connection?: DesktopLastConnection | null;
  last_workspace_id?: string | null;
};

export type DesktopConnectLocalOptions = {
  launch_mode?: DesktopDaemonLaunchMode;
  workspace_roots?: string[];
};

export type SshConnectReq = {
  host: string;
  user?: string | null;
  remote_port?: number | null;
  start_remote?: boolean;
  remote_data_dir?: string | null;
};

export type DesktopHttpResponse = {
  status: number;
  body: string;
  content_type?: string | null;
};

export type DesktopDeepLinkToken = {
  token: string;
  expires_at_ms: number;
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

type DesktopTestBridge = {
  isDesktopApp?: boolean;
  invoke?: <T>(cmd: string, args?: any) => Promise<T>;
};

const getDesktopBridge = (): DesktopTestBridge | null => {
  if (typeof window === "undefined") return null;
  return (window as any).__CTX_DESKTOP_BRIDGE__ ?? null;
};

export const isDesktopApp = (): boolean => {
  try {
    const bridge = getDesktopBridge();
    if (bridge?.isDesktopApp) return true;
    return Boolean((window as any).__TAURI_INTERNALS__ || (window as any).__TAURI__);
  } catch {
    return false;
  }
};

// UI-only desktop affordances (e.g. hide the in-app topbar when a native window
// chrome/titlebar exists). This is intentionally broader than `isDesktopApp()`
// so Playwright/WebKit parity runs can opt into desktop UI without requiring
// Tauri APIs.
export const isDesktopUi = (): boolean => {
  try {
    if (isDesktopApp()) return true;
    if (Boolean((window as any).__CTX_DESKTOP_UI__)) return true;
    const params = new URLSearchParams(window.location.search);
    return params.get("desktop_ui") === "1";
  } catch {
    return false;
  }
};

const invoke = async <T>(cmd: string, args?: any): Promise<T> => {
  const bridge = getDesktopBridge();
  if (bridge?.invoke) return bridge.invoke<T>(cmd, args);
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

export const desktopConnectLocal = async (
  opts?: DesktopConnectLocalOptions,
): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_local", opts ? { req: opts } : undefined);

export const desktopConnectSsh = async (req: SshConnectReq): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_ssh", { req });

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

export const desktopGetDaemonSettings = async (): Promise<DesktopDaemonSettings> =>
  invoke<DesktopDaemonSettings>("desktop_get_daemon_settings");

export const desktopUpdateDaemonSettings = async (
  settings: DesktopDaemonSettings,
): Promise<DesktopDaemonSettings> =>
  invoke<DesktopDaemonSettings>("desktop_update_daemon_settings", { settings });

export const desktopSetLastWorkspace = async (workspace_id: string | null): Promise<void> =>
  invoke<void>("desktop_set_last_workspace", { workspace_id });

export const desktopDaemonRequest = async (req: {
  method: string;
  path: string;
  body?: string | null;
  headers?: Array<[string, string]>;
}): Promise<DesktopHttpResponse> =>
  invoke<DesktopHttpResponse>("desktop_daemon_request", { req });

export const desktopUploadBlob = async (args: {
  bytes: number[];
  mime_type: string;
  name?: string | null;
}): Promise<any> =>
  invoke<any>("desktop_upload_blob", args);

export const desktopSetOpenWorkspaces = async (workspace_ids: string[]): Promise<void> =>
  invoke<void>("desktop_set_open_workspaces", { workspace_ids });

export const desktopOpenWorkspaceInNewWindow = async (workspace_id: string): Promise<void> =>
  invoke<void>("desktop_open_workspace_in_new_window", { workspace_id });
