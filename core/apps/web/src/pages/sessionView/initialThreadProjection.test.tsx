import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Message, SessionEvent, SessionTurn, SessionTurnTool } from "../../api/client";
import {
  shouldFreezeInitialThreadProjection,
  shouldReleaseInitialThreadProjection,
  shouldRestoreBottomAnchorAfterProjectionRelease,
  useInitialThreadProjection,
  type InitialThreadProjection,
} from "./initialThreadProjection";

type HarnessProps = {
  projection: InitialThreadProjection;
  releaseToLive: boolean;
};

let latestProjection: InitialThreadProjection | null = null;

function Harness({ projection, releaseToLive }: HarnessProps) {
  latestProjection = useInitialThreadProjection(projection, { releaseToLive });
  return null;
}

afterEach(() => {
  cleanup();
  latestProjection = null;
});

const baseProjection: InitialThreadProjection = {
  sessionId: "session-1",
  turnsStamp: "1:turn",
  turns: [] as SessionTurn[],
  messagesStamp: "1:msg",
  messages: [] as Message[],
  eventsStamp: "1:evt",
  events: [] as SessionEvent[],
  toolsByTurnId: {} as Record<string, SessionTurnTool[]>,
  toolSummariesReady: false,
};

describe("initialThreadProjection helpers", () => {
  it("freezes loaded projections until the current session is released", () => {
    expect(
      shouldFreezeInitialThreadProjection({
        loaded: true,
        sessionId: "session-1",
        releasedSessionId: null,
      }),
    ).toBe(true);

    expect(
      shouldFreezeInitialThreadProjection({
        loaded: true,
        sessionId: "session-1",
        releasedSessionId: "session-1",
      }),
    ).toBe(false);
  });

  it("releases only once the current session has settled and has not yet been released", () => {
    expect(
      shouldReleaseInitialThreadProjection({
        loaded: true,
        sessionId: "session-1",
        settledSessionId: "session-1",
        releasedSessionId: null,
      }),
    ).toBe(true);

    expect(
      shouldReleaseInitialThreadProjection({
        loaded: true,
        sessionId: "session-1",
        settledSessionId: null,
        releasedSessionId: null,
      }),
    ).toBe(false);

    expect(
      shouldReleaseInitialThreadProjection({
        loaded: true,
        sessionId: "session-1",
        settledSessionId: "session-1",
        releasedSessionId: "session-1",
      }),
    ).toBe(false);
  });

  it("restores the bottom anchor only when switching from initial to coalesced while already at bottom", () => {
    expect(
      shouldRestoreBottomAnchorAfterProjectionRelease({
        previousSource: "initial",
        nextSource: "coalesced",
        wasAtBottom: true,
      }),
    ).toBe(true);

    expect(
      shouldRestoreBottomAnchorAfterProjectionRelease({
        previousSource: "coalesced",
        nextSource: "coalesced",
        wasAtBottom: true,
      }),
    ).toBe(false);

    expect(
      shouldRestoreBottomAnchorAfterProjectionRelease({
        previousSource: "initial",
        nextSource: "coalesced",
        wasAtBottom: false,
      }),
    ).toBe(false);
  });
});

describe("useInitialThreadProjection", () => {
  it("keeps the first projection frozen while release is disabled", () => {
    const first = { ...baseProjection, turnsStamp: "1:turn" };
    const second = {
      ...baseProjection,
      turnsStamp: "2:turn",
      messagesStamp: "2:msg",
      toolSummariesReady: true,
    };

    const { rerender } = render(<Harness projection={first} releaseToLive={false} />);

    expect(latestProjection).toBe(first);

    rerender(<Harness projection={second} releaseToLive={false} />);

    expect(latestProjection).toBe(first);
  });

  it("adopts the first non-empty projection before release so the session does not stay blank", () => {
    const empty = { ...baseProjection };
    const seeded = {
      ...baseProjection,
      turnsStamp: "2:turn",
      turns: [{ turn_id: "turn-1" } as SessionTurn],
    };

    const { rerender } = render(<Harness projection={empty} releaseToLive={false} />);

    expect(latestProjection).toBe(empty);

    rerender(<Harness projection={seeded} releaseToLive={false} />);

    expect(latestProjection).toBe(seeded);
  });

  it("releases to the latest live projection once release is enabled", () => {
    const first = { ...baseProjection };
    const second = {
      ...baseProjection,
      turnsStamp: "2:turn",
      turns: [{ turn_id: "turn-1" } as SessionTurn],
      toolSummariesReady: true,
    };

    const { rerender } = render(<Harness projection={first} releaseToLive={false} />);

    expect(latestProjection).toBe(first);

    rerender(<Harness projection={second} releaseToLive={true} />);

    expect(latestProjection).toBe(second);
  });

  it("resets the frozen projection when the session id changes", () => {
    const first = {
      ...baseProjection,
      sessionId: "session-1",
      turnsStamp: "1:turn",
    };
    const second = {
      ...baseProjection,
      sessionId: "session-2",
      turnsStamp: "2:turn",
    };

    const { rerender } = render(<Harness projection={first} releaseToLive={false} />);

    expect(latestProjection?.sessionId).toBe("session-1");

    rerender(<Harness projection={second} releaseToLive={false} />);

    expect(latestProjection).toBe(second);
    expect(latestProjection?.sessionId).toBe("session-2");
  });
});
