use ctx_provider_install::install_state::InstallTarget;
use ctx_settings_model::{ExecutionMode, ExecutionSettings};

#[test]
fn install_target_for_settings_matches_execution_mode() {
    let host = ExecutionSettings {
        mode: ExecutionMode::Host,
        ..ExecutionSettings::default()
    };
    let container = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        ..ExecutionSettings::default()
    };

    assert_eq!(
        ctx_settings_service::install_target_for_settings(&host),
        InstallTarget::Host
    );
    assert_eq!(
        ctx_settings_service::install_target_for_settings(&container),
        InstallTarget::Container
    );
}
