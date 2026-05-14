use super::*;

mod auth_url;
mod runtime;
mod session;

#[cfg(test)]
mod tests;

#[cfg(test)]
use auth_url::{
    extract_preferred_claude_auth_url, read_claude_browser_open_capture_url,
    should_replace_observed_claude_auth_url, ClaudeAuthUrlSource, CLAUDE_BROWSER_OPEN_MARKER,
};
#[cfg(test)]
use runtime::{
    claude_browser_open_shim_script, claude_login_should_skip_browser_open,
    CLAUDE_BROWSER_AUTH_TIER,
};
pub(crate) use session::{get_claude_login, start_claude_login};

#[cfg(test)]
pub(crate) async fn resolve_claude_login_runtime_from_config(
    data_root: &std::path::Path,
) -> anyhow::Result<ctx_daemon::daemon::providers::ProviderLoginRuntimeCommand> {
    runtime::resolve_claude_login_runtime_from_config(data_root).await
}
