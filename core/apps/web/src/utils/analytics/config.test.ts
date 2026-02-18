import { describe, expect, it } from "vitest";
import { resolveAnalyticsEnvironment } from "./config";

describe("resolveAnalyticsEnvironment", () => {
  it("uses explicit override when provided", () => {
    expect(resolveAnalyticsEnvironment("production", "staging")).toBe("production");
    expect(resolveAnalyticsEnvironment("staging", "production")).toBe("staging");
  });

  it("maps production mode to production when no explicit override exists", () => {
    expect(resolveAnalyticsEnvironment(undefined, "production")).toBe("production");
    expect(resolveAnalyticsEnvironment(undefined, "prod")).toBe("production");
  });

  it("defaults unknown modes to staging", () => {
    expect(resolveAnalyticsEnvironment(undefined, "staging")).toBe("staging");
    expect(resolveAnalyticsEnvironment(undefined, "development")).toBe("staging");
    expect(resolveAnalyticsEnvironment(undefined, "preview")).toBe("staging");
  });
});
