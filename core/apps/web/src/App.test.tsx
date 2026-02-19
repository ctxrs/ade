import type { ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, test, vi } from "vitest";
import App from "./App";
import { desktopListen, isDesktopApp } from "./utils/desktop";

let desktopHandler: ((payload?: unknown) => void) | null = null;

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
  desktopListen: vi.fn(async <T,>(_event: string, handler: (payload: T) => void) => {
    desktopHandler = (payload?: unknown) => handler(payload as T);
    return () => {
      desktopHandler = null;
    };
  }),
}));

vi.mock("./state/uiStateStore", () => ({
  loadSettingsV1: vi.fn(async () => null),
  saveSettingsV1: vi.fn(async () => {}),
}));

beforeEach(() => {
  vi.clearAllMocks();
  desktopHandler = null;
  vi.mocked(isDesktopApp).mockReturnValue(false);
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

  if (!desktopHandler) {
    throw new Error("desktop_open_settings handler was not registered");
  }
  const handler = desktopHandler;
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
