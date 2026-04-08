import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

const TAURI_GLOBALS = {
  __TAURI__: {},
};

describe("desktop request envelopes", () => {
  beforeEach(() => {
    vi.resetModules();
    invokeMock.mockReset();
    Object.assign(globalThis, TAURI_GLOBALS);
  });

  it("wraps raw desktop commands in req payloads", async () => {
    invokeMock.mockResolvedValue(undefined);

    const desktop = await import("./desktop");

    await desktop.desktopGitClone("https://example.com/repo.git", "/tmp/workspaces");
    await desktop.desktopSaveTextFile({ suggested_name: "notes.md", contents: "hello" });
    await desktop.desktopStorageGet("theme");
    await desktop.desktopStorageBatch([{ kind: "delete", key: "theme" }]);
    await desktop.desktopSetOpenWorkspaces(["ws-1", "ws-2"]);
    await desktop.desktopOpenWorkspaceInNewWindow("ws-3");
    await desktop.desktopSetDockRecentLocalWorkspaces([{ label: "Workspace", root_path: "/tmp/ws" }]);
    await desktop.desktopRecordWorkspaceVisit("ws-4", "Workspace 4");
    await desktop.desktopSetTitlebarColor({ r: 1, g: 2, b: 3 });
    await desktop.desktopSetMenuState([{ id: "task.new", enabled: true }]);
    await desktop.desktopSetWindowTitle("ctx");
    await desktop.desktopUpdateEditorSettings({ target: "cursor" });

    expect(invokeMock.mock.calls).toEqual([
      ["desktop_git_clone", { req: { repo_url: "https://example.com/repo.git", dest_parent: "/tmp/workspaces" } }],
      ["desktop_save_text_file", { req: { suggested_name: "notes.md", contents: "hello" } }],
      ["desktop_storage_get", { req: { key: "theme" } }],
      ["desktop_storage_batch", { req: { ops: [{ kind: "delete", key: "theme" }] } }],
      ["desktop_set_open_workspaces", { req: { workspace_ids: ["ws-1", "ws-2"] } }],
      ["desktop_open_workspace_in_new_window", { req: { workspace_id: "ws-3" } }],
      ["desktop_set_dock_recent_local_workspaces", { req: { entries: [{ label: "Workspace", root_path: "/tmp/ws" }] } }],
      ["desktop_record_workspace_visit", { req: { workspace_id: "ws-4", workspace_label: "Workspace 4" } }],
      ["desktop_set_titlebar_color", { req: { r: 1, g: 2, b: 3 } }],
      ["desktop_set_menu_state", { req: { items: [{ id: "task.new", enabled: true }] } }],
      ["desktop_set_window_title", { req: { title: "ctx" } }],
      ["desktop_update_editor_settings", { req: { target: "cursor" } }],
    ]);
  });
});
