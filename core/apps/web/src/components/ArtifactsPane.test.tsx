import React from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Artifact } from "../api/client";
import { ArtifactsPane } from "./ArtifactsPane";

const makeArtifact = (overrides: Partial<Artifact> = {}): Artifact =>
  ({
    id: "artifact-1",
    session_id: "session-1",
    task_id: "task-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
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

function extractTransformNumber(transform: string, name: "translate" | "scale", axis?: "x" | "y"): number {
  if (name === "scale") {
    const match = /scale\(([-+\d.]+)\)/.exec(transform);
    return match ? Number(match[1]) : Number.NaN;
  }
  const match = /translate\(([-+\d.]+)px(?:,\s*|\s+)([-+\d.]+)px\)/.exec(transform);
  if (!match) return Number.NaN;
  return Number(axis === "y" ? match[2] : match[1]);
}

function openImageViewer() {
  render(
    <ArtifactsPane
      artifacts={[
        makeArtifact({
          name: "sample.png",
          absolute_path: "/tmp/sample.png",
          mime_type: "image/png",
          bytes: 2048,
        }),
      ]}
    />,
  );
  fireEvent.click(screen.getByText("sample.png"));
  const body = document.querySelector(".wb-artifact-modal-body") as HTMLDivElement;
  const image = document.querySelector(".wb-artifact-modal-image") as HTMLImageElement;
  Object.defineProperty(body, "clientWidth", { configurable: true, value: 300 });
  Object.defineProperty(body, "clientHeight", { configurable: true, value: 200 });
  body.getBoundingClientRect = () =>
    ({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 300,
      bottom: 200,
      width: 300,
      height: 200,
      toJSON: () => ({}),
    }) as DOMRect;
  Object.defineProperty(image, "clientWidth", { configurable: true, value: 200 });
  Object.defineProperty(image, "clientHeight", { configurable: true, value: 100 });
  fireEvent.load(image);
  return { body, image };
}

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

    const headings = await screen.findAllByText("Heading");
    expect(headings).toHaveLength(2);
    const bodyText = screen.getAllByText("Some artifact text.");
    expect(bodyText).toHaveLength(2);
  });

  it("renders markdown artifacts as markdown in the inline preview", async () => {
    mockTextFetch({ text: "## Inline Heading\n\nPreview body text." });

    render(
      <ArtifactsPane
        artifacts={[
          makeArtifact({
            name: "preview.md",
            mime_type: "text/markdown",
            absolute_path: "/tmp/preview.md",
          }),
        ]}
      />,
    );

    expect(await screen.findByText("Inline Heading")).toBeInTheDocument();
    expect(screen.getByText("Preview body text.")).toBeInTheDocument();
    expect(screen.queryByText("## Inline Heading")).not.toBeInTheDocument();
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

  it("keeps wheel zoom changes bounded for image artifacts", () => {
    const { body, image } = openImageViewer();

    fireEvent.wheel(body, { deltaY: -400, clientX: 150, clientY: 100 });

    expect(extractTransformNumber(image.style.transform, "scale")).toBeLessThan(1.2);
    expect(extractTransformNumber(image.style.transform, "scale")).toBeGreaterThan(1);
  });

  it("keeps drag transforms finite while zoomed", async () => {
    const { body, image } = openImageViewer();

    Object.defineProperty(body, "setPointerCapture", { configurable: true, value: vi.fn() });
    Object.defineProperty(body, "releasePointerCapture", { configurable: true, value: vi.fn() });
    Object.defineProperty(body, "hasPointerCapture", { configurable: true, value: vi.fn(() => true) });

    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    fireEvent.click(screen.getByRole("button", { name: "Zoom in" }));
    await waitFor(() => {
      expect(extractTransformNumber(image.style.transform, "scale")).toBeGreaterThan(1.7);
    });

    fireEvent.pointerDown(body, { pointerId: 1, clientX: 100, clientY: 100 });
    expect(body).toHaveClass("wb-artifact-dragging");

    fireEvent.pointerMove(body, { pointerId: 1, clientX: 260, clientY: 100 });
    await waitFor(() => {
      expect(image.style.transform).not.toContain("NaN");
      expect(Number.isFinite(extractTransformNumber(image.style.transform, "translate", "x"))).toBe(true);
      expect(Number.isFinite(extractTransformNumber(image.style.transform, "translate", "y"))).toBe(true);
    });

    fireEvent.pointerUp(body, { pointerId: 1, clientX: 260, clientY: 100 });
    expect(body).not.toHaveClass("wb-artifact-dragging");
  });
});
