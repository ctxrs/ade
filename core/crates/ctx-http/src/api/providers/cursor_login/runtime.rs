use super::super::login::{
    resolve_provider_login_command_from_config, resolve_runtime_provider_command_from_config,
};
use super::*;

fn is_cursor_login_command(path: &StdPath) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "cursor-agent" || name == "cursor-agent.exe")
}

pub(super) async fn resolve_cursor_login_runtime_from_config(
    data_root: &StdPath,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    if let Some(runtime) = resolve_provider_login_command_from_config(data_root, "cursor").await? {
        if is_cursor_login_command(StdPath::new(&runtime.command_abs_path)) {
            return Ok(runtime);
        }
        anyhow::bail!(
            "runtime_command_invalid: provider=cursor-login (configured login executable must point to `cursor-agent`)"
        );
    }

    if let Some(runtime) = resolve_runtime_provider_command_from_config(data_root, "cursor").await?
    {
        if matches!(
            runtime.source,
            installer::ProviderRuntimeCommandSource::BundledSeed
        ) {
            anyhow::bail!(
                "runtime_command_missing: provider=cursor-login (ctx requires a managed or explicitly configured `cursor-agent` login executable; bundled runtime discovery is not supported)"
            );
        }
        if is_cursor_login_command(StdPath::new(&runtime.command_abs_path)) {
            return Ok(runtime);
        }
        anyhow::bail!(
            "runtime_command_invalid: provider=cursor-login (configured runtime command must point to `cursor-agent`)"
        );
    }

    anyhow::bail!(
        "runtime_command_missing: provider=cursor-login (ctx requires a managed or explicitly configured `cursor-agent` login executable; host PATH lookup is not supported)"
    );
}

pub(super) async fn resolve_cursor_login_runtime(
    state: &Arc<AppState>,
) -> anyhow::Result<installer::ProviderRuntimeCommand> {
    resolve_cursor_login_runtime_from_config(&state.core.data_root).await
}
