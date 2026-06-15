import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  initMock,
  isFeatureEnabledMock,
  optInCapturingMock,
  optOutCapturingMock,
  registerMock,
  recordSemanticTelemetryEventMock,
  setSemanticTelemetryRemoteEnabledMock,
} = vi.hoisted(() => ({
  initMock: vi.fn(),
  isFeatureEnabledMock: vi.fn(() => false),
  optInCapturingMock: vi.fn(),
  optOutCapturingMock: vi.fn(),
  registerMock: vi.fn(),
  recordSemanticTelemetryEventMock: vi.fn(),
  setSemanticTelemetryRemoteEnabledMock: vi.fn(),
}));

vi.mock("posthog-js", () => ({
  default: {
    init: initMock,
    isFeatureEnabled: isFeatureEnabledMock,
    opt_in_capturing: optInCapturingMock,
    opt_out_capturing: optOutCapturingMock,
    register: registerMock,
  },
}));

vi.mock("../../api/client", () => ({
  recordSemanticTelemetryEvent: recordSemanticTelemetryEventMock,
  setSemanticTelemetryRemoteEnabled: setSemanticTelemetryRemoteEnabledMock,
}));

vi.mock("./config", () => ({
  getAnalyticsEnvironment: () => "production",
  getPostHogHost: () => "https://t.ctx.rs",
  getPostHogKey: () => "phc_test_key",
  getPostHogProjectId: () => "example-project",
  getPostHogUiHost: () => "https://us.posthog.com",
}));

vi.mock("./identity", () => ({
  getInstallId: () => "install-test",
}));

vi.mock("../runtime", () => ({
  getAppShellKind: () => "desktop",
}));

