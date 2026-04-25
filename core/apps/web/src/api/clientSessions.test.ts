import { beforeEach, describe, expect, it, vi } from "vitest";
import { deriveBrowserCapabilityToken } from "./browserCapabilityAuth";
import type { DaemonConnection } from "./daemonConnection";

const {
  apiAnyMock,
  desktopUploadBlobMock,
  trackFirstTurnSubmittedMock,
  trackSessionCreatedMock,
  trackUserMessageSentMock,
  getDaemonConnectionMock,
  isDesktopAppMock,
} = vi.hoisted(() => ({
  apiAnyMock: vi.fn(),
  desktopUploadBlobMock: vi.fn(),
  trackFirstTurnSubmittedMock: vi.fn(),
  trackSessionCreatedMock: vi.fn(),
  trackUserMessageSentMock: vi.fn(),
  getDaemonConnectionMock: vi.fn(
    (): DaemonConnection => ({
      baseUrl: "http://127.0.0.1:4399",
      wsBaseUrl: "ws://127.0.0.1:4399",
      targetScope: { kind: "desktop_local" },
      authToken: null,
      runId: null,
    }),
  ),
  isDesktopAppMock: vi.fn(() => false),
}));

vi.mock("./clientBase", () => ({
  apiAny: apiAnyMock,
  authToken: vi.fn(() => null),
}));

vi.mock("./daemonConnection", () => ({
  getDaemonConnection: getDaemonConnectionMock,
  getDaemonHttpUrl: vi.fn(),
}));

vi.mock("../utils/desktop", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/desktop")>();
  return {
    ...actual,
    desktopUploadBlob: desktopUploadBlobMock,
    isDesktopApp: isDesktopAppMock,
  };
});

vi.mock("../utils/analytics", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../utils/analytics")>();
  return {
    ...actual,
    trackFirstTurnSubmitted: trackFirstTurnSubmittedMock,
    trackSessionCreated: trackSessionCreatedMock,
    trackUserMessageSent: trackUserMessageSentMock,
  };
});

import {
  artifactUrl,
  blobUrl,
  createSession,
  postMessage,
  uploadBlob,
} from "./clientSessions";
import { getDaemonHttpUrl } from "./daemonConnection";

describe("createSession analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    apiAnyMock.mockResolvedValue({ id: "session-123" });
  });

  it("posts canonical execution_environment and split analytics fields", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "sandbox",
    });

    const [, options] = apiAnyMock.mock.calls[0] ?? [];
    expect(JSON.parse(String(options?.body ?? "{}"))).toEqual({
      provider_id: "codex",
      model_id: "gpt-5-codex",
      execution_environment: "sandbox",
    });
    expect(trackSessionCreatedMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex",
      executionEnvironment: "sandbox",
      sessionRootKind: "worktree",
      sessionLocation: "local",
    });
  });

  it("emits first_turn_submitted when session starts with initial_prompt", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "host",
      initial_prompt: "hello",
      initial_message_id: "message-1",
      initial_turn_id: "turn-1",
    });

    expect(trackFirstTurnSubmittedMock).toHaveBeenCalledWith({
      sessionId: "session-123",
      providerId: "codex",
      modelId: "gpt-5-codex",
    });
    expect(trackUserMessageSentMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex",
      reasoningEffort: null,
      executionEnvironment: "host",
      sessionKind: "primary",
      isFirstTurn: true,
    });
  });

  it("does not emit first_turn_submitted when initial_prompt is absent", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", { execution_environment: "host" });
    expect(trackFirstTurnSubmittedMock).not.toHaveBeenCalled();
  });

  it("posts split reasoning_effort while preserving analytics on the effective full model id", async () => {
    await createSession("task-1", "codex", "gpt-5-codex", {
      execution_environment: "host",
      reasoning_effort: "xhigh",
    });

    const [, options] = apiAnyMock.mock.calls[0] ?? [];
    expect(JSON.parse(String(options?.body ?? "{}"))).toEqual({
      provider_id: "codex",
      model_id: "gpt-5-codex",
      reasoning_effort: "xhigh",
      execution_environment: "host",
    });
    expect(trackSessionCreatedMock).toHaveBeenCalledWith(expect.objectContaining({
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
    }));
  });
});

describe("postMessage analytics", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isDesktopAppMock.mockReturnValue(false);
    apiAnyMock.mockResolvedValue({ id: "message-123" });
  });

  it("emits user_message_sent with bounded analytics metadata", async () => {
    await postMessage("session-1", "hello", "immediate", [], {
      id: "message-1",
      turn_id: "turn-1",
      analytics: {
        providerId: "codex",
        modelId: "gpt-5-codex/xhigh",
        reasoningEffort: "xhigh",
        executionEnvironment: "sandbox",
        sessionKind: "primary",
        isFirstTurn: false,
      },
    });

    expect(trackUserMessageSentMock).toHaveBeenCalledWith({
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
      reasoningEffort: "xhigh",
      executionEnvironment: "sandbox",
      sessionKind: "primary",
      isFirstTurn: false,
    });
    expect(trackFirstTurnSubmittedMock).toHaveBeenCalledWith({
      sessionId: "session-1",
      providerId: "codex",
      modelId: "gpt-5-codex/xhigh",
    });
  });
});

describe("browser download urls", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(getDaemonHttpUrl).mockImplementation((path: string) => `http://daemon.test${path}`);
  });

  it("builds blob urls with a scoped capability token instead of the raw daemon bearer", () => {
    getDaemonConnectionMock.mockReturnValueOnce({
      baseUrl: "http://daemon.test",
      wsBaseUrl: "ws://daemon.test",
      targetScope: { kind: "desktop_local" },
      authToken: "daemon-secret",
      runId: null,
    });

    const expectedToken = deriveBrowserCapabilityToken("daemon-secret", {
      kind: "blob",
      blobId: "blob-1",
    });

    expect(blobUrl("blob-1")).toBe(
      `http://daemon.test/api/blobs/blob-1?token=${expectedToken}`,
    );
  });

  it("builds artifact urls with a scoped capability token instead of the raw daemon bearer", () => {
    getDaemonConnectionMock.mockReturnValueOnce({
      baseUrl: "http://daemon.test",
      wsBaseUrl: "ws://daemon.test",
      targetScope: { kind: "desktop_local" },
      authToken: "daemon-secret",
      runId: null,
    });

    const expectedToken = deriveBrowserCapabilityToken("daemon-secret", {
      kind: "session_artifact",
      sessionId: "session-1",
      artifactId: "artifact-1",
    });

    expect(artifactUrl("session-1", "artifact-1")).toBe(
      `http://daemon.test/api/sessions/session-1/artifacts/artifact-1?token=${expectedToken}`,
    );
  });
});

describe("uploadBlob", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    isDesktopAppMock.mockReturnValue(false);
  });

  it("infers image MIME for desktop uploads when File.type is empty", async () => {
    isDesktopAppMock.mockReturnValue(true);
    desktopUploadBlobMock.mockResolvedValue({
      blob_id: "blob-1",
      sha256: "sha",
      bytes: 3,
      mime_type: "image/png",
      name: "image.png",
    });

    const bytes = new Uint8Array([1, 2, 3]);
    const file = new File([bytes], "image.png");
    Object.defineProperty(file, "arrayBuffer", {
      value: vi.fn(async () => bytes.buffer.slice(0)),
    });
    await uploadBlob(file);

    expect(desktopUploadBlobMock).toHaveBeenCalledWith({
      bytes: [1, 2, 3],
      mime_type: "image/png",
      name: "image.png",
    });
  });
});
