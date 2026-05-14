mod process;
mod shim;

#[cfg(test)]
pub(super) use ctx_daemon::daemon::providers::resolve_claude_login_runtime_from_config;
pub(super) use process::{spawn_claude_setup_token_command, ClaudeLoginSpawn};
#[cfg(test)]
pub(super) use shim::{
    claude_browser_open_shim_script, claude_login_should_skip_browser_open,
    CLAUDE_BROWSER_AUTH_TIER,
};