describe("analytics client", () => {
  beforeEach(() => {
    vi.resetModules();
    initMock.mockReset();
    isFeatureEnabledMock.mockReset();
    isFeatureEnabledMock.mockReturnValue(false);
    optInCapturingMock.mockReset();
    optOutCapturingMock.mockReset();
    registerMock.mockReset();
    recordSemanticTelemetryEventMock.mockReset();
    setSemanticTelemetryRemoteEnabledMock.mockReset();
  });

  it("captures remote semantic product events through the ctx emitter", async () => {
    const mod = await import("./client");

    mod.setAnalyticsEnabled(true);
    const accepted = mod.captureProductEvent("foreground_backlog_observed", 2, {
      provider_id: "codex",
      backlog_ms: 1800,
      env_target: "local",
      workspace_id: "workspace-raw",
      sessionId: "session-raw",
    });

    expect(accepted).toBe(true);
    expect(setSemanticTelemetryRemoteEnabledMock).toHaveBeenCalledWith(true);
    expect(recordSemanticTelemetryEventMock).toHaveBeenCalledWith(expect.objectContaining({
      event_name: "foreground_backlog_observed",
      event_version: 2,
      plane: "product",
      delivery: "remote",
      origin_runtime: "desktop",
      origin_install_id: "install-test",
      surface: "desktop",
      env_target: "local",
      properties: expect.objectContaining({
        provider_id: "codex",
        backlog_ms: 1800,
        analytics_environment: "production",
      }),
    }));
    expect(recordSemanticTelemetryEventMock.mock.calls[0]?.[0].properties).not.toHaveProperty("workspace_id");
    expect(recordSemanticTelemetryEventMock.mock.calls[0]?.[0].properties).not.toHaveProperty("sessionId");
    expect(recordSemanticTelemetryEventMock.mock.calls[0]?.[0].properties).not.toHaveProperty("env_target");
  });

  it("keeps reserved semantic envelope fields authoritative over raw properties", async () => {
    const mod = await import("./client");

    mod.setAnalyticsEnabled(true);
    const accepted = mod.captureProductEvent("foreground_freshness_sla_missed", 5, {
      app_version: "raw-version",
      arch: "raw-arch",
      analytics_environment: "raw-environment",
      env_target: "remote",
      event_id: "raw-event-id",
      event_name: "raw-event-name",
      event_version: 99,
      occurred_at: "1999-01-01T00:00:00.000Z",
      origin_install_id: "raw-install",
      origin_runtime: "daemon",
      os: "raw-os",
      plane: "incident",
      source: "raw-source",
      surface: "foreground_backlog",
      traffic_class: "bot",
      freshness_surface: "foreground_backlog",
    });

    expect(accepted).toBe(true);
    const event = recordSemanticTelemetryEventMock.mock.calls[0]?.[0];
    expect(event).toEqual(expect.objectContaining({
      event_name: "foreground_freshness_sla_missed",
      event_version: 5,
      plane: "product",
      origin_runtime: "desktop",
      origin_install_id: "install-test",
      app_version: expect.not.stringMatching(/^raw-/),
      os: expect.not.stringMatching(/^raw-/),
      arch: expect.not.stringMatching(/^raw-/),
      surface: "desktop",
      env_target: "remote",
      source: null,
    }));
    expect(event.occurred_at).not.toBe("1999-01-01T00:00:00.000Z");
    expect(event.properties).toEqual(expect.objectContaining({
      analytics_environment: "production",
      freshness_surface: "foreground_backlog",
      traffic_class: "user",
    }));
    for (const key of [
      "app_version",
      "arch",
      "env_target",
      "event_id",
      "event_name",
      "event_version",
      "occurred_at",
      "origin_install_id",
      "origin_runtime",
      "os",
      "plane",
      "source",
      "surface",
    ]) {
      expect(event.properties).not.toHaveProperty(key);
    }
  });

  it("keeps incident semantic surfaces in non-reserved properties", async () => {
    const mod = await import("./client");

    mod.setAnalyticsEnabled(true);
    expect(mod.captureIncidentEvent("foreground_freshness_sla_missed", 1, {
      metric: "final_delivery_stale_ms",
      freshness_surface: "foreground_backlog",
      severity_bucket: "severe",
    })).toBe(true);
    expect(mod.captureProductEvent("desktop_webview_recovery_observed", 1, {
      trigger: "heartbeat_timeout",
      action: "recreate",
      recovery_surface: "workbench",
      daemon_health: "ok",
    })).toBe(true);

    expect(recordSemanticTelemetryEventMock).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({
        event_name: "foreground_freshness_sla_missed",
        plane: "incident",
        surface: "desktop",
        properties: expect.objectContaining({
          freshness_surface: "foreground_backlog",
        }),
      }),
    );
    expect(recordSemanticTelemetryEventMock.mock.calls[0]?.[0].properties).not.toHaveProperty("surface");

    expect(recordSemanticTelemetryEventMock).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({
        event_name: "desktop_webview_recovery_observed",
        plane: "product",
        surface: "desktop",
        properties: expect.objectContaining({
          recovery_surface: "workbench",
        }),
      }),
    );
    expect(recordSemanticTelemetryEventMock.mock.calls[1]?.[0].properties).not.toHaveProperty("surface");
  });

  it("does not initialize PostHog until analytics is enabled", async () => {
    const mod = await import("./client");

    mod.initAnalytics();

    expect(initMock).not.toHaveBeenCalled();

    mod.setAnalyticsEnabled(true);

    expect(initMock).toHaveBeenCalledTimes(1);
  });

  it("drops remote captures when analytics is disabled", async () => {
    const mod = await import("./client");

    mod.setAnalyticsEnabled(false);
    const accepted = mod.captureAnalyticsEvent("foreground_backlog_observed", { backlog_ms: 1800 });

    expect(accepted).toBe(false);
    expect(setSemanticTelemetryRemoteEnabledMock).toHaveBeenCalledWith(false);
    expect(recordSemanticTelemetryEventMock).not.toHaveBeenCalled();
  });

  it("still records local-only incident events when remote analytics is disabled", async () => {
    const mod = await import("./client");

    mod.setAnalyticsEnabled(false);
    const accepted = mod.captureIncidentEvent(
      "renderer_backlog_sample",
      1,
      { queue_age_ms: 3200, env_target: "remote" },
      { delivery: "local_only", source: "worker_patch" },
    );

    expect(accepted).toBe(true);
    expect(recordSemanticTelemetryEventMock).toHaveBeenCalledWith(expect.objectContaining({
      event_name: "renderer_backlog_sample",
      plane: "incident",
      delivery: "local_only",
      source: "worker_patch",
      env_target: "remote",
      properties: expect.objectContaining({
        queue_age_ms: 3200,
      }),
    }));
    expect(recordSemanticTelemetryEventMock.mock.calls[0]?.[0].properties).not.toHaveProperty("env_target");
  });
});
