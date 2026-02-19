import type { ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, test, vi } from "vitest";
import App from "./App";
import { WEB_MENU_COMMAND_EVENT } from "./utils/desktopMenuCommands";
import {
  desktopListen,
  desktopSetMenuState,
  desktopSetTitlebarColor,
  desktopSetWindowTitle,
  getDesktopPlatform,
  isDesktopApp,
  openExternalLink,
} from "./utils/desktop";

const desktopHandlers = new Map<string, (payload?: unknown) => void>();

vi.mock("./api/client", () => ({
  appendDesktopLog: vi.fn(async () => {}),
}));

vi.mock("./components/DaemonAvailabilityOverlay", () => ({
  __esModule: true,
  default: () => null,
}));

vi.mock("./pages/LauncherPage", () => ({
  __esModule: true,
  default: () => <div>New Workspace</div>,
}));

vi.mock("./pages/AppSettingsPage", () => ({
  __esModule: true,
  default: () => <div>App Settings Screen</div>,
}));

vi.mock("./pages/WorkspacesPage", () => ({
  __esModule: true,
  default: () => <div>Workspaces Screen</div>,
}));

vi.mock("./pages/WorkbenchPage", () => ({
  __esModule: true,
  default: () => <div>Workbench Screen</div>,
}));

vi.mock("./pages/CursorDiffDemoPage", () => ({
  __esModule: true,
  default: () => <div>Diff Demo Screen</div>,
}));

vi.mock("./pages/ProvidersPage", () => ({
  __esModule: true,
  default: () => <div>Providers Screen</div>,
}));

vi.mock("./pages/DiagnosticsPage", () => ({
  __esModule: true,
  default: () => <div>Diagnostics Screen</div>,
}));

vi.mock("./pages/SettingsPage", () => ({
  __esModule: true,
  default: () => <div>Settings Screen</div>,
}));

vi.mock("./pages/WorkspaceSetupPage", () => ({
  __esModule: true,
  default: () => <div>Workspace Setup Screen</div>,
}));

vi.mock("./pages/CrashCoursePage", () => ({
  __esModule: true,
  default: () => <div>Crash Course Screen</div>,
}));

vi.mock("./state/sessionSupervisor", () => ({
  SessionSupervisorProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock("./state/settingsStore", () => ({
  SettingsStoreProvider: ({ children }: { children: ReactNode }) => <>{children}</>,
  useSettingsSnapshot: () => ({ loaded: false, settings: null }),
}));

vi.mock("./utils/harnessCatalog", () => ({
  preloadHarnessLogos: vi.fn(() => {}),
}));

vi.mock("./utils/updateNotice", () => ({
  refreshUpdateCheck: vi.fn(async () => {}),
}));

vi.mock("./utils/analytics", () => ({
  initAnalytics: vi.fn(() => {}),
  setAnalyticsEnabled: vi.fn(() => {}),
  trackAppOpened: vi.fn(() => {}),
}));

vi.mock("./utils/desktop", () => ({
  isDesktopApp: vi.fn(() => false),
  getDesktopPlatform: vi.fn(async () => "unknown"),
  desktopSetTitlebarColor: vi.fn(async () => {}),
  desktopSetWindowTitle: vi.fn(async () => {}),
  desktopListen: vi.fn(async <T,>(event: string, handler: (payload: T) => void) => {
    desktopHandlers.set(event, (payload?: unknown) => handler(payload as T));
    return () => {
      desktopHandlers.delete(event);
    };
  }),
  desktopSetMenuState: vi.fn(async () => {}),
  desktopOpenWorkspaceInNewWindow: vi.fn(async () => {}),
  openExternalLink: vi.fn(async () => true),
}));

vi.mock("./state/uiStateStore", () => ({
  loadSettingsV1: vi.fn(async () => null),
  saveSettingsV1: vi.fn(async () => {}),
}));

beforeEach(() => {
  vi.clearAllMocks();
  desktopHandlers.clear();
  vi.mocked(isDesktopApp).mockReturnValue(false);
  vi.mocked(getDesktopPlatform).mockResolvedValue("unknown");
  vi.mocked(desktopSetTitlebarColor).mockResolvedValue();
  vi.mocked(desktopSetWindowTitle).mockResolvedValue();
  window.history.pushState({}, "", "/");
  const globalWithFetch = globalThis as typeof globalThis & { fetch: typeof fetch };
  globalWithFetch.fetch = vi.fn(async () => {
    return new Response(JSON.stringify([]), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });
});

test("renders app shell", async () => {
  render(<App />);
  // App root route is the launcher.
  expect(await screen.findByText("New Workspace")).toBeInTheDocument();
});

test("desktop settings event uses SPA navigation to preserve workspace context", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-123");

  render(<App />);
  expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

  await waitFor(() => {
    expect(vi.mocked(desktopListen)).toHaveBeenCalledWith("desktop_open_settings", expect.any(Function));
  });

  const handler = desktopHandlers.get("desktop_open_settings");
  if (!handler) {
    throw new Error("desktop_open_settings handler was not registered");
  }
  act(() => {
    handler();
  });

  await waitFor(() => {
    expect(window.location.pathname).toBe("/settings");
    expect(new URLSearchParams(window.location.search).get("ws")).toBe("ws-123");
  });
  expect(await screen.findByText("Settings Screen")).toBeInTheDocument();
});

test("desktop titlebar DOM event navigates to the provided settings target", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-789");

  render(<App />);
  expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

  act(() => {
    window.dispatchEvent(
      new CustomEvent("ctx:open-settings", {
        detail: { target: "/settings?ws=ws-789" },
      }),
    );
  });

  await waitFor(() => {
    expect(window.location.pathname).toBe("/settings");
    expect(new URLSearchParams(window.location.search).get("ws")).toBe("ws-789");
  });
  expect(await screen.findByText("Settings Screen")).toBeInTheDocument();
});

test("desktop menu action routes to workspace settings and updates menu state", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-321");

  render(<App />);
  expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

  await waitFor(() => {
    expect(vi.mocked(desktopListen)).toHaveBeenCalledWith("desktop_menu_action", expect.any(Function));
    expect(vi.mocked(desktopSetMenuState)).toHaveBeenCalled();
  });

  const handler = desktopHandlers.get("desktop_menu_action");
  if (!handler) {
    throw new Error("desktop_menu_action handler was not registered");
  }
  act(() => {
    handler({ commandId: "go.settings" });
  });

  await waitFor(() => {
    expect(window.location.pathname).toBe("/settings");
    expect(new URLSearchParams(window.location.search).get("ws")).toBe("ws-321");
  });
  expect(await screen.findByText("Settings Screen")).toBeInTheDocument();
});

test("desktop menu action accepts snake_case payload for compatibility", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-901");

  render(<App />);
  expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

  const handler = await waitFor(() => {
    const value = desktopHandlers.get("desktop_menu_action");
    if (!value) {
      throw new Error("desktop_menu_action handler not ready");
    }
    return value;
  });

  act(() => {
    handler({ command_id: "go.settings" });
  });

  await waitFor(() => {
    expect(window.location.pathname).toBe("/settings");
    expect(new URLSearchParams(window.location.search).get("ws")).toBe("ws-901");
  });
});

