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
    ExecutionLaunchSnapshot, ExecutionLaunchState, ExecutionLaunchStreamEvent,
    ExecutionSetupCoordinator,
};
#[cfg(test)]
use sha2::{Digest, Sha256};
#[cfg(test)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(test)]
use tokio::net::TcpListener;
#[cfg(test)]
use tokio::task::JoinHandle;

/// Tests that mutate process-global environment or manifest override state
/// must hold this lock for the full lifetime of the test.
pub(crate) fn process_env_test_lock() -> &'static AsyncMutex<()> {
    static LOCK: OnceLock<AsyncMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| AsyncMutex::new(()))
}

/// Workspace-runtime tests historically used a sandbox-specific name for the
/// shared sandbox-runtime lock. Keep that lock separate from the broader
/// process-env lock so long-lived runtime jobs are not queued behind unrelated
/// bundle/env tests.
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
    match tokio::time::timeout(timeout, async {
        let (initial, mut rx) = coordinator
            .subscribe_launch(job_id)
            .await
            .expect("missing launch job");
        if matches!(
            initial.state,
            ExecutionLaunchState::Ready | ExecutionLaunchState::Error
        ) {
            return initial;
        }

        loop {
            match rx.recv().await {
                Ok(ExecutionLaunchStreamEvent::LaunchComplete { snapshot })
                | Ok(ExecutionLaunchStreamEvent::LaunchError { snapshot }) => break snapshot,
                Ok(ExecutionLaunchStreamEvent::LaunchSnapshot { snapshot })
                    if matches!(
                        snapshot.state,
                        ExecutionLaunchState::Ready | ExecutionLaunchState::Error
                    ) =>
                {
                    break snapshot;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
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
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
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
                    panic!("launch stream closed before terminal state");
                }
            }
        }
    })
    .await
    {
        Ok(snapshot) => snapshot,
        Err(_) => {
            let latest = coordinator
                .launch_status(job_id)
                .await
                .expect("missing launch job after timeout");
            panic!("timed out waiting for terminal launch state: {latest:?}");
        }
    }
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
            "expected launch to reach Ready, got {terminal:?}"
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
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ] && [ \"$2\" = \"{container}\" ]; then\n  suffix=${{2#ctx-harness-}}\n  printf '[{{\"Mounts\":[{{\"Type\":\"volume\",\"Name\":\"ctx-ws-%s\",\"Destination\":\"/ctx/ws\"}}]}}]\\n' \"$suffix\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  shift\n  while [ \"$#\" -gt 0 ]; do\n    case \"$1\" in\n      --interactive)\n        shift\n        ;;\n      --user|--workdir|--env)\n        shift 2\n        ;;\n      *)\n        break\n        ;;\n    esac\n  done\n  container_name=\"$1\"\n  shift\n  command=\"$1\"\n  shift\n  if [ \"$container_name\" != \"{container}\" ]; then\n    echo \"unexpected container: $container_name\" >&2\n    exit 1\n  fi\n  if [ \"$command\" = \"tar\" ] && [ \"$1\" = \"-xf\" ] && [ \"$2\" = \"-\" ]; then\n    cat >/dev/null\n    exit 0\n  fi\n  if [ \"$command\" = \"git\" ] && [ \"$1\" = \"checkout\" ]; then\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-u\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"id\" ] && [ \"$1\" = \"-g\" ]; then\n    printf '1000\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"df\" ] && [ \"$1\" = \"-Pk\" ]; then\n    printf 'Filesystem 1024-blocks Used Available Capacity Mounted on\\n'\n    printf 'overlay 10485760 1024 7340032 1%% /ctx/ws\\n'\n    exit 0\n  fi\n  if [ \"$command\" = \"sh\" ] && [ \"$1\" = \"-lc\" ]; then\n    case \"$2\" in\n      *\"git rev-parse --is-inside-work-tree\"*)\n        printf 'true\\n'\n        exit 0\n        ;;\n      *)\n        exit 0\n        ;;\n    esac\n  fi\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        ),
    )
    .expect("write running-container sandbox CLI shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod running-container sandbox CLI shim");
    path
}

