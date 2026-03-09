import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const daemonFetchRawMock = vi.hoisted(() => vi.fn());
const desktopGetConnectionMock = vi.hoisted(() => vi.fn());
const desktopGetVersionMock = vi.hoisted(() => vi.fn());

vi.mock("../api/client", () => ({
  daemonFetchRaw: daemonFetchRawMock,
}));

vi.mock("../utils/desktop", () => ({
  desktopGetConnection: desktopGetConnectionMock,
  desktopGetVersion: desktopGetVersionMock,
  isDesktopApp: () => true,
}));

describe("daemonAvailabilityMonitor", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
    vi.resetModules();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shares one liveness poller across multiple subscribers", async () => {
    daemonFetchRawMock.mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        daemon_version: "1.2.3",
        compatibility: {
          desktop_exact_version: "1.2.3",
        },
      }),
      content_type: "application/json",
    });
    desktopGetConnectionMock.mockResolvedValue({ kind: "local" });
    desktopGetVersionMock.mockResolvedValue("1.2.3");

    const mod = await import("./daemonAvailabilityMonitor");
    const listenerA = vi.fn();
    const listenerB = vi.fn();

    const unsubscribeA = mod.subscribeDaemonAvailability(listenerA);
    const unsubscribeB = mod.subscribeDaemonAvailability(listenerB);
    await mod.checkDaemonAvailabilityNow();

    expect(daemonFetchRawMock).toHaveBeenCalledTimes(1);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(1);
    expect(desktopGetVersionMock).toHaveBeenCalledTimes(1);
    expect(mod.getDaemonAvailabilitySnapshot().status).toBe("ok");

    await vi.advanceTimersByTimeAsync(20_000);

    expect(daemonFetchRawMock).toHaveBeenCalledTimes(2);
    expect(desktopGetConnectionMock).toHaveBeenCalledTimes(2);
    expect(desktopGetVersionMock).toHaveBeenCalledTimes(2);

    unsubscribeA();
    unsubscribeB();
    await vi.advanceTimersByTimeAsync(20_000);

    expect(daemonFetchRawMock).toHaveBeenCalledTimes(2);
  });

  it("classifies daemon and desktop version mismatches", async () => {
    daemonFetchRawMock.mockResolvedValue({
      status: 200,
      body: JSON.stringify({
        daemon_version: "2.0.0",
        compatibility: {
          desktop_exact_version: "2.0.0",
        },
      }),
      content_type: "application/json",
    });
    desktopGetConnectionMock.mockResolvedValue({ kind: "ssh" });
    desktopGetVersionMock.mockResolvedValue("1.5.0");

    const mod = await import("./daemonAvailabilityMonitor");
    const unsubscribe = mod.subscribeDaemonAvailability(() => {});
    await mod.checkDaemonAvailabilityNow();

    expect(mod.getDaemonAvailabilitySnapshot()).toMatchObject({
      status: "mismatch",
      desktopKind: "ssh",
      desktopVersion: "1.5.0",
      mismatch: {
        desktop_version: "1.5.0",
        daemon_version: "2.0.0",
        expected_version: "2.0.0",
        kind: "desktop_older",
      },
    });

    unsubscribe();
  });
});
