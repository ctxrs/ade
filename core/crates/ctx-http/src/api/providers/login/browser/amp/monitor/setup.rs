use super::*;

pub(super) use crate::daemon::providers::PreparedAmpLoginPaths as AmpLoginPaths;

pub(super) async fn prepare_amp_login_paths(
    providers: &ProvidersHandle,
    login_id: &str,
) -> Result<AmpLoginPaths, String> {
    providers.prepare_amp_login_paths(login_id).await
}

pub(super) fn amp_provider_env(
    providers: &ProvidersHandle,
    amp_home: &StdPath,
) -> HashMap<String, String> {
    providers.amp_login_provider_env(amp_home)
}
