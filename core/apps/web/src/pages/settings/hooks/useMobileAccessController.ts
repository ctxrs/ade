import { useCallback, useState } from "react";
import type { SupabaseClient } from "@supabase/supabase-js";
import type { EnableMobileAccessResponse, MobileAccessStatus } from "../../../api/client";
import { disableMobileAccess, enableMobileAccess, getMobileAccessStatus } from "../../../api/client";
import { errorMessage } from "../../../utils/errorMessage";

type Params = {
  supabase: SupabaseClient | null;
};

export function useMobileAccessController({ supabase }: Params) {
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

  const getSupabaseToken = useCallback(async (): Promise<string> => {
    if (!supabase) {
      throw new Error("Supabase is not configured.");
    }
    const { data, error } = await supabase.auth.getSession();
    if (error) throw error;
    const token = data.session?.access_token;
    if (!token) {
      throw new Error("Sign in required to manage mobile access.");
    }
    return token;
  }, [supabase]);

  const handleEnableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await getSupabaseToken();
      const resp = await enableMobileAccess(token);
      setMobileQr(resp);
      setMobileStatus(resp.status);
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [getSupabaseToken]);

  const handleDisableMobile = useCallback(async () => {
    setMobileEnableBusy(true);
    setMobileEnableError(null);
    try {
      const token = await getSupabaseToken();
      await disableMobileAccess(token);
      setMobileQr(null);
      await refreshMobileAccess();
    } catch (e: unknown) {
      setMobileEnableError(errorMessage(e));
    } finally {
      setMobileEnableBusy(false);
    }
  }, [getSupabaseToken, refreshMobileAccess]);

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
