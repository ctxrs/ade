use std::path::Path as FsPath;

use ctx_harness_sources as harness_sources;
use ctx_observability::logs;

pub(in crate::api) async fn load_provider_source_config_with_error(
    data_root: &FsPath,
    provider_id: &str,
) -> (
    Option<harness_sources::HarnessProviderSourceConfig>,
    Option<String>,
) {
    if !harness_sources::supports_harness_endpoint(provider_id) {
        return (None, None);
    }

    match harness_sources::get_provider_source_config(data_root, provider_id).await {
        Ok(config) => (Some(config), None),
        Err(err) => (None, Some(logs::redact_sensitive(&err.to_string()))),
    }
}

pub(in crate::api) async fn load_managed_agent_server_config_with_error(
    data_root: &FsPath,
) -> (
    crate::daemon::installer::AgentServerConfigFile,
    Option<String>,
) {
    match crate::daemon::installer::load_agent_server_config(data_root).await {
        Ok(config) => (config, None),
        Err(err) => (
            crate::daemon::installer::AgentServerConfigFile::default(),
            Some(logs::redact_sensitive(&err.to_string())),
        ),
    }
}
