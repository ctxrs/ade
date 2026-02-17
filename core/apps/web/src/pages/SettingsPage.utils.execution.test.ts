import { describe, expect, it } from "vitest";

import {
  isContainerizedEnvironment,
  promptAutosaveStatusLabel,
  worktreeBootstrapFormFromConfig,
} from "./SettingsPage.utils";

describe("isContainerizedEnvironment", () => {
  it("returns true for container environments", () => {
    expect(isContainerizedEnvironment("container_host_mounted")).toBe(true);
    expect(isContainerizedEnvironment("container_disk_isolated")).toBe(true);
  });

  it("returns false for host and empty environments", () => {
    expect(isContainerizedEnvironment("host")).toBe(false);
    expect(isContainerizedEnvironment(null)).toBe(false);
    expect(isContainerizedEnvironment(undefined)).toBe(false);
  });
});

describe("promptAutosaveStatusLabel", () => {
  it("maps statuses to user-facing labels", () => {
    expect(promptAutosaveStatusLabel("pending")).toBe("Pending changes");
    expect(promptAutosaveStatusLabel("saving")).toBe("Saving...");
    expect(promptAutosaveStatusLabel("saved")).toBe("Saved");
    expect(promptAutosaveStatusLabel("error")).toBe("Save failed");
    expect(promptAutosaveStatusLabel("idle")).toBe("");
  });
});

describe("worktreeBootstrapFormFromConfig", () => {
  it("maps missing config to blank defaults", () => {
    expect(worktreeBootstrapFormFromConfig(null)).toEqual({
      setup_command: "",
      timeout_sec: "",
      wait_for_completion: false,
    });
  });

  it("maps configured values into editable form fields", () => {
    expect(
      worktreeBootstrapFormFromConfig({
        setup_command: "pnpm install",
        timeout_sec: 120,
        wait_for_completion: true,
      }),
    ).toEqual({
      setup_command: "pnpm install",
      timeout_sec: "120",
      wait_for_completion: true,
    });
  });
});
