import { render, screen } from "@testing-library/react";
import { vi } from "vitest";
import App from "./App";

vi.mock("./state/uiStateStore", () => ({
  loadSettingsV1: vi.fn(async () => null),
  saveSettingsV1: vi.fn(async () => {}),
}));

beforeEach(() => {
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
