import type {
  ContainerMachineMemoryProfile,
  SandboxingSettings,
} from "../../../api/client";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../../components/ui/select";
import { Card, Row } from "../../SettingsPage.components";

export function SandboxingSection({
  loaded,
  providerControlMode,
  onProviderControlModeChange,
  machineMemoryProfile,
  onMachineMemoryProfileChange,
  customMemoryMb,
  onCustomMemoryMbChange,
  idleShutdownSeconds,
  onIdleShutdownSecondsChange,
  hostPressureSwapThresholdMb,
  onHostPressureSwapThresholdMbChange,
  canSaveMachineSettings,
}: {
  loaded: boolean;
  providerControlMode: SandboxingSettings["provider_control_mode"];
  onProviderControlModeChange: (value: SandboxingSettings["provider_control_mode"]) => void;
  machineMemoryProfile: ContainerMachineMemoryProfile;
  onMachineMemoryProfileChange: (value: ContainerMachineMemoryProfile) => void;
  customMemoryMb: string;
  onCustomMemoryMbChange: (value: string) => void;
  idleShutdownSeconds: string;
  onIdleShutdownSecondsChange: (value: string) => void;
  hostPressureSwapThresholdMb: string;
  onHostPressureSwapThresholdMbChange: (value: string) => void;
  canSaveMachineSettings: boolean;
}) {
  return (
    <>
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

      <Card title="Local Sandbox Runtime">
        <Row
          title="Machine memory profile"
          description="Podman on macOS uses fixed VM memory. Changing the profile recreates the local sandbox runtime on next launch."
          control={
            <Select value={machineMemoryProfile} onValueChange={(value) => onMachineMemoryProfileChange(value as ContainerMachineMemoryProfile)} disabled={!loaded}>
              <SelectTrigger className="tw-min-w-[12rem]">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="economy">Economy (2 GiB)</SelectItem>
                <SelectItem value="balanced">Balanced (4 GiB)</SelectItem>
                <SelectItem value="performance">Performance (8 GiB)</SelectItem>
                <SelectItem value="custom">Custom</SelectItem>
              </SelectContent>
            </Select>
          }
        />
        {machineMemoryProfile === "custom" ? (
          <Row
            title="Custom memory (MiB)"
            description="Minimum 1024 MiB. The runtime will be recreated from the cached machine image on next use."
            control={
              <input
                className="settings-control"
                type="number"
                min={1024}
                step={256}
                value={customMemoryMb}
                onChange={(event) => onCustomMemoryMbChange(event.target.value)}
                disabled={!loaded}
                placeholder="4096"
              />
            }
          />
        ) : null}
        <Row
          title="Idle shutdown (seconds)"
          description="Stop the local sandbox VM after this much inactivity to reclaim RAM and swap."
          control={
            <input
              className="settings-control"
              type="number"
              min={60}
              step={60}
              value={idleShutdownSeconds}
              onChange={(event) => onIdleShutdownSecondsChange(event.target.value)}
              disabled={!loaded}
              placeholder="900"
            />
          }
        />
        <Row
          title="Host pressure swap threshold (MiB)"
          description="If host swap use exceeds this threshold, ctx may stop an idle sandbox VM sooner."
          control={
            <input
              className="settings-control"
              type="number"
              min={0}
              step={128}
              value={hostPressureSwapThresholdMb}
              onChange={(event) => onHostPressureSwapThresholdMbChange(event.target.value)}
              disabled={!loaded}
              placeholder="1024"
            />
          }
        />
      </Card>

      {!canSaveMachineSettings ? (
        <div className="settings-banner settings-banner-error">
          Enter a valid idle timeout and swap threshold. Custom memory profiles require at least 1024 MiB.
        </div>
      ) : null}
    </>
  );
}
