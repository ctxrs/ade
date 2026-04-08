import { describe, expect, it, vi } from "vitest";

import { createInternalEntry } from "./entryState";
import { syncSupportLoadsForOpenSession } from "./supportLoads";

describe("syncSupportLoadsForOpenSession", () => {
  it("autoloads state for open sessions even before an authoritative state revision is known", () => {
    const entry = createInternalEntry("session-1", {
      transientSeqStart: -1,
      warmTtlMs: 1_000,
    });
    entry.refCount = 1;
    entry.mode = "active";
    entry.freshness = "replica";

    const ensureState = vi.fn(async () => {});
    const ensureSubagentInvocations = vi.fn(async () => {});
    const resolveRequestedStateRev = vi.fn(() => undefined);

    syncSupportLoadsForOpenSession(entry, {
      resolveRequestedStateRev,
      ensureState,
      ensureSubagentInvocations,
    });

    expect(resolveRequestedStateRev).toHaveBeenCalledWith(entry);
    expect(entry.support.stateAutoLoadKey).toBe("epoch:0");
    expect(entry.support.subagentAutoLoadKey).toBe("epoch:0");
    expect(ensureState).toHaveBeenCalledTimes(1);
    expect(ensureState).toHaveBeenCalledWith(entry);
    expect(ensureSubagentInvocations).toHaveBeenCalledTimes(1);
    expect(ensureSubagentInvocations).toHaveBeenCalledWith(entry);

    syncSupportLoadsForOpenSession(entry, {
      resolveRequestedStateRev,
      ensureState,
      ensureSubagentInvocations,
    });

    expect(ensureState).toHaveBeenCalledTimes(1);
    expect(ensureSubagentInvocations).toHaveBeenCalledTimes(1);
  });
});