test("desktop menu action forwards workbench-scoped commands to the web menu bus", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-777");

  const received: string[] = [];
  const onCommand = (event: Event) => {
    const custom = event as CustomEvent<{ commandId?: unknown }>;
    if (typeof custom.detail?.commandId === "string") {
      received.push(custom.detail.commandId);
    }
  };
  window.addEventListener(WEB_MENU_COMMAND_EVENT, onCommand as EventListener);

  try {
    render(<App />);
    expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

    const handler = await waitFor(() => {
      const value = desktopHandlers.get("desktop_menu_action");
      if (!value) {
        throw new Error("desktop_menu_action handler not ready");
      }
      return value;
    });

    act(() => {
      handler({ command_id: "view.toggle-diff" });
    });

    await waitFor(() => {
      expect(received).toContain("view.toggle-diff");
    });
  } finally {
    window.removeEventListener(WEB_MENU_COMMAND_EVENT, onCommand as EventListener);
  }
});

test("desktop menu report issue opens external tracker link", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  window.history.pushState({}, "", "/workspaces/ws-654");

  render(<App />);
  expect(await screen.findByText("Workbench Screen")).toBeInTheDocument();

  const handler = await waitFor(() => {
    const value = desktopHandlers.get("desktop_menu_action");
    if (!value) {
      throw new Error("desktop_menu_action handler not ready");
    }
    return value;
  });

  act(() => {
    handler({ commandId: "help.report-issue" });
  });

  await waitFor(() => {
    expect(vi.mocked(openExternalLink)).toHaveBeenCalledWith(
      "https://github.com/context-labs/ctx/issues/new",
    );
  });
});

test("desktop settings route updates native window title", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  vi.mocked(getDesktopPlatform).mockResolvedValue("macos");
  window.history.pushState({}, "", "/settings");

  render(<App />);
  expect(await screen.findByText("Settings Screen")).toBeInTheDocument();

  await waitFor(() => {
    expect(vi.mocked(desktopSetWindowTitle)).toHaveBeenCalledWith("Settings");
  });
});

test("desktop launcher route clears native window title", async () => {
  vi.mocked(isDesktopApp).mockReturnValue(true);
  vi.mocked(getDesktopPlatform).mockResolvedValue("macos");
  window.history.pushState({}, "", "/");

  render(<App />);
  expect(await screen.findByText("New Workspace")).toBeInTheDocument();

  await waitFor(() => {
    expect(vi.mocked(desktopSetWindowTitle)).toHaveBeenCalledWith("");
  });
});
