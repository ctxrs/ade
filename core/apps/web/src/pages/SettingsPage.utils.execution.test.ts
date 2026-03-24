import { describe, expect, it } from "vitest";

import {
  MIN_MACHINE_IDLE_SHUTDOWN_SECONDS,
  canSaveSandboxMachineSettings,
  defaultContainerMountMode,
  defaultContainerRuntimeKind,
  defaultExecutionSettings,
  normalizeExecutionSettings,
} from "./settings/sandboxExecutionSettings";
import {
  desktopEditorSettingsEqual,
  executionSettingsStableKey,
  isContainerizedEnvironment,
  normalizeDesktopEditorSettings,
  promptAutosaveStatusLabel,
  worktreeBootstrapFormFromConfig,
} from "./SettingsPage.utils";

describe("isContainerizedEnvironment", () => {
  it("returns true for sandbox environments", () => {
    expect(isContainerizedEnvironment("sandbox")).toBe(true);
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
          mount_mode: "disk_isolated",
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
          mount_mode: "disk_isolated",
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
        mount_mode: "disk_isolated" as const,
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

describe("defaultExecutionSettings", () => {
  it("uses the daemon-reported runtime default when present", () => {
    expect(defaultContainerRuntimeKind("avf_linux_vm")).toBe("avf_linux_vm");
    expect(defaultExecutionSettings("avf_linux_vm").container.runtime).toBe("avf_linux_vm");
    expect(defaultContainerMountMode("avf_linux_vm")).toBe("disk_isolated");
    expect(defaultExecutionSettings("avf_linux_vm").container.mount_mode).toBe("disk_isolated");
  });

  it("keeps podman as the fallback when the daemon has not reported a preferred runtime", () => {
    expect(defaultContainerRuntimeKind()).toBe("podman");
    expect(defaultContainerRuntimeKind(null)).toBe("podman");
    expect(defaultExecutionSettings().container.runtime).toBe("podman");
    expect(defaultContainerMountMode("podman")).toBe("disk_isolated");
    expect(defaultExecutionSettings().container.mount_mode).toBe("disk_isolated");
  });

  it("keeps sandbox execution settings disk-isolated", () => {
    expect(
      normalizeExecutionSettings({
        mode: "container",
        container: {
          runtime: "avf_linux_vm",
          mount_mode: "disk_isolated",
          network_mode: "llm_only",
          allowlist: [],
          image: null,
          machine: {
            memory_profile: "economy",
            custom_memory_mb: null,
            idle_shutdown_seconds: 3600,
            host_pressure_swap_threshold_mb: 1024,
          },
        },
      }).container.mount_mode,
    ).toBe("disk_isolated");
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
