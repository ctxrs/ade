type CapturePolicyInput = {
  settingsLoaded: boolean;
  telemetryEnabled: boolean;
  isDev: boolean;
  devCaptureFlag: string | undefined;
};

export const computeAnalyticsCaptureEnabled = (input: CapturePolicyInput): boolean => {
  if (!input.settingsLoaded) return false;
  if (!input.telemetryEnabled) return false;
  if (!input.isDev) return true;
  return String(input.devCaptureFlag ?? "") === "1";
};