#[cfg(test)]
async fn spawn_static_http_server_with_suffix(
    body: Vec<u8>,
    suffix: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local http listener");
    let addr = listener.local_addr().expect("listener local addr");
    let shared = Arc::new(body);
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let payload = Arc::clone(&shared);
            tokio::spawn(async move {
                let mut req_buf = [0u8; 1024];
                let _ = socket.read(&mut req_buf).await;
                let headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = socket.write_all(headers.as_bytes()).await;
                let _ = socket.write_all(payload.as_slice()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}/{suffix}"), task)
}

#[cfg(test)]
async fn install_test_managed_harness_image_source(
    body: Vec<u8>,
) -> (
    ctx_bundled_assets::test_support::TestManagedCtxHarnessImageSourceGuard,
    JoinHandle<()>,
) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server_with_suffix(body, "ctx-harness.tar").await;
    let guard =
        ctx_bundled_assets::test_support::override_managed_ctx_harness_image_source_for_test(
            ctx_bundled_assets::ManagedArtifactSource {
                uri: url,
                sha256: digest,
            },
        );
    (guard, server)
}

#[cfg(test)]
pub(crate) struct TestManagedAvfLinuxRuntimeFixtureGuard {
    _runtime: ctx_avf_linux_runtime::TestManagedAvfLinuxRuntimeSourceGuard,
    _image: ctx_bundled_assets::test_support::TestManagedCtxHarnessImageSourceGuard,
}

#[cfg(test)]
fn avf_runtime_archive_bytes() -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut encoder);
        let payload = b"rootfs";
        let mut header = tar::Header::new_gnu();
        header
            .set_path("runtime/rootfs.img")
            .expect("set AVF runtime tar path");
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, &payload[..])
            .expect("append AVF rootfs image");
        tar.finish().expect("finish AVF runtime tar");
    }
    encoder.finish().expect("finish AVF runtime gzip")
}

#[cfg(test)]
pub(crate) async fn install_test_managed_avf_linux_runtime_source(
) -> (TestManagedAvfLinuxRuntimeFixtureGuard, Vec<JoinHandle<()>>) {
    let archive_bytes = avf_runtime_archive_bytes();
    let kernel_bytes = b"kernel".to_vec();
    let initrd_bytes = b"initrd".to_vec();
    let guest_agent_bytes = b"guest-agent".to_vec();
    let egress_proxy_bytes = b"egress-proxy".to_vec();
    let container_stack_bytes = b"container-stack".to_vec();
    let (archive_url, archive_server) =
        spawn_static_http_server_with_suffix(archive_bytes.clone(), "guest-runtime.tar.gz").await;
    let (kernel_url, kernel_server) =
        spawn_static_http_server_with_suffix(kernel_bytes.clone(), "vmlinuz").await;
    let (initrd_url, initrd_server) =
        spawn_static_http_server_with_suffix(initrd_bytes.clone(), "initrd.img").await;
    let (guest_agent_url, guest_agent_server) = spawn_static_http_server_with_suffix(
        guest_agent_bytes.clone(),
        "ctx-avf-linux-guest-agent",
    )
    .await;
    let (egress_proxy_url, egress_proxy_server) =
        spawn_static_http_server_with_suffix(egress_proxy_bytes.clone(), "ctx-egress-proxy").await;
    let (container_stack_url, container_stack_server) = spawn_static_http_server_with_suffix(
        container_stack_bytes.clone(),
        "container-stack.tar.gz",
    )
    .await;
    let (image_guard, image_server) =
        install_test_managed_harness_image_source(b"ctx-harness-image".to_vec()).await;
    let source = ctx_bundled_assets::ManagedRuntimeSource {
        uri: archive_url,
        sha256: hex::encode(Sha256::digest(&archive_bytes)),
        version: "ubuntu-minimal-test".to_string(),
        bin: "rootfs.img".to_string(),
        helpers: [
            (
                "kernel".to_string(),
                ctx_bundled_assets::ManagedArtifactSource {
                    uri: kernel_url,
                    sha256: hex::encode(Sha256::digest(&kernel_bytes)),
                },
            ),
            (
                "initrd".to_string(),
                ctx_bundled_assets::ManagedArtifactSource {
                    uri: initrd_url,
                    sha256: hex::encode(Sha256::digest(&initrd_bytes)),
                },
            ),
            (
                "guest-agent".to_string(),
                ctx_bundled_assets::ManagedArtifactSource {
                    uri: guest_agent_url,
                    sha256: hex::encode(Sha256::digest(&guest_agent_bytes)),
                },
            ),
            (
                "egress-proxy".to_string(),
                ctx_bundled_assets::ManagedArtifactSource {
                    uri: egress_proxy_url,
                    sha256: hex::encode(Sha256::digest(&egress_proxy_bytes)),
                },
            ),
            (
                "container-stack".to_string(),
                ctx_bundled_assets::ManagedArtifactSource {
                    uri: container_stack_url,
                    sha256: hex::encode(Sha256::digest(&container_stack_bytes)),
                },
            ),
        ]
        .into_iter()
        .collect(),
    };
    let runtime_guard =
        ctx_avf_linux_runtime::override_managed_avf_linux_runtime_source_for_test(source);
    (
        TestManagedAvfLinuxRuntimeFixtureGuard {
            _runtime: runtime_guard,
            _image: image_guard,
        },
        vec![
            archive_server,
            kernel_server,
            initrd_server,
            guest_agent_server,
            egress_proxy_server,
            container_stack_server,
            image_server,
        ],
    )
}

mod avf_linux;
pub(crate) use avf_linux::*;
