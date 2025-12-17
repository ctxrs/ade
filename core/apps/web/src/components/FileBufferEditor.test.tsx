import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FileBufferEditor } from "./FileBufferEditor";
import { daemonFetchRaw } from "../api/client";

vi.mock("@monaco-editor/react", () => ({
  default: (props: any) => {
    return (
      <textarea
        data-testid="editor"
        value={props.value ?? ""}
        onChange={(e) => props.onChange?.((e.target as HTMLTextAreaElement).value)}
      />
    );
  },
}));

vi.mock("../state/sessionSupervisor", () => ({
  useSessionEntry: () => ({ diagnosticsByPath: {} }),
}));

vi.mock("../api/client", () => ({
  daemonFetchRaw: vi.fn(),
}));

const flushMicrotasks = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe("FileBufferEditor", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();

    const mock = vi.mocked(daemonFetchRaw);
    mock.mockImplementation(async (path: string, init?: RequestInit) => {
      if (path === "/api/buffers/open") {
        return {
          status: 200,
          body: JSON.stringify({
            buffer_id: "buf-1",
            path: "/repo/file.ts",
            version: 1,
            text: "initial",
            last_disk_sha256: "sha",
          }),
          content_type: "application/json",
        };
      }
      if (path === "/api/buffers/update") {
        const body = init?.body ? JSON.parse(String(init.body)) : {};
        return {
          status: 200,
          body: JSON.stringify({
            buffer_id: String(body.buffer_id ?? "buf-1"),
            version: Number(body.version ?? 1),
            last_disk_sha256: "sha",
          }),
          content_type: "application/json",
        };
      }
      if (path === "/api/buffers/close") {
        return {
          status: 200,
          body: "",
          content_type: "application/json",
        };
      }
      return { status: 404, body: "not found", content_type: "text/plain" };
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("autosave uses the latest text", async () => {
    render(<FileBufferEditor sessionId="s1" path="/repo/file.ts" onClose={vi.fn()} />);
    await act(async () => {
      await flushMicrotasks();
    });

    const editor = screen.getByTestId("editor") as HTMLTextAreaElement;
    expect(editor.value).toBe("initial");

    await act(async () => {
      fireEvent.change(editor, { target: { value: "a" } });
    });
    await act(async () => {
      fireEvent.change(editor, { target: { value: "ab" } });
    });

    act(() => {
      vi.advanceTimersByTime(500);
    });
    await act(async () => {
      await flushMicrotasks();
    });

    const updateCalls = vi
      .mocked(daemonFetchRaw)
      .mock.calls.filter(([p]) => p === "/api/buffers/update")
      .map(([, init]) => (init?.body ? JSON.parse(String(init.body)) : null))
      .filter(Boolean);
    const persisted = updateCalls.filter((b: any) => b.persist === true);
    expect(persisted[persisted.length - 1]?.text).toBe("ab");
  });

  it("closes the latest buffer id on unmount", async () => {
    const { unmount } = render(<FileBufferEditor sessionId="s1" path="/repo/file.ts" onClose={vi.fn()} />);
    await act(async () => {
      await flushMicrotasks();
    });

    unmount();
    await act(async () => {
      await flushMicrotasks();
    });

    const closeCalls = vi.mocked(daemonFetchRaw).mock.calls.filter(([p]) => p === "/api/buffers/close");
    expect(closeCalls.length).toBe(1);
    const closeBody = closeCalls[0]?.[1]?.body ? JSON.parse(String(closeCalls[0][1]!.body)) : null;
    expect(closeBody).toEqual({ session_id: "s1", buffer_id: "buf-1" });
  });
});
