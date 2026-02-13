import { useCallback } from "react";
import type { NavigateFunction } from "react-router-dom";

type UseSettingsActionsArgs = {
  pathname: string;
  search: string;
  hash: string;
  navigate: NavigateFunction;
};

export function useSettingsActions({ pathname, search, hash, navigate }: UseSettingsActionsArgs) {
  const clearCheckoutStatus = useCallback(() => {
    const params = new URLSearchParams(search);
    if (!params.has("checkout") && !params.has("session_id")) return;
    params.delete("checkout");
    params.delete("session_id");
    const nextSearch = params.toString();
    navigate(
      {
        pathname,
        search: nextSearch ? `?${nextSearch}` : "",
        hash,
      },
      { replace: true },
    );
  }, [hash, navigate, pathname, search]);

  return {
    clearCheckoutStatus,
  };
}
