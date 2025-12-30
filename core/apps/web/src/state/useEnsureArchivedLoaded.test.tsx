import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render } from "@testing-library/react";

import { useEnsureArchivedLoaded } from "./useEnsureArchivedLoaded";

type HarnessProps = {
  archivedCollapsed: boolean;
  archivedLoaded: boolean;
  fetchState: "idle" | "loading" | "error";
  ensureArchivedLoaded: () => void;
};

function Harness(props: HarnessProps) {
  useEnsureArchivedLoaded(props);
  return null;
}

afterEach(() => cleanup());

describe("useEnsureArchivedLoaded", () => {
  it("loads archived items when expanded and not loaded", () => {
    const ensureArchivedLoaded = vi.fn();
    render(
      <Harness
        archivedCollapsed={false}
        archivedLoaded={false}
        fetchState="idle"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    expect(ensureArchivedLoaded).toHaveBeenCalledTimes(1);
  });

  it("skips when collapsed or already loading", () => {
    const ensureArchivedLoaded = vi.fn();
    const { rerender } = render(
      <Harness
        archivedCollapsed
        archivedLoaded={false}
        fetchState="idle"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    rerender(
      <Harness
        archivedCollapsed={false}
        archivedLoaded={false}
        fetchState="loading"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    expect(ensureArchivedLoaded).toHaveBeenCalledTimes(0);
  });

  it("retries when expanded after an error", () => {
    const ensureArchivedLoaded = vi.fn();
    render(
      <Harness
        archivedCollapsed={false}
        archivedLoaded={false}
        fetchState="error"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    expect(ensureArchivedLoaded).toHaveBeenCalledTimes(1);
  });

  it("retries when the archived cache is cleared while expanded", () => {
    const ensureArchivedLoaded = vi.fn();
    const { rerender } = render(
      <Harness
        archivedCollapsed={false}
        archivedLoaded
        fetchState="idle"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    rerender(
      <Harness
        archivedCollapsed={false}
        archivedLoaded={false}
        fetchState="idle"
        ensureArchivedLoaded={ensureArchivedLoaded}
      />,
    );
    expect(ensureArchivedLoaded).toHaveBeenCalledTimes(1);
  });
});
