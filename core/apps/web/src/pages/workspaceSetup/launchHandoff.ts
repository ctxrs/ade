import type {
  ExecutionLaunchLogLine,
  ExecutionLaunchSnapshot,
  ExecutionLaunchStreamEvent,
} from "../../api/client";
import {
  buildExecutionLaunchWsUrl,
  getExecutionLaunchStatus,
  startWorkspaceSetupLaunchHandoff as requestWorkspaceSetupLaunchHandoff,
  startWorkspaceSetupRuntimePrewarm as requestWorkspaceSetupRuntimePrewarm,
} from "../../api/client";
import {
  launchErrorFromSnapshot as formatLaunchErrorFromSnapshot,
  mergeLaunchLogs,
} from "./launchProgress";
import { messageFromError } from "./wizardTypes";

type LaunchCallbacks = {
  applySnapshot: (snapshot: ExecutionLaunchSnapshot) => void;
  appendLine: (line: ExecutionLaunchLogLine) => void;
};

export const startWorkspaceSetupLaunchHandoff = (workspaceId: string) =>
  requestWorkspaceSetupLaunchHandoff(workspaceId);

export const startWorkspaceSetupRuntimePrewarm = () =>
  requestWorkspaceSetupRuntimePrewarm();

export const launchErrorFromSnapshot = (snapshot: ExecutionLaunchSnapshot): string =>
  formatLaunchErrorFromSnapshot(snapshot);

export const waitForLaunchHandoffTerminal = async (
  initial: ExecutionLaunchSnapshot,
  callbacks: LaunchCallbacks,
): Promise<void> => {
  callbacks.applySnapshot(initial);

  if (initial.state === "ready") return;
  if (initial.state === "error") throw new Error(launchErrorFromSnapshot(initial));

  await new Promise<void>((resolve, reject) => {
    let settled = false;
    const ws = new WebSocket(buildExecutionLaunchWsUrl(initial.job_id));

    const settle = (error?: Error) => {
      if (settled) return;
      settled = true;
      ws.close();
      if (error) reject(error);
      else resolve();
    };

    ws.onmessage = (event) => {
      let parsed: ExecutionLaunchStreamEvent | null = null;
      try {
        parsed = JSON.parse(String(event.data ?? "")) as ExecutionLaunchStreamEvent;
      } catch {
        return;
      }
      if (!parsed) return;
      if (parsed.type === "launch_log") {
        callbacks.appendLine(parsed.line);
        return;
      }
      if (parsed.type === "launch_snapshot") {
        callbacks.applySnapshot(parsed.snapshot);
        return;
      }
      if (parsed.type === "launch_complete") {
        callbacks.applySnapshot(parsed.snapshot);
        settle();
        return;
      }
      if (parsed.type === "launch_error") {
        callbacks.applySnapshot(parsed.snapshot);
        settle(new Error(launchErrorFromSnapshot(parsed.snapshot)));
      }
    };

    ws.onclose = () => {
      if (settled) return;
      getExecutionLaunchStatus(initial.job_id)
        .then((latest) => {
          callbacks.applySnapshot(latest);
          if (latest.state === "ready") {
            settle();
          } else if (latest.state === "error") {
            settle(new Error(launchErrorFromSnapshot(latest)));
          } else {
            settle(new Error("Lost workspace launch stream before setup finished."));
          }
        })
        .catch((error: unknown) => {
          settle(new Error(messageFromError(error)));
        });
    };
  });
};

export const mergeWorkspaceSetupLaunchLogs = (
  previous: ExecutionLaunchLogLine[],
  nextLines: ExecutionLaunchLogLine[],
) => mergeLaunchLogs(previous, nextLines);
