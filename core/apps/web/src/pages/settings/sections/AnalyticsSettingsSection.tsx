import { Row, Toggle } from "../../SettingsPage.components";
import { GeneralSection } from "./GeneralSection";

type AnalyticsSettingsSectionProps = {
  telemetryEnabled: boolean;
  loaded: boolean;
  setTelemetryEnabled: (next: boolean) => void;
};

export function AnalyticsSettingsSection({
  telemetryEnabled,
  loaded,
  setTelemetryEnabled,
}: AnalyticsSettingsSectionProps) {
  return (
    <GeneralSection>
      <div className="settings-preferences-flat">
        <div className="settings-preferences-group">
          <Row
            title="Health and Usage Metrics"
            description="Share anonymous health and usage metrics with ctx. Does not include PII, code, or prompts."
            control={
              <Toggle
                checked={telemetryEnabled}
                disabled={!loaded}
                onChange={setTelemetryEnabled}
                ariaLabel="Health and Usage Metrics"
              />
            }
          />
        </div>
      </div>
    </GeneralSection>
  );
}
