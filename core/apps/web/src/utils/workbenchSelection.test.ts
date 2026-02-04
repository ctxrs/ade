import { describe, expect, it } from "vitest";
import { pickPreferredSession, pickPreferredSessionId } from "./workbenchSelection";

describe("workbenchSelection", () => {
  it("prefers an active session when present", () => {
    const sessions = [
      { id: "s1", status: "completed" },
      { id: "s2", status: "active" },
      { id: "s3", status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("s2");
    expect(pickPreferredSession(sessions)?.id).toBe("s2");
  });

  it("honors a preferred session id when provided", () => {
    const sessions = [
      { id: "s1", status: "completed" },
      { id: "s2", status: "active" },
      { id: "s3", status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions, "s3")).toBe("s3");
    expect(pickPreferredSession(sessions, "s3")?.id).toBe("s3");
  });

  it("prefers non-subagent sessions over subagents", () => {
    const sessions = [
      { id: "main-old", status: "completed" },
      { id: "sub", status: "active", relationship: "sub_agent" },
      { id: "main-new", status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("main-new");
    expect(pickPreferredSession(sessions)?.id).toBe("main-new");
  });

  it("falls back to subagents when no main sessions exist", () => {
    const sessions = [{ id: "sub", status: "active", relationship: "sub_agent" }];
    expect(pickPreferredSessionId(sessions)).toBe("sub");
    expect(pickPreferredSession(sessions)?.id).toBe("sub");
  });

  it("otherwise prefers the most recent session", () => {
    const sessions = [
      { id: "old", status: "completed" },
      { id: "new", status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("new");
  });

});
