use std::collections::HashMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::process::Command;
use tokio::sync::RwLock;

use ctx_worker_protocol::{RepoSpec, StartWorkerRequest};

use super::WorkerDriver;

const DAEMON_AUTH_ENV_VARS: &[&str] = &[
    "CTX_AUTH_TOKEN",
    "CTX_MCP_TOKEN",
    "CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN",
];

fn scrub_daemon_auth_env(cmd: &mut Command) {
    for key in DAEMON_AUTH_ENV_VARS {
        cmd.env_remove(key);
    }
}

pub struct LocalDriver {
    pub shim_path: String,
    pub processes: RwLock<HashMap<String, tokio::process::Child>>,
}

#[async_trait]
impl WorkerDriver for LocalDriver {
    async fn start(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<ctx_worker_protocol::SshInfo>> {
        let workdir = match &spec.repo {
            RepoSpec::Local { path } => path.clone(),
            _ => anyhow::bail!("local driver requires RepoSpec::Local"),
        };

        let mut cmd = Command::new(&self.shim_path);
        cmd.env("CTX_WORKER_ID", worker_id)
            .env("CTX_GATEWAY_URL", gateway_url)
            .env("CTX_WORKDIR", &workdir)
            .env("CTX_BASE_COMMIT", base_commit_sha)
            .env(
                "CTX_DIFF_DEBOUNCE_MS",
                spec.diff_debounce_ms.unwrap_or(1500).to_string(),
            )
            .current_dir(&workdir);
        if let Some(provider_id) = spec.provider_id.as_ref() {
            cmd.env("CTX_PROVIDER_ID", provider_id);
        }
        if let Some(model_id) = spec.model_id.as_ref() {
            cmd.env("CTX_MODEL_ID", model_id);
        }
        for (key, value) in &spec.env {
            cmd.env(key, value);
        }
        scrub_daemon_auth_env(&mut cmd);

        let child = cmd.spawn().context("spawning worker shim")?;
        let mut processes = self.processes.write().await;
        processes.insert(worker_id.to_string(), child);
        Ok(None)
    }

    async fn stop(&self, worker_id: &str) -> Result<()> {
        let mut processes = self.processes.write().await;
        if let Some(mut child) = processes.remove(worker_id) {
            let _ = child.kill().await;
        }
        Ok(())
    }

    async fn pause(&self, worker_id: &str) -> Result<()> {
        self.stop(worker_id).await
    }

    async fn resume(
        &self,
        worker_id: &str,
        spec: &StartWorkerRequest,
        base_commit_sha: &str,
        gateway_url: &str,
    ) -> Result<Option<ctx_worker_protocol::SshInfo>> {
        self.start(worker_id, spec, base_commit_sha, gateway_url)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn scrub_daemon_auth_env_removes_sensitive_tokens_from_local_driver_spawn() {
        let mut cmd = Command::new("ctx-worker-shim");
        cmd.env("CTX_AUTH_TOKEN", "daemon-token");
        cmd.env("CTX_MCP_TOKEN", "mcp-token");
        cmd.env("CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN", "shutdown-token");
        cmd.env("CTX_WORKER_GATEWAY_TOKEN", "gateway-token");
        scrub_daemon_auth_env(&mut cmd);

        let envs: HashMap<_, _> = cmd.as_std().get_envs().collect();
        for key in DAEMON_AUTH_ENV_VARS {
            assert_eq!(envs.get(std::ffi::OsStr::new(key)), Some(&None));
        }
        assert_eq!(
            envs.get(std::ffi::OsStr::new("CTX_WORKER_GATEWAY_TOKEN")),
            Some(&Some(std::ffi::OsStr::new("gateway-token")))
        );
    }
}
