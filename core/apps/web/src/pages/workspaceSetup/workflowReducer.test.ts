import { describe, expect, it } from "vitest";
import {
  createInitialWorkflowDraftState,
  makeDraftFieldSetter,
  workspaceSetupWorkflowReducer,
} from "./workflowReducer";

describe("workflowReducer", () => {
  it("updates a draft field through the shared setter helper", () => {
    const actions: Parameters<typeof workspaceSetupWorkflowReducer>[1][] = [];
    const dispatch = (action: Parameters<typeof workspaceSetupWorkflowReducer>[1]) => {
      actions.push(action);
    };
    const setSourcePath = makeDraftFieldSetter(dispatch, "sourcePath");
    setSourcePath("/tmp/repo");

    const next = actions.reduce(
      (state, action) => workspaceSetupWorkflowReducer(state, action),
      createInitialWorkflowDraftState(),
    );
    expect(next.sourcePath).toBe("/tmp/repo");
  });

  it("supports functional updates for draft fields", () => {
    const actions: Parameters<typeof workspaceSetupWorkflowReducer>[1][] = [];
    const dispatch = (action: Parameters<typeof workspaceSetupWorkflowReducer>[1]) => {
      actions.push(action);
    };
    const setPushOnSuccess = makeDraftFieldSetter(dispatch, "pushOnSuccess");
    setPushOnSuccess((prev) => !prev);

    const next = actions.reduce(
      (state, action) => workspaceSetupWorkflowReducer(state, action),
      createInitialWorkflowDraftState(),
    );
    expect(next.pushOnSuccess).toBe(true);
  });
});
