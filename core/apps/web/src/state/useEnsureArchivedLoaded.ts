import { useEffect } from "react";

type EnsureArchivedLoadedParams = {
  archivedCollapsed: boolean;
  archivedLoaded: boolean;
  fetchState: "idle" | "loading" | "error";
  ensureArchivedLoaded: () => void;
};

export function useEnsureArchivedLoaded({
  archivedCollapsed,
  archivedLoaded,
  fetchState,
  ensureArchivedLoaded,
}: EnsureArchivedLoadedParams) {
  useEffect(() => {
    if (archivedCollapsed) return;
    if (archivedLoaded) return;
    if (fetchState === "loading") return;
    ensureArchivedLoaded();
  }, [archivedCollapsed, archivedLoaded, fetchState, ensureArchivedLoaded]);
}
