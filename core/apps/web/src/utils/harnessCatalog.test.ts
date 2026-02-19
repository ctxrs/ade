import { describe, expect, it } from "vitest";
import { HARNESS_CATALOG } from "./harnessCatalog";

describe("harnessCatalog", () => {
  it("maps Pi to the official pi.dev logo asset", () => {
    const pi = HARNESS_CATALOG.find((entry) => entry.id === "pi");
    expect(pi).toBeDefined();
    expect(pi?.logoSrc).toContain("pi.svg");
    expect(pi?.invertInLight).toBe(true);
  });
});
