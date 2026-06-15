import { describe, expect, it } from "vitest";
import { buildEventEnvelope } from "./context";

describe("buildEventEnvelope", () => {
  it("emits the required baseline envelope fields", () => {
    const envelope = buildEventEnvelope(1, { provider_id: "codex" });

    expect(envelope.event_version).toBe(1);
    expect(typeof envelope.occurred_at).toBe("string");
    expect(typeof envelope.app_version).toBe("string");
    expect(typeof envelope.os).toBe("string");
    expect(typeof envelope.arch).toBe("string");
    expect(typeof envelope.surface).toBe("string");
    expect(typeof envelope.analytics_environment).toBe("string");
    expect(envelope.traffic_class).toBe("user");
    expect(envelope.provider_id).toBe("codex");
  });

  it("does not allow properties to override reserved envelope fields", () => {
    const envelope = buildEventEnvelope(3, {
      app_version: "raw-version",
      arch: "raw-arch",
      analytics_environment: "raw-environment",
      env_target: "remote",
      event_name: "raw-event",
      event_version: 99,
      occurred_at: "1999-01-01T00:00:00.000Z",
      origin_install_id: "raw-install",
      os: "raw-os",
      surface: "foreground_backlog",
      traffic_class: "bot",
      provider_id: "codex",
    });

    expect(envelope.event_version).toBe(3);
    expect(envelope.occurred_at).not.toBe("1999-01-01T00:00:00.000Z");
    expect(envelope.app_version).not.toBe("raw-version");
    expect(envelope.os).not.toBe("raw-os");
    expect(envelope.arch).not.toBe("raw-arch");
    expect(envelope.surface).not.toBe("foreground_backlog");
    expect(envelope.analytics_environment).not.toBe("raw-environment");
    expect(envelope.traffic_class).toBe("user");
    expect(envelope.provider_id).toBe("codex");
    expect(envelope.env_target).toBe("remote");
    expect(envelope).not.toHaveProperty("event_name");
    expect(envelope).not.toHaveProperty("origin_install_id");
  });
});
