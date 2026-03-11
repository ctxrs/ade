import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";
import { SettingsShell } from "./SettingsShell";

const sidebarSections = [
  {
    id: "general",
    label: "General",
    group: "main",
  },
] as const;

describe("SettingsShell", () => {
  it("does not show the transient saving subtitle", () => {
    render(
      <MemoryRouter>
        <SettingsShell
          backLink={{ to: "/", label: "Back" }}
          query=""
          onQueryChange={() => {}}
          sidebarSections={[...sidebarSections]}
          active="general"
          onSectionChange={() => {}}
          headerLabel="General"
          saveError={null}
        >
          <div>Content</div>
        </SettingsShell>
      </MemoryRouter>,
    );

    expect(screen.queryByText("Saving…")).not.toBeInTheDocument();
    expect(screen.queryByText("Saving...")).not.toBeInTheDocument();
  });

  it("keeps the not-saved subtitle when there is an error", () => {
    render(
      <MemoryRouter>
        <SettingsShell
          backLink={{ to: "/", label: "Back" }}
          query=""
          onQueryChange={() => {}}
          sidebarSections={[...sidebarSections]}
          active="general"
          onSectionChange={() => {}}
          headerLabel="General"
          saveError="Save failed"
        >
          <div>Content</div>
        </SettingsShell>
      </MemoryRouter>,
    );

    expect(screen.getByText("Not saved")).toBeInTheDocument();
  });
});
