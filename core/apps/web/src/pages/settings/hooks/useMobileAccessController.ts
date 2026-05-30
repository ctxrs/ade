import { useCallback, useState } from "react";
import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";
import { disableMobileAccess, enableMobileAccess, getMobileAccessStatus } from "../../../api/client";
import { errorMessage } from "../../../utils/errorMessage";

type Params = {
  getAuthToken: (() => Promise<string>) | null;
};

export function useMobileAccessController({ getAuthToken }: Params) {
  const [mobileStatus, setMobileStatus] = useState<MobileAccessStatus | null>(null);
  const [mobileStatusBusy, setMobileStatusBusy] = useState(false);
  const [mobileStatusError, setMobileStatusError] = useState<string | null>(null);
  const [mobileEnableBusy, setMobileEnableBusy] = useState(false);
  const [mobileEnableError, setMobileEnableError] = useState<string | null>(null);
  const [mobileQr, setMobileQr] = useState<EnableMobileAccessResponse | null>(null);

  const refreshMobileAccess = useCallback(async () => {
    setMobileStatusBusy(true);
    setMobileStatusError(null);
    try {
      const status = await getMobileAccessStatus();
      setMobileStatus(status);
    } catch (e: unknown) {
      setMobileStatusError(errorMessage(e));
    } finally {
      setMobileStatusBusy(false);
    }
  }, []);

  const resolveAuthToken = useCallback(async (): Promise<string> => {
    if (!getAuthToken) {
      throw new Error("Managed mobile tunnel grants are not configured.");
    }
    return getAuthToken();
  }, [getAuthToken]);

  const handleEnableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await resolveAuthToken();
      const resp = await enableMobileAccess(token);
      setMobileQr(resp);
      setMobileStatus(resp.status);
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [resolveAuthToken]);

  const handleDisableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await resolveAuthToken();
      await disableMobileAccess(token);
      setMobileQr(null);
      await refreshMobileAccess();
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [refreshMobileAccess, resolveAuthToken]);

  return {
    mobileStatus,
    mobileStatusBusy,
    mobileStatusError,
    mobileEnableBusy,
    mobileEnableError,
    mobileQr,
    refreshMobileAccess,
    handleEnableMobile,
    handleDisableMobile,
  };
}
