import type { ClientSettingsState } from "../../../state/clientSettings";
import { Row, Toggle } from "../../SettingsPage.components";
import { GeneralSection } from "./GeneralSection";

type NotificationsSettingsSectionProps = {
  isDesktopApp: () => boolean;
  desktopTurnNotifications: boolean;
  clientSettingsState: ClientSettingsState;
  clientSettingsSaving: boolean;
  clientSettingsError: string | null;
  onToggleTurnNotifications: (next: boolean) => Promise<void>;
};

export function NotificationsSettingsSection({
  isDesktopApp,
  desktopTurnNotifications,
  clientSettingsState,
  clientSettingsSaving,
  clientSettingsError,
  onToggleTurnNotifications,
}: NotificationsSettingsSectionProps) {
  return (
    <GeneralSection>
      <div className="settings-preferences-flat">
        <div className="settings-preferences-group">
          <Row
            title="Turn completed"
            description={
              isDesktopApp()
                ? "Send a system notification when a turn completes and the app is not focused."
                : "Available in the desktop app."
            }
            control={
              <Toggle
                checked={desktopTurnNotifications}
                disabled={!isDesktopApp() || !clientSettingsState.loaded || clientSettingsSaving}
                onChange={(next) => {
                  void onToggleTurnNotifications(next);
                }}
                ariaLabel="Turn completed notifications"
              />
            }
          />
        </div>
      </div>
      {clientSettingsError ? <div className="settings-banner settings-banner-error">{clientSettingsError}</div> : null}
    </GeneralSection>
  );
}
