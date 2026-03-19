import { describe, expect, it } from "vitest";

import {
  desktopEditorSettingsEqual,
  executionSettingsStableKey,
  isContainerizedEnvironment,
  normalizeDesktopEditorSettings,
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
    expect(promptAutosaveStatusLabel("saving")).toBe("");
    expect(promptAutosaveStatusLabel("saved")).toBe("Saved");
    expect(promptAutosaveStatusLabel("error")).toBe("Save failed");
    expect(promptAutosaveStatusLabel("idle")).toBe("");
  });
});

describe("normalizeDesktopEditorSettings", () => {
  it("trims fields and clears custom commands for non-custom targets", () => {
    expect(
      normalizeDesktopEditorSettings({
        target: "cursor",
        custom_command: " code --goto {path}:{line}:{col} ",
        remote_authority: " ssh-remote+ctx ",
      }),
    ).toEqual({
      target: "cursor",
      custom_command: null,
      remote_authority: "ssh-remote+ctx",
    });
  });
});

describe("desktopEditorSettingsEqual", () => {
  it("compares editor settings by their normalized persisted values", () => {
    expect(
      desktopEditorSettingsEqual(
        {
          target: "cursor",
          custom_command: " code --goto {path}:{line}:{col} ",
          remote_authority: " ssh-remote+ctx ",
        },
        {
          target: "cursor",
          custom_command: null,
          remote_authority: "ssh-remote+ctx",
        },
      ),
    ).toBe(true);
  });
});

describe("executionSettingsStableKey", () => {
  it("matches for equivalent execution payloads across fresh objects", () => {
    expect(
      executionSettingsStableKey({
        mode: "host",
        container: {
          runtime: "podman",
          mount_mode: "host_mounted",
          network_mode: "llm_only",
          allowlist: [],
          image: null,
          machine: {
            memory_profile: "balanced",
            custom_memory_mb: null,
            idle_shutdown_seconds: 900,
            host_pressure_swap_threshold_mb: 1024,
          },
        },
      }),
    ).toBe(
      executionSettingsStableKey({
        mode: "host",
        container: {
          runtime: "podman",
          mount_mode: "host_mounted",
          network_mode: "llm_only",
          allowlist: [],
          image: null,
          machine: {
            memory_profile: "balanced",
            custom_memory_mb: null,
            idle_shutdown_seconds: 900,
            host_pressure_swap_threshold_mb: 1024,
          },
        },
      }),
    );
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
