import type {
  ExecutionLaunchLogLine,
  ExecutionLaunchPhase,
  ExecutionLaunchPhaseStatus,
  ExecutionLaunchSnapshot,
} from "../../api/client";

const LAUNCH_LOG_MAX = 400;
const ARTIFACT_ACQUISITION_PREPARATION_MS = 40_000;
const SHARED_VM_STARTUP_MS = 19_000;
const SANDBOX_SETUP_MS = 12_000;
const TOTAL_LAUNCH_BUDGET_MS =
  ARTIFACT_ACQUISITION_PREPARATION_MS + SHARED_VM_STARTUP_MS + SANDBOX_SETUP_MS;

type LaunchEtaBucket =
  | "artifact_acquisition_preparation"
  | "shared_vm_startup"
  | "sandbox_setup";

const LAUNCH_ETA_BUCKET_ORDER: LaunchEtaBucket[] = [
  "artifact_acquisition_preparation",
  "shared_vm_startup",
  "sandbox_setup",
];

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

export const launchElapsedMs = (
  snapshot: ExecutionLaunchSnapshot | null,
  nowMs: number,
): number | null => {
  if (!snapshot) return null;

  const startedAt = parseUtcMs(snapshot.started_at) ?? parseUtcMs(snapshot.created_at);
  if (startedAt !== null) {
    return Math.max(0, nowMs - startedAt);
  }

  return null;
};

export const formatLaunchTime = (ts: string): string => {
  const value = parseUtcMs(ts);
  if (value === null) return ts;
  const date = new Date(value);
  return date.toLocaleTimeString([], { hour12: false });
};

const etaBucketForPhase = (
  phase?: ExecutionLaunchPhase | null,
): LaunchEtaBucket | null => {
  switch (phase) {
    case "artifact_download":
    case "machine_check":
      return "artifact_acquisition_preparation";
    case "machine_start_or_init":
      return "shared_vm_startup";
    case "image_check":
    case "image_load":
    case "container_check":
    case "container_start_or_create":
    case "runtime_network_setup":
      return "sandbox_setup";
    default:
      return null;
  }
};

const etaBucketBudgetMs = (bucket: LaunchEtaBucket): number => {
  switch (bucket) {
    case "artifact_acquisition_preparation":
      return ARTIFACT_ACQUISITION_PREPARATION_MS;
    case "shared_vm_startup":
      return SHARED_VM_STARTUP_MS;
    case "sandbox_setup":
      return SANDBOX_SETUP_MS;
  }
};

const remainingDownstreamBucketBudgetMs = (bucket: LaunchEtaBucket): number => {
  switch (bucket) {
    case "artifact_acquisition_preparation":
      return SHARED_VM_STARTUP_MS + SANDBOX_SETUP_MS;
    case "shared_vm_startup":
      return SANDBOX_SETUP_MS;
    case "sandbox_setup":
      return 0;
  }
};

const phaseFinishedAtMs = (phase: ExecutionLaunchPhaseStatus): number | null => {
  const completedAt =
    "completed_at" in phase && typeof phase.completed_at === "string"
      ? phase.completed_at
      : "finished_at" in phase && typeof phase.finished_at === "string"
        ? phase.finished_at
        : null;
  return parseUtcMs(completedAt);
};

const bucketHasIncompletePhase = (
  snapshot: ExecutionLaunchSnapshot,
  bucket: LaunchEtaBucket,
): boolean => {
  return snapshot.phases.some((phase) => {
    if (etaBucketForPhase(phase.phase) !== bucket) return false;
    return phaseFinishedAtMs(phase) === null && phase.status !== "completed";
  });
};

const bucketHasPhaseHistory = (
  snapshot: ExecutionLaunchSnapshot,
  bucket: LaunchEtaBucket,
): boolean => {
  return snapshot.phases.some((phase) => etaBucketForPhase(phase.phase) === bucket);
};

const hasLaterBucketPhaseHistory = (
  snapshot: ExecutionLaunchSnapshot,
  bucket: LaunchEtaBucket,
): boolean => {
  const bucketIndex = LAUNCH_ETA_BUCKET_ORDER.indexOf(bucket);
  if (bucketIndex < 0) return false;
  return snapshot.phases.some((phase) => {
    const phaseBucket = etaBucketForPhase(phase.phase);
    return phaseBucket !== null && LAUNCH_ETA_BUCKET_ORDER.indexOf(phaseBucket) > bucketIndex;
  });
};

