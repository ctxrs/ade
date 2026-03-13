import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
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

const originalFetch = global.fetch;

function mockTextFetch(opts: { ok?: boolean; status?: number; text?: string } = {}) {
  const response = {
    ok: opts.ok ?? true,
    status: opts.status ?? 200,
    text: async () => opts.text ?? "",
  } as Response;
  global.fetch = vi.fn(async () => response);
}

afterEach(() => {
  global.fetch = originalFetch;
});

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

  it("renders an inline text preview for previewable text artifacts", async () => {
    mockTextFetch({ text: "line 1\nline 2\nline 3" });

    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "notes.txt",
            mime_type: "text/plain",
            absolute_path: "/tmp/notes.txt",
          }),
        ]}
      />,
    );

    expect(await screen.findByText(/line 2/, { selector: "pre" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
  });

  it("renders markdown artifacts in the viewer", async () => {
    mockTextFetch({ text: "# Heading\n\nSome artifact text." });

    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "notes.md",
            mime_type: "text/markdown",
            absolute_path: "/tmp/notes.md",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle("/tmp/notes.md"));

    expect(await screen.findByText("Heading")).toBeInTheDocument();
    expect(screen.getByText("Some artifact text.")).toBeInTheDocument();
  });

  it("renders json artifacts as text in the viewer", async () => {
    mockTextFetch({ text: '{\n  "ok": true\n}' });

    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "report.json",
            mime_type: "application/json",
            absolute_path: "/tmp/report.json",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle("/tmp/report.json"));

    const jsonPreviews = await screen.findAllByText(/"ok": true/, { selector: "pre" });
    expect(jsonPreviews).toHaveLength(2);
  });

  it("shows an inline error when text artifact loading fails", async () => {
    mockTextFetch({ ok: false, status: 500 });

    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "broken.txt",
            mime_type: "text/plain",
            absolute_path: "/tmp/broken.txt",
          }),
        ]}
      />,
    );

    fireEvent.click(screen.getByTitle("/tmp/broken.txt"));

    expect(await screen.findByRole("alert")).toHaveTextContent("Failed to load artifact (500).");
  });
});
