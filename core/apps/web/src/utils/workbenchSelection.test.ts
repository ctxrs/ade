import { describe, expect, it } from "vitest";
import { pickPreferredSession, pickPreferredSessionId, pickPreferredTrackId } from "./workbenchSelection";

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

  it("otherwise prefers the most recent session", () => {
    const sessions = [
      { id: { 0: "old" }, status: "completed" },
      { id: { 0: "new" }, status: "completed" },
    ];
    expect(pickPreferredSessionId(sessions)).toBe("new");
  });

  it("keeps the current track when valid and has sessions", () => {
    const ids = ["t1", "t2"];
    const sessionsByTrack = { t1: [{ id: { 0: "s1" } }], t2: [{ id: { 0: "s2" } }] };
    expect(pickPreferredTrackId(ids, sessionsByTrack, "t2")).toBe("t2");
  });

  it("falls back to a track with sessions when the current track has none", () => {
    const ids = ["t1", "t2"];
    const sessionsByTrack = { t1: [], t2: [{ id: { 0: "s2" } }] };
    expect(pickPreferredTrackId(ids, sessionsByTrack, "t1")).toBe("t2");
  });

  it("falls back to the first track when none have sessions", () => {
    const ids = ["t1", "t2"];
    const sessionsByTrack = { t1: [], t2: [] };
    expect(pickPreferredTrackId(ids, sessionsByTrack, "missing")).toBe("t1");
  });
});

