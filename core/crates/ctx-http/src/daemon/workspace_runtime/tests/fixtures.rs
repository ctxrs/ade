use super::*;

pub(super) struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    pub(super) fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.prev.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

pub(super) fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::sandbox_cli_env_test_lock()
}

pub(super) fn sample_workspace(tmp: &TempDir) -> Workspace {
    Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: tmp.path().to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    }
}

pub(super) fn sample_worktree(tmp: &TempDir, workspace_id: WorkspaceId) -> Worktree {
    Worktree {
        id: WorktreeId::new(),
        workspace_id,
        root_path: tmp.path().to_string_lossy().to_string(),
        base_commit_sha: "deadbeef".to_string(),
        git_branch: Some("main".to_string()),
        vcs_kind: None,
        base_revision: None,
        vcs_ref: None,
        created_at: Utc::now(),
        bootstrap_status: None,
        bootstrap_started_at: None,
        bootstrap_finished_at: None,
        bootstrap_exit_code: None,
        bootstrap_timeout_sec: None,
        bootstrap_error: None,
        bootstrap_log_path: None,
        bootstrap_log_truncated: None,
        bootstrap_command: None,
        bootstrap_script_path: None,
    }
}

pub(super) async fn runtime_manager(tmp: &TempDir) -> HarnessRuntimeManager {
    HarnessRuntimeManager::new(tmp.path().to_path_buf())
}

#[cfg(target_os = "macos")]
pub(super) async fn create_session_with_environment(
    stores: &StoreManager,
    root: &std::path::Path,
    execution_environment: ExecutionEnvironment,
) -> SessionId {
    let workspace = stores
        .global()
        .create_workspace(
            "ws".to_string(),
            root.to_string_lossy().to_string(),
            ctx_core::models::VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let store = stores
        .workspace(workspace.id)
        .await
        .expect("workspace store");
    let worktree = store
        .create_worktree(
            workspace.id,
            root.to_string_lossy().to_string(),
            "base".to_string(),
            None,
        )
        .await
        .expect("create worktree");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            execution_environment,
            "fake".to_string(),
            "fake-model".to_string(),
            "assistant".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create session")
        .id
}

pub(super) async fn spawn_static_http_server(body: Vec<u8>) -> (String, JoinHandle<()>) {
    spawn_static_http_server_with_suffix(body, "image.tar").await
}

pub(super) async fn spawn_static_http_server_with_suffix(
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

pub(super) async fn install_test_managed_machine_cache_source(
    body: Vec<u8>,
) -> (TestManagedSandboxMachineCacheSourceGuard, JoinHandle<()>) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server(body).await;
    let guard = override_managed_sandbox_machine_cache_source_for_test(
        bundled_assets::ManagedArtifactSource {
            uri: url,
            sha256: digest,
        },
    );
    (guard, server)
}

pub(super) async fn install_test_managed_harness_image_source(
    body: Vec<u8>,
) -> (TestManagedCtxHarnessImageSourceGuard, JoinHandle<()>) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server_with_suffix(body, "ctx-harness.tar").await;
    let guard =
        override_managed_ctx_harness_image_source_for_test(bundled_assets::ManagedArtifactSource {
            uri: url,
            sha256: digest,
        });
    (guard, server)
}

pub(super) struct TestManagedAvfLinuxRuntimeFixtureGuard {
    _runtime: ctx_avf_linux_runtime::TestManagedAvfLinuxRuntimeSourceGuard,
    _image: TestManagedCtxHarnessImageSourceGuard,
}
pub(super) fn avf_runtime_archive_bytes() -> Vec<u8> {
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

pub(super) async fn install_test_managed_avf_linux_runtime_source(
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
    let source = bundled_assets::ManagedRuntimeSource {
        uri: archive_url,
        sha256: hex::encode(Sha256::digest(&archive_bytes)),
        version: "ubuntu-minimal-test".to_string(),
        bin: "rootfs.img".to_string(),
        helpers: [
            (
                "kernel".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: kernel_url,
                    sha256: hex::encode(Sha256::digest(&kernel_bytes)),
                },
            ),
            (
                "initrd".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: initrd_url,
                    sha256: hex::encode(Sha256::digest(&initrd_bytes)),
                },
            ),
            (
                "guest-agent".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: guest_agent_url,
                    sha256: hex::encode(Sha256::digest(&guest_agent_bytes)),
                },
            ),
            (
                "egress-proxy".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: egress_proxy_url,
                    sha256: hex::encode(Sha256::digest(&egress_proxy_bytes)),
                },
            ),
            (
                "container-stack".to_string(),
                bundled_assets::ManagedArtifactSource {
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
