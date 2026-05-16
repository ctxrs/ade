use super::*;

#[cfg(test)]
mod runtime;
mod session;

pub(crate) use session::{get_claude_login, start_claude_login};

#[cfg(test)]
pub(crate) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<ctx_daemon::daemon::providers::ProviderLoginRuntimeCommand> {
    runtime::resolve_claude_login_runtime_from_config(data_root).await
}
