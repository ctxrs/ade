import React from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ArtifactsPane } from "./ArtifactsPane";

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
});
