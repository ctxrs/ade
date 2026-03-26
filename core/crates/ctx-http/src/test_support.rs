use std::sync::OnceLock;
use tokio::sync::Mutex as AsyncMutex;

#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

#[cfg(test)]
use crate::execution_setup::{
    ExecutionLaunchSnapshot, ExecutionLaunchState, ExecutionSetupCoordinator,
};

/// Tests that mutate workspace-runtime-related process globals, or that execute
/// launch/prewarm/runtime flows which can observe those globals, must hold
/// this lock for the full lifetime of the test and drain any spawned
/// background work before returning.
pub(crate) fn sandbox_cli_env_test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

// `start_workspace_launch` and `start_runtime_prewarm` return snapshots while
// the real work continues in background tasks. Tests that start these jobs
// must wait for a terminal state before returning or they can leak work into
// later tests that rebind global sandbox CLI env vars.
#[cfg(test)]
pub(crate) async fn wait_for_execution_launch_terminal(
    coordinator: &Arc<ExecutionSetupCoordinator>,
    job_id: &str,
    timeout: Duration,
) -> ExecutionLaunchSnapshot {
    tokio::time::timeout(timeout, async {
        loop {
            let latest = coordinator
                .launch_status(job_id)
                .await
                .expect("missing launch job");
            if matches!(
                latest.state,
                ExecutionLaunchState::Ready | ExecutionLaunchState::Error
            ) {
                break latest;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for terminal launch state")
}

#[cfg(test)]
pub(crate) struct TrackedExecutionLaunch {
    coordinator: Arc<ExecutionSetupCoordinator>,
    snapshot: ExecutionLaunchSnapshot,
}

#[cfg(test)]
impl TrackedExecutionLaunch {
    pub(crate) fn new(
        coordinator: &Arc<ExecutionSetupCoordinator>,
        snapshot: ExecutionLaunchSnapshot,
    ) -> Self {
        Self {
            coordinator: Arc::clone(coordinator),
            snapshot,
        }
    }

    pub(crate) async fn wait_ready(&self, timeout: Duration) -> ExecutionLaunchSnapshot {
        let terminal =
            wait_for_execution_launch_terminal(&self.coordinator, &self.snapshot.job_id, timeout)
                .await;
        assert_eq!(
            terminal.state,
            ExecutionLaunchState::Ready,
            "expected launch to reach Ready, got {:?}",
            terminal
        );
        terminal
    }
}

#[cfg(unix)]
pub(crate) fn write_running_container_sandbox_cli_shim(
    dir: &Path,
    log_path: &Path,
    container_name: &str,
) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("sandbox-cli-running-container-test.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"exists\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  suffix=${{2#ctx-harness-}}\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"ctx-ws-%s\",\"Destination\":\"/ctx/ws\"}}]}}]\\n' \"$suffix\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write running-container sandbox CLI shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod running-container sandbox CLI shim");
    path
}
