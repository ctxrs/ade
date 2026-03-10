import type {
  ExecutionLaunchDownloadStatus,
  ExecutionLaunchLogLine,
  ExecutionLaunchPhase,
  ExecutionLaunchPhaseStatus,
  ExecutionLaunchSnapshot,
} from "../../api/client";

const LAUNCH_LOG_MAX = 400;

export type WorkspaceSetupLaunchLogLine = ExecutionLaunchLogLine & {
  phaseLabel: string;
  timeLabel: string;
};

export const launchPhaseLabel = (phase?: ExecutionLaunchPhase | null): string => {
  if (!phase) return "Preparing";
  switch (phase) {
    case "artifact_download":
      return "Downloading required artifacts";
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

const capitalizeLabel = (value: string): string =>
  value ? value.charAt(0).toUpperCase() + value.slice(1) : value;

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

const decorateLaunchLogLine = (line: ExecutionLaunchLogLine): WorkspaceSetupLaunchLogLine => ({
  ...line,
  phaseLabel: launchPhaseLabel(line.phase),
  timeLabel: formatLaunchTime(line.ts),
});

const isStrictlyIncreasingBySeq = (lines: ExecutionLaunchLogLine[]): boolean => {
  for (let i = 1; i < lines.length; i += 1) {
    if (lines[i].seq <= lines[i - 1].seq) return false;
  }
  return true;
};

export const mergeLaunchLogs = (
  current: WorkspaceSetupLaunchLogLine[],
  incoming: ExecutionLaunchLogLine[],
): WorkspaceSetupLaunchLogLine[] => {
  if (!incoming.length) return current.slice(-LAUNCH_LOG_MAX);

  const lastCurrent = current.length > 0 ? current[current.length - 1] : null;
  if (
    isStrictlyIncreasingBySeq(incoming)
    && (lastCurrent === null || incoming[0].seq > lastCurrent.seq)
  ) {
    return current.concat(incoming.map(decorateLaunchLogLine)).slice(-LAUNCH_LOG_MAX);
  }

  const bySeq = new Map<number, WorkspaceSetupLaunchLogLine>();
  for (const line of current) bySeq.set(line.seq, line);
  for (const line of incoming) bySeq.set(line.seq, decorateLaunchLogLine(line));
  const merged = Array.from(bySeq.values()).sort((a, b) => a.seq - b.seq);
  return merged.slice(-LAUNCH_LOG_MAX);
};

export const launchErrorFromSnapshot = (snapshot: ExecutionLaunchSnapshot): string => {
  const phase = launchPhaseLabel(snapshot.current_phase);
  const message = String(snapshot.error ?? "").trim();
  if (!message) return `Workspace launch failed during ${phase}.`;
  return `${phase}: ${message}`;
};

export const currentLaunchStepLabel = (snapshot: ExecutionLaunchSnapshot | null): string => {
  const raw = String(snapshot?.current_step_label ?? "").trim();
  if (raw) return capitalizeLabel(raw);
  return launchPhaseLabel(snapshot?.current_phase);
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

export const launchEtaRemainingMs = (
  snapshot: ExecutionLaunchSnapshot | null,
  nowMs: number,
): number | null => {
  if (!snapshot) return null;
  if (snapshot.state === "ready") return 0;
  if (snapshot.state === "error") return null;
  if (snapshot.eta_ms === null || snapshot.eta_ms === undefined) return null;
  if (!snapshot.active_download) {
    return snapshot.eta_ms > 0 ? snapshot.eta_ms : null;
  }
  const updatedAt = parseUtcMs(snapshot.updated_at);
  if (updatedAt === null) return Math.max(0, snapshot.eta_ms);
  return Math.max(0, snapshot.eta_ms - Math.max(0, nowMs - updatedAt));
};

export const formatLaunchRemaining = (ms: number | null): string => {
  if (ms === null || !Number.isFinite(ms) || ms < 0) return "Estimating remaining…";
  const totalSeconds = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  if (hours > 0) {
    return `${hours}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")} remaining`;
  }
  if (minutes > 0) {
    return `${minutes}:${String(seconds).padStart(2, "0")} remaining`;
  }
  return `${seconds}s remaining`;
};

const formatByteCount = (value: number): string => {
  if (!Number.isFinite(value) || value < 0) return "0 B";
  if (value >= 1024 ** 3) return `${(value / 1024 ** 3).toFixed(1)} GB`;
  if (value >= 1024 ** 2) return `${(value / 1024 ** 2).toFixed(1)} MB`;
  if (value >= 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${Math.floor(value)} B`;
};

export const formatLaunchDownloadSummary = (
  download: ExecutionLaunchDownloadStatus | null | undefined,
): string | null => {
  if (!download) return null;
  const bytes = download.total_bytes && download.total_bytes > 0
    ? `${formatByteCount(download.downloaded_bytes)} / ${formatByteCount(download.total_bytes)}`
    : formatByteCount(download.downloaded_bytes);
  const rate = download.bytes_per_sec && download.bytes_per_sec > 0
    ? ` · ${formatByteCount(download.bytes_per_sec)}/s`
    : "";
  return `${download.artifact} · ${bytes}${rate}`;
};