const bucketIsComplete = (
  snapshot: ExecutionLaunchSnapshot,
  bucket: LaunchEtaBucket,
): boolean => {
  if (bucketHasIncompletePhase(snapshot, bucket)) return false;
  if (etaBucketForPhase(snapshot.current_phase) === bucket && snapshot.state === "running") return false;
  if (bucketHasPhaseHistory(snapshot, bucket)) return true;
  return hasLaterBucketPhaseHistory(snapshot, bucket);
};

const currentEtaBucket = (
  snapshot: ExecutionLaunchSnapshot,
): LaunchEtaBucket | null => {
  const currentPhaseBucket = etaBucketForPhase(snapshot.current_phase);
  const hasMappedPhaseHistory = snapshot.phases.some((phase) => etaBucketForPhase(phase.phase) !== null);
  if (currentPhaseBucket === null && !hasMappedPhaseHistory) return null;
  for (const bucket of LAUNCH_ETA_BUCKET_ORDER) {
    if (!bucketIsComplete(snapshot, bucket)) return bucket;
  }
  return null;
};

const bucketStartedAtMs = (
  snapshot: ExecutionLaunchSnapshot,
  bucket: LaunchEtaBucket,
): number | null => {
  let earliestStartedAtMs: number | null = null;
  for (const phase of snapshot.phases) {
    if (etaBucketForPhase(phase.phase) !== bucket) continue;
    const startedAtMs = parseUtcMs(phase.started_at);
    if (startedAtMs === null) continue;
    if (earliestStartedAtMs === null || startedAtMs < earliestStartedAtMs) {
      earliestStartedAtMs = startedAtMs;
    }
  }
  return earliestStartedAtMs;
};

const liveDownloadRemainingMs = (
  snapshot: ExecutionLaunchSnapshot,
  nowMs: number,
): number | null => {
  const download = snapshot.active_download;
  if (!download) return null;
  const totalBytes = download.total_bytes;
  const bytesPerSec = download.bytes_per_sec;
  if (
    totalBytes === null
    || totalBytes === undefined
    || bytesPerSec === null
    || bytesPerSec === undefined
    || bytesPerSec <= 0
  ) {
    return null;
  }
  const baseRemainingMs = Math.max(
    0,
    Math.floor(
      (Math.max(0, totalBytes - download.downloaded_bytes) * 1000) / bytesPerSec,
    ),
  );
  const updatedAtMs = parseUtcMs(snapshot.updated_at);
  if (updatedAtMs === null) return baseRemainingMs;
  return Math.max(0, baseRemainingMs - Math.max(0, nowMs - updatedAtMs));
};

export const launchEtaRemainingMs = (
  snapshot: ExecutionLaunchSnapshot | null,
  nowMs: number,
): number | null => {
  if (!snapshot) return null;
  if (snapshot.state === "ready") return 0;
  if (snapshot.state === "error") return null;
  const currentBucket = currentEtaBucket(snapshot);
  if (!currentBucket) {
    const downloadRemainingMs = liveDownloadRemainingMs(snapshot, nowMs);
    if (downloadRemainingMs !== null) {
      return downloadRemainingMs + SHARED_VM_STARTUP_MS + SANDBOX_SETUP_MS;
    }

    const launchElapsed = launchElapsedMs(snapshot, nowMs);
    if (launchElapsed === null) return TOTAL_LAUNCH_BUDGET_MS;
    return Math.max(0, TOTAL_LAUNCH_BUDGET_MS - launchElapsed);
  }

  const downstreamRemainingMs = remainingDownstreamBucketBudgetMs(currentBucket);
  const startedAtMs = bucketStartedAtMs(snapshot, currentBucket);
  const elapsedMs = startedAtMs === null ? 0 : Math.max(0, nowMs - startedAtMs);
  const currentBucketRemainingMs = Math.max(
    0,
    etaBucketBudgetMs(currentBucket) - elapsedMs,
  );
  if (currentBucket === "artifact_acquisition_preparation") {
    const downloadRemainingMs = liveDownloadRemainingMs(snapshot, nowMs);
    if (downloadRemainingMs !== null) {
      return downloadRemainingMs + downstreamRemainingMs;
    }
  }
  return currentBucketRemainingMs + downstreamRemainingMs;
};

export const formatLaunchRemaining = (ms: number | null): string => {
  if (ms === null || !Number.isFinite(ms) || ms <= 0) return "Finishing up…";
  const totalSeconds = Math.max(1, Math.ceil(ms / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes > 0) {
    return `${minutes}m ${seconds}s est. remaining`;
  }
  return `${seconds}s est. remaining`;
};
