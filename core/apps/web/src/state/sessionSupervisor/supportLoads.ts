import { errorMessage } from "../../utils/errorMessage";
import type { SessionSupportLoadErrorKey } from "../sessionSupervisorCore";

const SUPPORT_LOAD_ERROR_LABELS: Record<SessionSupportLoadErrorKey, string> = {
  state: "session state",
  artifacts: "artifacts",
  subagentInvocations: "subagent invocations",
};

export function formatSupportLoadError(key: SessionSupportLoadErrorKey, value: unknown): string {
  const detail = String(errorMessage(value) ?? "").trim();
  if (!detail || detail === "undefined" || detail === "null" || detail === "[object Object]") {
    return `Failed to load ${SUPPORT_LOAD_ERROR_LABELS[key]}.`;
  }
  if (detail.startsWith("Failed to load ")) {
    return detail;
  }
  return `Failed to load ${SUPPORT_LOAD_ERROR_LABELS[key]}: ${detail}`;
}

type SessionStateLoadStatus = {
  stateLoaded: boolean;
  stateLoading: boolean;
  stateRev?: number;
  stateAppliedRev?: number;
};

export function shouldFetchSessionState(
  entry: SessionStateLoadStatus,
  opts?: { force?: boolean },
): boolean {
  if (entry.stateLoading) return false;
  if (opts?.force) return true;
  if (!entry.stateLoaded) return true;
  if (
    typeof entry.stateRev === "number" &&
    typeof entry.stateAppliedRev === "number" &&
    entry.stateAppliedRev < entry.stateRev
  ) {
    return true;
  }
  if (typeof entry.stateRev === "number" && typeof entry.stateAppliedRev !== "number") {
    return true;
  }
  return false;
}

export function adoptLoadedStateRevision(
  stateLoaded: boolean,
  currentAppliedRev: number | undefined,
  nextKnownRev: number | undefined,
): number | undefined {
  if (!stateLoaded || typeof nextKnownRev !== "number") return currentAppliedRev;
  if (typeof currentAppliedRev === "number") return currentAppliedRev;
  return nextKnownRev;
}
