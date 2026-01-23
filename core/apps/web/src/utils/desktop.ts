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
  remote_data_dir?: string | null;
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

export const isDesktopApp = (): boolean => {
  try {
    const g = globalThis as any;
    return Boolean(g?.__TAURI_INTERNALS__ || g?.__TAURI__);
  } catch {
    return false;
  }
};

const invoke = async <T>(cmd: string, args?: any): Promise<T> => {
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

export const desktopDaemonRequest = async (req: {
  method: string;
  path: string;
  body?: string | null;
  headers?: Array<[string, string]>;
}): Promise<DesktopHttpResponse> =>
  invoke<DesktopHttpResponse>("desktop_daemon_request", { req });

export const desktopStorageGet = async (key: string): Promise<unknown | null> =>
  invoke<unknown | null>("desktop_storage_get", { key });

export const desktopStorageBatch = async (ops: DesktopStorageBatchOp[]): Promise<void> =>
  invoke<void>("desktop_storage_batch", { ops });

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
