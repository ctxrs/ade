import { describe, expect, it } from "vitest";
import { extractErrorMessage } from "./authDerivations";

describe("extractErrorMessage", () => {
  it("returns plain-string payload errors", () => {
    expect(extractErrorMessage(" provider runtime crashed ")).toBe("provider runtime crashed");
  });

  it("keeps object message + details formatting", () => {
    expect(extractErrorMessage({ message: "Provider failed", details: "Timed out" })).toBe(
      "Provider failed\nDetails: Timed out",
    );
  });
});
