import { useCallback, useState } from "react";
import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";

type Params = {
  getAuthToken: (() => Promise<string>) | null;
};

export function useMobileAccessController(_params: Params) {
  const [mobileEnableError, setMobileEnableError] = useState<string | null>(null);

  const refreshMobileAccess = useCallback(async () => {}, []);

  const handleEnableMobile = useCallback(async () => {
    setMobileEnableError("This settings action is unavailable in the source build/local ADE.");
  }, []);

  const handleDisableMobile = useCallback(async () => {
    setMobileEnableError(null);
  }, []);

  return {
    mobileStatus: null as MobileAccessStatus | null,
    mobileStatusBusy: false,
    mobileStatusError: null as string | null,
    mobileEnableBusy: false,
    mobileEnableError,
    mobileQr: null as EnableMobileAccessResponse | null,
    refreshMobileAccess,
    handleEnableMobile,
    handleDisableMobile,
  };
}
