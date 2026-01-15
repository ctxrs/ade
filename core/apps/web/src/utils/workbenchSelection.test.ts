import { describe, expect, it } from "vitest";
import { pickPreferredSession, pickPreferredSessionId } from "./workbenchSelection";

describe("workbenchSelection", () => {
  it("prefers an active session when present", () => {
    const sessions = [
      { id: { 0: "s1" }, status: "completed" },
      { id: { 0: "s2" }, status: "active" },
      { id: { 0: "s3" }, status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("s2");
    expect(pickPreferredSession(sessions)?.id?.[0]).toBe("s2");
  });

  it("honors a preferred session id when provided", () => {
    const sessions = [
      { id: { 0: "s1" }, status: "completed" },
      { id: { 0: "s2" }, status: "active" },
      { id: { 0: "s3" }, status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions, "s3")).toBe("s3");
    expect(pickPreferredSession(sessions, "s3")?.id?.[0]).toBe("s3");
  });

  it("prefers non-subagent sessions over subagents", () => {
    const sessions = [
      { id: { 0: "main-old" }, status: "completed" },
      { id: { 0: "sub" }, status: "active", relationship: "sub_agent" },
      { id: { 0: "main-new" }, status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("main-new");
    expect(pickPreferredSession(sessions)?.id?.[0]).toBe("main-new");
  });

  it("falls back to subagents when no main sessions exist", () => {
    const sessions = [{ id: { 0: "sub" }, status: "active", relationship: "sub_agent" }];
    expect(pickPreferredSessionId(sessions)).toBe("sub");
    expect(pickPreferredSession(sessions)?.id?.[0]).toBe("sub");
  });

  it("otherwise prefers the most recent session", () => {
    const sessions = [
      { id: { 0: "old" }, status: "completed" },
      { id: { 0: "new" }, status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("new");
  });

});
