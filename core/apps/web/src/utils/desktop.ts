export type DesktopConnectionKind = "none" | "local" | "ssh";

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
  auth_token?: string | null;
  remote_data_dir?: string | null;
};

export type DesktopHttpResponse = {
  status: number;
  body: string;
  content_type?: string | null;
};

export type DesktopEditorTarget =
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

export type DesktopEditorSettings = {
  target: DesktopEditorTarget;
  custom_command?: string | null;
};

export type DesktopDeepLinkToken = {
  token: string;
  expires_at_ms: number;
};

export type DesktopReadFileResp = {
  path: string;
  text: string;
};

export const isDesktopApp = (): boolean => {
  try {
    const w = window as any;
    return Boolean(w?.isTauri || w?.__TAURI__?.invoke || w?.__TAURI__?.tauri?.invoke);
  } catch {
    return false;
  }
};

const invoke = async <T>(cmd: string, args?: any): Promise<T> => {
  const mod = await import("@tauri-apps/api/core");
  return mod.invoke<T>(cmd, args);
};

export const desktopGetConnection = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_get_connection");

export const desktopDisconnect = async (): Promise<void> =>
  invoke<void>("desktop_disconnect");

export const desktopConnectLocal = async (): Promise<DesktopConnectionInfo> =>
  invoke<DesktopConnectionInfo>("desktop_connect_local");

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

export const desktopGetEditorSettings = async (): Promise<DesktopEditorSettings> =>
  invoke<DesktopEditorSettings>("desktop_get_editor_settings");

export const desktopUpdateEditorSettings = async (
  settings: DesktopEditorSettings,
): Promise<DesktopEditorSettings> =>
  invoke<DesktopEditorSettings>("desktop_update_editor_settings", { settings });

export const desktopOpenFile = async (args: {
  worktree_id: string;
  path: string;
  line?: number | null;
  col?: number | null;
}): Promise<void> =>
  invoke<void>("desktop_open_file", args);

export const desktopOpenPath = async (args: {
  path: string;
  line?: number | null;
  col?: number | null;
}): Promise<void> =>
  invoke<void>("desktop_open_path", args);

export const desktopReadFile = async (args: { path: string }): Promise<DesktopReadFileResp> =>
  invoke<DesktopReadFileResp>("desktop_read_file", args);

export const desktopGetDeepLinkToken = async (): Promise<DesktopDeepLinkToken> =>
  invoke<DesktopDeepLinkToken>("desktop_get_deep_link_token");

export const desktopRegisterWorkspaceWindow = async (args: {
  workspace_id: string;
  window_label: string;
}): Promise<void> =>
  invoke<void>("desktop_register_workspace_window", args);

export const desktopUnregisterWorkspaceWindow = async (args: { window_label: string }): Promise<void> =>
  invoke<void>("desktop_unregister_workspace_window", args);
