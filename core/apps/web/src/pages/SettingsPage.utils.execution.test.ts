import { describe, expect, it } from "vitest";

import {
  MIN_MACHINE_IDLE_SHUTDOWN_SECONDS,
  canSaveSandboxMachineSettings,
} from "./SettingsPage";
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

  it("ignores display-only resolved machine memory from public settings", () => {
    const baseSettings = {
      mode: "host" as const,
      container: {
        runtime: "podman" as const,
        mount_mode: "host_mounted" as const,
        network_mode: "llm_only" as const,
        allowlist: [],
        image: null,
        machine: {
          memory_profile: "balanced" as const,
          custom_memory_mb: null,
          idle_shutdown_seconds: 900,
          host_pressure_swap_threshold_mb: 1024,
        },
      },
    };
    const publicSettingsWithDisplayOnlyFields: typeof baseSettings & {
      container: typeof baseSettings.container & {
        machine: typeof baseSettings.container.machine & {
          target_memory_mb: number;
        };
      };
    } = {
      ...baseSettings,
      container: {
        ...baseSettings.container,
        machine: {
          ...baseSettings.container.machine,
          target_memory_mb: 4096,
        },
      },
    };

    expect(executionSettingsStableKey(publicSettingsWithDisplayOnlyFields)).toBe(
      executionSettingsStableKey(baseSettings),
    );
  });
});

describe("canSaveSandboxMachineSettings", () => {
  it("blocks save and autosave for idle shutdown values below the 60-second minimum", () => {
    for (let idleSeconds = 1; idleSeconds < MIN_MACHINE_IDLE_SHUTDOWN_SECONDS; idleSeconds += 1) {
      expect(
        canSaveSandboxMachineSettings({
          machineIdleShutdownSeconds: String(idleSeconds),
          machineHostPressureSwapThresholdMb: "1024",
        }),
      ).toBe(false);
    }
  });

  it("allows save again at the 60-second minimum", () => {
    expect(
      canSaveSandboxMachineSettings({
        machineIdleShutdownSeconds: String(MIN_MACHINE_IDLE_SHUTDOWN_SECONDS),
        machineHostPressureSwapThresholdMb: "1024",
      }),
    ).toBe(true);
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
