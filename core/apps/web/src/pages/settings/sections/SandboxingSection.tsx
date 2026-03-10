import type { SandboxingSettings } from "../../../api/client";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../components/ui/select";
import { Card, Row } from "../../SettingsPage.components";

export function SandboxingSection({
  loaded,
  providerControlMode,
  onProviderControlModeChange,
}: {
  loaded: boolean;
  providerControlMode: SandboxingSettings["provider_control_mode"];
  onProviderControlModeChange: (value: SandboxingSettings["provider_control_mode"]) => void;
}) {
  return (
    <Card title="Sandboxing">
      <Row
        title="Provider control"
        description="Default is full capability. Switch to honor the harness's native permission settings."
        control={
          <Select value={providerControlMode} onValueChange={(value) => onProviderControlModeChange(value as SandboxingSettings["provider_control_mode"])} disabled={!loaded}>
            <SelectTrigger className="tw-min-w-[10rem]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="full">Full capability</SelectItem>
              <SelectItem value="harness_native">Harness-native permissions</SelectItem>
              <SelectItem value="ctx_enforced">ctx-enforced (coming soon)</SelectItem>
            </SelectContent>
          </Select>
        }
      />
    </Card>
  );
}
