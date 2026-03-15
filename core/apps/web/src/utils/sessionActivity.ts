import type { SessionActivityState } from "@ctx/types";

export function isSessionWorkingActivity(
  activity: SessionActivityState | null | undefined,
): boolean {
  return activity?.is_working === true;
}

export function hasSessionActiveTurn(
  activity: SessionActivityState | null | undefined,
): boolean {
  const status = activity?.last_turn_status ?? null;
  return status === "running" || status === "queued";
}
