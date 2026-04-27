use ctx_core::env::DAEMON_AUTH_ENV_VARS;
use tokio::process::Command;

pub(crate) fn scrub_daemon_auth_env(cmd: &mut Command) {
    for key in DAEMON_AUTH_ENV_VARS {
        cmd.env_remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.previous {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[test]
    fn scrub_daemon_auth_env_removes_sensitive_tokens() {
        let _auth = ScopedEnvVar::set("CTX_AUTH_TOKEN", "daemon-token");
        let _mcp = ScopedEnvVar::set("CTX_MCP_TOKEN", "mcp-token");
        let mut cmd = Command::new("/usr/bin/env");

        scrub_daemon_auth_env(&mut cmd);

        let envs: HashMap<_, _> = cmd.as_std().get_envs().collect();
        for key in DAEMON_AUTH_ENV_VARS {
            assert_eq!(
                envs.get(std::ffi::OsStr::new(key))
                    .and_then(|value| value.as_deref()),
                None,
                "expected {key} to be removed"
            );
        }
    }
}
