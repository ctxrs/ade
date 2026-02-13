import type {
  ExecutionLaunchLogLine,
  ExecutionLaunchPhase,
  ExecutionLaunchPhaseStatus,
  ExecutionLaunchSnapshot,
} from "../../api/client";

const LAUNCH_LOG_MAX = 400;

export const launchPhaseLabel = (phase?: ExecutionLaunchPhase | null): string => {
  if (!phase) return "Preparing";
  switch (phase) {
    case "machine_check":
      return "Machine check";
    case "machine_start_or_init":
      return "Machine start/init";
    case "image_check":
      return "Image check";
    case "image_load":
      return "Image load";
    case "container_check":
      return "Container check";
    case "container_start_or_create":
      return "Container start/create";
    case "runtime_network_setup":
      return "Network setup";
    case "ready":
      return "Ready";
    default:
      return phase;
  }
};

export const parseUtcMs = (value?: string | null): number | null => {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
};

export const phaseEntryForCurrent = (snapshot: ExecutionLaunchSnapshot): ExecutionLaunchPhaseStatus | null => {
  if (!snapshot.current_phase) return null;
  for (let i = snapshot.phases.length - 1; i >= 0; i -= 1) {
    if (snapshot.phases[i].phase === snapshot.current_phase) {
      return snapshot.phases[i];
    }
  }
  return null;
};

export const mergeLaunchLogs = (
  current: ExecutionLaunchLogLine[],
  incoming: ExecutionLaunchLogLine[],
): ExecutionLaunchLogLine[] => {
  if (!incoming.length) return current.slice(-LAUNCH_LOG_MAX);
  const bySeq = new Map<number, ExecutionLaunchLogLine>();
  for (const line of current) bySeq.set(line.seq, line);
  for (const line of incoming) bySeq.set(line.seq, line);
  const merged = Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
  return merged.slice(-LAUNCH_LOG_MAX);
};

export const launchErrorFromSnapshot = (snapshot: ExecutionLaunchSnapshot): string => {
  const phase = launchPhaseLabel(snapshot.current_phase);
  const message = String(snapshot.error ?? "").trim();
  if (!message) return `Workspace launch failed during ${phase}.`;
  return `${phase}: ${message}`;
};

export const formatLaunchElapsed = (ms: number | null): string => {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return "0s";
  const rounded = Math.floor(ms / 1000);
  const minutes = Math.floor(rounded / 60);
  const seconds = rounded % 60;
  if (minutes <= 0) return `${seconds}s`;
  return `${minutes}m ${seconds}s`;
};

export const formatLaunchTime = (ts: string): string => {
  const value = parseUtcMs(ts);
  if (value === null) return ts;
  const date = new Date(value);
  return date.toLocaleTimeString([], { hour12: false });
};
