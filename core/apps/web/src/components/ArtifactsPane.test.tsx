import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { Artifact } from "../api/client";
import { ArtifactsPane } from "./ArtifactsPane";

const makeArtifact = (overrides: Partial<Artifact> = {}): Artifact =>
  ({
    id: "artifact-1",
    session_id: "session-1",
    task_id: "task-1",
    turn_id: null,
    name: "artifact",
    mime_type: "text/plain",
    bytes: 128,
    absolute_path: "/tmp/artifact.txt",
    missing: false,
    created_at: "2026-03-13T00:00:00.000Z",
    ...overrides,
  }) as Artifact;

describe("ArtifactsPane", () => {
  it("renders an explicit load error with retry affordance", () => {
    const onRetry = vi.fn();

    render(
      <ArtifactsPane
        artifacts={[]}
        error="Failed to load artifacts: daemon offline"
        onRetry={onRetry}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Failed to load artifacts: daemon offline");
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it("keeps the existing empty state when there is no load error", () => {
    render(<ArtifactsPane artifacts={[]} />);
    expect(screen.getByText("No artifacts yet.")).toBeInTheDocument();
  });

  it("opens the viewer for previewable artifacts", () => {
    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "chart.png",
            mime_type: "image/png",
            absolute_path: "/tmp/chart.png",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle("/tmp/chart.png"));

    expect(screen.getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("does not open the viewer for unsupported artifacts", () => {
    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "report.csv",
            mime_type: "text/csv",
            absolute_path: "/tmp/report.csv",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle("/tmp/report.csv"));

    expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
  });
});
