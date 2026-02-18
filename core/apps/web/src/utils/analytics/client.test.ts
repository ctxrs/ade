import { beforeEach, describe, expect, it, vi } from "vitest";

const {
  captureMock,
  initMock,
  isFeatureEnabledMock,
  optInCapturingMock,
  optOutCapturingMock,
  registerMock,
} = vi.hoisted(() => ({
  captureMock: vi.fn(),
  initMock: vi.fn(),
  isFeatureEnabledMock: vi.fn(() => false),
  optInCapturingMock: vi.fn(),
  optOutCapturingMock: vi.fn(),
  registerMock: vi.fn(),
}));

vi.mock("posthog-js", () => ({
  default: {
    capture: captureMock,
    init: initMock,
    isFeatureEnabled: isFeatureEnabledMock,
    opt_in_capturing: optInCapturingMock,
    opt_out_capturing: optOutCapturingMock,
    register: registerMock,
  },
}));

vi.mock("./config", () => ({
  getPostHogHost: () => "https://us.i.posthog.com",
  getPostHogKey: () => "phc_test_key",
  getPostHogProjectId: () => "example-project",
  getPostHogUiHost: () => "https://us.posthog.com",
}));

vi.mock("./identity", () => ({
  getInstallId: () => "install-test",
}));

describe("analytics client queue bounds", () => {
  beforeEach(() => {
    vi.resetModules();
    captureMock.mockReset();
    initMock.mockReset();
    isFeatureEnabledMock.mockReset();
    isFeatureEnabledMock.mockReturnValue(false);
    optInCapturingMock.mockReset();
    optOutCapturingMock.mockReset();
    registerMock.mockReset();
  });

  it("bounds pending captures when init never resolves", async () => {
    const mod = await import("./client");
    const loadedCallbacks: Array<() => void> = [];
    initMock.mockImplementation((_key: string, options: { loaded?: () => void }) => {
      if (typeof options.loaded === "function") {
        loadedCallbacks.push(options.loaded);
      }
    });

    mod.setAnalyticsEnabled(true);
    const totalEvents = mod.MAX_PENDING_CAPTURES + 37;
    for (let idx = 0; idx < totalEvents; idx += 1) {
      mod.captureAnalyticsEvent(`event_${idx}`, { seq: idx });
    }

    mod.initAnalytics();
    expect(initMock).toHaveBeenCalledTimes(1);
    expect(loadedCallbacks).toHaveLength(1);
    loadedCallbacks[0]();

    expect(captureMock).toHaveBeenCalledTimes(mod.MAX_PENDING_CAPTURES);
    const firstCapturedName = String(captureMock.mock.calls[0]?.[0] ?? "");
    expect(firstCapturedName).toBe("event_37");
  });
});
