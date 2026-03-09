import type { Dispatch, SetStateAction } from "react";
import {
  createInitialWorkspaceSetupDraftState,
  type WorkspaceSetupDraftState,
} from "./workflowTypes";

type WorkspaceSetupWorkflowAction = {
  type: "update";
  updater: (state: WorkspaceSetupDraftState) => WorkspaceSetupDraftState;
};

export const workspaceSetupWorkflowReducer = (
  state: WorkspaceSetupDraftState,
  action: WorkspaceSetupWorkflowAction,
): WorkspaceSetupDraftState => {
  switch (action.type) {
    case "update":
      return action.updater(state);
    default:
      return state;
  }
};

const resolveStateAction = <T>(prev: T, next: SetStateAction<T>): T =>
  typeof next === "function" ? (next as (prevState: T) => T)(prev) : next;

export const makeDraftFieldSetter = <K extends keyof WorkspaceSetupDraftState>(
  dispatch: Dispatch<WorkspaceSetupWorkflowAction>,
  field: K,
) =>
  (value: SetStateAction<WorkspaceSetupDraftState[K]>) => {
    dispatch({
      type: "update",
      updater: (state) => ({
        ...state,
        [field]: resolveStateAction(state[field], value),
      }),
    });
  };

export const createInitialWorkflowDraftState = createInitialWorkspaceSetupDraftState;
