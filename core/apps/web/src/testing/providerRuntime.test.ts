import { describe, expect, it, vi } from "vitest";
import type { JsonRequestLike, JsonResponseLike } from "./providerRuntime";
import {
  ensureProviderInstalledAndHealthy,
  resolveWorkspaceProviderModelId,
  verifyProviderForWorkspace,
  waitForTerminalState,
} from "./providerRuntime";

const jsonResponse = (status: number, body: unknown): JsonResponseLike => ({
  ok: () => status >= 200 && status < 300,
  status: () => status,
  json: async () => body,
  text: async () => (typeof body === "string" ? body : JSON.stringify(body)),
});

const createRequest = (
  getImpl: JsonRequestLike["get"],
  postImpl: JsonRequestLike["post"],
): JsonRequestLike => ({
  get: getImpl,
  post: postImpl,
});

describe("providerRuntime", () => {
  it("installs a missing provider against the requested host target and rechecks health", async () => {
    const get = vi.fn<JsonRequestLike["get"]>()
      .mockResolvedValueOnce(jsonResponse(200, {
        installed: false,
        health: "missing",
        diagnostics: ["provider not installed"],
        details: {},
      }))
      .mockResolvedValueOnce(jsonResponse(200, {
        state: "running",
        last_event: { stage: "download" },
      }))
      .mockResolvedValueOnce(jsonResponse(200, {
        state: "succeeded",
      }))
      .mockResolvedValueOnce(jsonResponse(200, {
        installed: true,
        health: "ok",
        diagnostics: [],
        details: {
          install_target: "host",
          managed_target: "host",
        },
      }));
    const post = vi.fn<JsonRequestLike["post"]>()
      .mockResolvedValueOnce(jsonResponse(200, { install_id: "install-1" }));
    const request = createRequest(get, post);

    const status = await ensureProviderInstalledAndHealthy(request, "gemini", "host", {
      pollMs: 0,
      sleep: async () => {},
    });

    expect(status).toMatchObject({
      installed: true,
      health: "ok",
      details: {
        install_target: "host",
        managed_target: "host",
      },
    });
    expect(get).toHaveBeenNthCalledWith(1, "/api/providers/gemini?target=host", { timeout: 30_000 });
    expect(post).toHaveBeenCalledWith("/api/providers/gemini/install?target=host", {
      data: {},
      timeout: 30_000,
    });
    expect(get).toHaveBeenNthCalledWith(2, "/api/providers/install/install-1", { timeout: 30_000 });
    expect(get).toHaveBeenNthCalledWith(4, "/api/providers/gemini?target=host", { timeout: 30_000 });
  });

  it("retries workspace verification until the provider reports ok", async () => {
    const get = vi.fn<JsonRequestLike["get"]>();
    const post = vi.fn<JsonRequestLike["post"]>()
      .mockResolvedValueOnce(jsonResponse(200, {
        status: "error",
        message: "models.list probe timed out",
      }))
      .mockResolvedValueOnce(jsonResponse(200, {
        status: "ok",
        probed_at: "2026-03-06T00:00:00Z",
      }));
    const request = createRequest(get, post);

    const payload = await verifyProviderForWorkspace(request, "ws-1", "gemini", {
      pollMs: 0,
      sleep: async () => {},
    });

    expect(payload).toMatchObject({
      status: "ok",
      probed_at: "2026-03-06T00:00:00Z",
    });
    expect(post).toHaveBeenCalledTimes(2);
  });

  it("waits for provider options to publish a model id instead of using a fallback", async () => {
    const get = vi.fn<JsonRequestLike["get"]>()
      .mockResolvedValueOnce(jsonResponse(200, {
        models: {
          models: [],
        },
      }))
      .mockResolvedValueOnce(jsonResponse(200, {
        models: {
          current_model_id: "gemini-2.5-flash",
        },
      }));
    const post = vi.fn<JsonRequestLike["post"]>();
    const request = createRequest(get, post);

    const modelId = await resolveWorkspaceProviderModelId(request, "ws-1", "gemini", {
      pollMs: 0,
      sleep: async () => {},
    });

    expect(modelId).toBe("gemini-2.5-flash");
    expect(get).toHaveBeenCalledTimes(2);
    expect(get).toHaveBeenNthCalledWith(1, "/api/workspaces/ws-1/providers/gemini/options", { timeout: 30_000 });
  });

  it("returns actionable runtime failure details from terminal events", async () => {
    const get = vi.fn<JsonRequestLike["get"]>()
      .mockResolvedValueOnce(jsonResponse(200, {
        activity: {
          last_turn_status: "failed",
        },
        turns: [
          {
            status: "failed",
          },
        ],
        messages: [],
        session: {
          model_id: "gemini-2.5-flash",
        },
        events: [
          {
            event_type: "session.error",
            payload_json: {
              message: "invalid API key",
            },
          },
        ],
      }));
    const post = vi.fn<JsonRequestLike["post"]>();
    const request = createRequest(get, post);

    const state = await waitForTerminalState(request, "session-1", {
      pollMs: 0,
      sleep: async () => {},
    });

    expect(state).toMatchObject({
      done: true,
      terminalStatus: "failed",
      modelId: "gemini-2.5-flash",
    });
    expect(state.errorMessage).toContain("session.error");
    expect(state.errorMessage).toContain("invalid API key");
  });
});
