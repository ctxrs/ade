import { render, screen } from "@testing-library/react";
import { vi } from "vitest";
import App from "./App";

beforeEach(() => {
  (globalThis as any).fetch = vi.fn(async () => {
    return new Response(JSON.stringify([]), {
      status: 200,
      headers: { "Content-Type": "application/json" },
    });
  });
});

test("renders app shell", async () => {
  render(<App />);
  expect(await screen.findByText("Workspaces")).toBeInTheDocument();
});
