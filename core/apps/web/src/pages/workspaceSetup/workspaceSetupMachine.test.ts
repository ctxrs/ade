import { describe, expect, it } from "vitest";
import type { WizardRoutePlan } from "./wizardFlow";
import {
  createInitialWorkspaceSetupMachineState,
  workspaceSetupMachineReducer,
  type WorkspaceSetupMachineSnapshot,
} from "./workspaceSetupMachine";

const routePlanFixture = (overrides?: Partial<WizardRoutePlan>): WizardRoutePlan => ({
  targetKey: "local|container",
  containerSelection: "disk-isolated",
  includeHarnessDownloads: false,
  includeAuthImport: false,
  includeTitling: false,
  ...overrides,
});

const snapshotFixture = (
  overrides?: Partial<WorkspaceSetupMachineSnapshot>,
): WorkspaceSetupMachineSnapshot => ({
  stepKey: "location",
  routePlan: null,
  locationSelection: "local",
  harnessInstallBusy: false,
  harnessInstallError: null,
  selectedHarnessReadyToStartCount: 0,
  selectedHarnessRunningCount: 0,
  selectedHarnessFailedCount: 0,
  titlingMode: "unset",
  titlingRemoteValid: false,
  ...overrides,
});

describe("workspaceSetupMachine", () => {
  it("requests remote verification before advancing from the location step", () => {
    const state = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "next_requested",
        snapshot: snapshotFixture({
          stepKey: "location",
          locationSelection: "remote",
        }),
      },
    );

    expect(state.pendingEffects).toEqual([
      {
        id: 1,
        kind: "run_command",
        command: { kind: "verify_remote_connection" },
      },
    ]);

    const completed = workspaceSetupMachineReducer(state, {
      type: "command_completed",
      effectId: 1,
      result: {
        kind: "verify_remote_connection",
        connected: true,
      },
    });

    expect(completed.pendingEffects.at(-1)).toEqual({
      id: 2,
      kind: "go_to_step",
      stepKey: "container",
    });
  });

  it("routes container planning through an explicit command and advances to the planned boundary step", () => {
    const planning = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "option_selected",
        stepKey: "container",
        optionId: "disk-isolated",
        snapshot: snapshotFixture({
          stepKey: "container",
        }),
      },
    );

    expect(planning.pendingEffects).toEqual([
      {
        id: 1,
        kind: "run_command",
        command: {
          kind: "ensure_route_plan",
          containerSelectionOverride: "disk-isolated",
        },
      },
    ]);

    const advanced = workspaceSetupMachineReducer(planning, {
      type: "command_completed",
      effectId: 1,
      result: {
        kind: "ensure_route_plan",
        routePlan: routePlanFixture({
          includeHarnessDownloads: true,
        }),
      },
    });

    expect(advanced.pendingEffects.at(-1)).toEqual({
      id: 2,
      kind: "go_to_step",
      stepKey: "harness-downloads",
    });
  });

  it("skips harness downloads immediately when nothing remains startable", () => {
    const state = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "next_requested",
        snapshot: snapshotFixture({
          stepKey: "harness-downloads",
          routePlan: routePlanFixture({
            includeAuthImport: true,
          }),
          selectedHarnessReadyToStartCount: 0,
        }),
      },
    );

    expect(state.pendingEffects).toEqual([
      {
        id: 1,
        kind: "go_to_step",
        stepKey: "auth-import",
      },
    ]);
  });

  it("starts harness downloads in the background when skip is requested", () => {
    const state = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "skip_harness_downloads_requested",
        snapshot: snapshotFixture({
          stepKey: "harness-downloads",
          routePlan: routePlanFixture({
            includeAuthImport: true,
          }),
        }),
      },
    );

    expect(state.pendingEffects).toEqual([
      {
        id: 1,
        kind: "go_to_step",
        stepKey: "auth-import",
      },
      {
        id: 2,
        kind: "run_command",
        command: {
          kind: "advance_harness_downloads",
          clearSelections: true,
        },
      },
    ]);
  });

  it("surfaces titling validation errors before attempting persistence", () => {
    const state = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "next_requested",
        snapshot: snapshotFixture({
          stepKey: "session-titling",
          routePlan: routePlanFixture({
            includeTitling: true,
          }),
          titlingMode: "remote",
          titlingRemoteValid: false,
        }),
      },
    );

    expect(state.pendingEffects).toEqual([
      {
        id: 1,
        kind: "set_titling_persist_error",
        message: "Remote titling needs base URL, API key, and model.",
      },
    ]);
  });

  it("auto-advances once selected harness installs fail after background progress settles", () => {
    const state = workspaceSetupMachineReducer(
      createInitialWorkspaceSetupMachineState(),
      {
        type: "provisioning_snapshot_changed",
        snapshot: snapshotFixture({
          stepKey: "harness-downloads",
          routePlan: routePlanFixture({
            includeTitling: true,
          }),
          selectedHarnessReadyToStartCount: 0,
          selectedHarnessRunningCount: 0,
          selectedHarnessFailedCount: 1,
        }),
      },
    );

    expect(state.pendingEffects).toEqual([
      {
        id: 1,
        kind: "go_to_step",
        stepKey: "session-titling",
      },
    ]);
  });
});
