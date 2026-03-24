use super::network_policy_transition::transparent_proxy_policy;
use super::podman_recovery::{
    collect_ctx_managed_podman_helper_pids, collect_ctx_managed_podman_helper_pids_from_ps_output,
    initialize_podman_machine, is_ctx_managed_podman_helper_process_command,
    kill_ctx_managed_podman_helper_processes, literal_pkill_pattern,
    looks_like_missing_machine_error, looks_like_recoverable_machine_start_error,
    looks_like_running_but_unreachable_machine_start_error, podman_machine_temp_state_paths,
};
use super::*;
use chrono::Utc;
use ctx_core::ids::{SessionId, TaskId, WorkspaceId, WorktreeId};
use ctx_core::models::ExecutionEnvironment;
use ctx_store::StoreManager;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
#[path = "tests/recovery_tests.rs"]
mod recovery_tests;

struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
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

fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::podman_env_test_lock()
}

#[test]
fn container_machine_memory_profiles_scale_with_host_ram() {
    let mut settings = ContainerExecutionSettings::default();

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Economy;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 48 * 1024),
        6144
    );

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Balanced;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 48 * 1024),
        12 * 1024
    );

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Performance;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 48 * 1024),
        24 * 1024
    );
}

#[test]
fn container_machine_memory_profiles_apply_expected_floors_and_caps() {
    let mut settings = ContainerExecutionSettings::default();

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Economy;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 16 * 1024),
        4096
    );
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 128 * 1024),
        8192
    );

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Balanced;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 16 * 1024),
        4096
    );
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 128 * 1024),
        16 * 1024
    );

    settings.machine.memory_profile = crate::settings::ContainerMachineMemoryProfile::Performance;
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 16 * 1024),
        8192
    );
    assert_eq!(
        container_machine_memory_mb_for_host_memory(&settings, 128 * 1024),
        32 * 1024
    );
}

fn sample_workspace(tmp: &TempDir) -> Workspace {
    Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: tmp.path().to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    }
}

fn sample_worktree(tmp: &TempDir, workspace_id: WorkspaceId) -> Worktree {
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

async fn runtime_manager(tmp: &TempDir) -> HarnessRuntimeManager {
    HarnessRuntimeManager::new(tmp.path().to_path_buf())
}

async fn create_session_with_environment(
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

fn sample_cached_container() -> HarnessContainer {
    let mut external_mounts = HashSet::new();
    external_mounts.insert("/tmp/external".to_string());
    HarnessContainer {
        name: "ctx-harness-sample".to_string(),
        mount_mode: ContainerMountMode::HostMounted,
        network_mode: ContainerNetworkMode::Allowlist,
        allowlist: vec!["github.com".to_string()],
        external_mounts,
        egress_guard: true,
    }
}

fn sample_container_settings() -> ContainerExecutionSettings {
    ContainerExecutionSettings {
        runtime: crate::settings::ContainerRuntimeKind::Podman,
        mount_mode: ContainerMountMode::HostMounted,
        network_mode: ContainerNetworkMode::Allowlist,
        allowlist: vec!["github.com".to_string()],
        image: None,
        machine: crate::settings::ContainerMachineSettings::default(),
    }
}

async fn spawn_static_http_server(body: Vec<u8>) -> (String, JoinHandle<()>) {
    spawn_static_http_server_with_suffix(body, "image.tar").await
}

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

async fn install_test_managed_machine_cache_source(
    body: Vec<u8>,
) -> (
    crate::bundled_assets::TestManagedPodmanMachineCacheSourceGuard,
    JoinHandle<()>,
) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server(body).await;
    let guard = crate::bundled_assets::override_managed_podman_machine_cache_source_for_test(
        bundled_assets::ManagedArtifactSource {
            uri: url,
            sha256: digest,
        },
    );
    (guard, server)
}

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

async fn install_test_managed_avf_linux_runtime_source() -> (
    super::avf_linux_vm::TestManagedAvfLinuxRuntimeSourceGuard,
    Vec<JoinHandle<()>>,
) {
    let archive_bytes = avf_runtime_archive_bytes();
    let kernel_bytes = b"kernel".to_vec();
    let initrd_bytes = b"initrd".to_vec();
    let guest_agent_bytes = b"guest-agent".to_vec();
    let egress_proxy_bytes = b"egress-proxy".to_vec();
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
        ]
        .into_iter()
        .collect(),
    };
    let guard =
        crate::workspace_runtime::override_managed_avf_linux_runtime_source_for_test(source);
    (
        guard,
        vec![
            archive_server,
            kernel_server,
            initrd_server,
            guest_agent_server,
            egress_proxy_server,
        ],
    )
}

#[cfg(unix)]
fn write_avf_linux_lifecycle_helper(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("ctx-avf-linux-helper-runtime-manager-test.sh");
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let script = format!(
        r#"#!/bin/sh
cmd="$1"
shift
case "$cmd" in
  probe)
    printf '%s\n' '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","helper_version":"0.0.0-test","host_os":"{host_os}","host_arch":"{host_arch}","supported":true,"save_restore_supported":true,"rosetta_supported":true,"notes":["test helper"]}}'
    ;;
  prepare-runtime-layout)
    data_root="$1"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    logs_root="$vm_root/logs"
    state_path="$vm_root/shared-vm-state.json"
    mkdir -p "$logs_root"
    status_file="$vm_root/helper-status.txt"
    if [ ! -f "$status_file" ]; then
      printf 'stopped' > "$status_file"
    fi
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","vm_root":"%s","logs_root":"%s","state_path":"%s","layout_status":"prepared","notes":["layout ready"]}}\n' "$vm_root" "$logs_root" "$state_path"
    ;;
  workspace-vm-state|shared-vm-state)
    data_root="$1"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    logs_root="$vm_root/logs"
    state_path="$vm_root/shared-vm-state.json"
    log_path="$logs_root/shared-vm.log"
    status_file="$vm_root/helper-status.txt"
    version_file="$vm_root/runtime-version.txt"
    state=$(cat "$status_file" 2>/dev/null || printf 'missing')
    runtime_version=$(cat "$version_file" 2>/dev/null || true)
    if [ "$state" = "running" ]; then
      printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"running","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","runtime_version":"%s","transition_status":"scaffolded","simulated":true,"notes":["state ready"]}}\n' "$vm_root" "$logs_root" "$state_path" "$log_path" "$runtime_version"
    else
      printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"%s","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","simulated":true,"notes":["state ready"]}}\n' "$state" "$vm_root" "$logs_root" "$state_path" "$log_path"
    fi
    ;;
  start-workspace-vm|start-shared-vm)
    data_root="$1"
    runtime_root="$2"
    rootfs_image="$3"
    kernel_path="$4"
    initrd_path="$5"
    runtime_version="$6"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    logs_root="$vm_root/logs"
    state_path="$vm_root/shared-vm-state.json"
    log_path="$logs_root/shared-vm.log"
    mkdir -p "$logs_root"
    printf 'running' > "$vm_root/helper-status.txt"
    printf '%s' "$runtime_version" > "$vm_root/runtime-version.txt"
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"running","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","runtime_root":"%s","rootfs_image":"%s","kernel_path":"%s","initrd_path":"%s","runtime_version":"%s","transition_status":"scaffolded","simulated":true,"notes":["scaffolded"]}}\n' "$vm_root" "$logs_root" "$state_path" "$log_path" "$runtime_root" "$rootfs_image" "$kernel_path" "$initrd_path" "$runtime_version"
    ;;
  stop-workspace-vm|stop-shared-vm)
    data_root="$1"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    logs_root="$vm_root/logs"
    state_path="$vm_root/shared-vm-state.json"
    log_path="$logs_root/shared-vm.log"
    mkdir -p "$logs_root"
    printf 'stopped' > "$vm_root/helper-status.txt"
    rm -f "$vm_root/runtime-version.txt"
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"stopped","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","transition_status":"stopped","simulated":true,"notes":["stopped"]}}\n' "$vm_root" "$logs_root" "$state_path" "$log_path"
    ;;
  prepare-guest-worktree)
    data_root="$1"
    workspace_id="$2"
    worktree_id="$3"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    host_shadow_root="$vm_root/worktrees/$workspace_id/$worktree_id/shadow-root"
    metadata_path="$vm_root/worktrees/$workspace_id/$worktree_id/worktree.json"
    guest_root="/ctx/ws/worktrees/$worktree_id"
    mkdir -p "$host_shadow_root"
    if [ ! -f "$metadata_path" ]; then
      mkdir -p "$(dirname "$metadata_path")"
      printf '{{"workspace_id":"%s","worktree_id":"%s"}}\n' "$workspace_id" "$worktree_id" > "$metadata_path"
      status="prepared"
      note="prepared guest worktree"
    else
      status="already_present"
      note="existing guest worktree"
    fi
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","workspace_id":"%s","worktree_id":"%s","guest_root":"%s","host_shadow_root":"%s","metadata_path":"%s","status":"%s","simulated":true,"notes":["%s"]}}\n' "$workspace_id" "$worktree_id" "$guest_root" "$host_shadow_root" "$metadata_path" "$status" "$note"
    ;;
  guest-exec)
    data_root=""
    workspace_id=""
    worktree_id=""
    cwd=""
    guest_command=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --data-root) data_root="$2"; shift 2 ;;
        --workspace-id) workspace_id="$2"; shift 2 ;;
        --worktree-id) worktree_id="$2"; shift 2 ;;
        --cwd) cwd="$2"; shift 2 ;;
        --command) guest_command="$2"; shift 2 ;;
        --user) shift 2 ;;
        --pty) shift ;;
        --env)
          kv="$2"
          key=${{kv%%=*}}
          value=${{kv#*=}}
          export "$key=$value"
          shift 2
          ;;
        --) shift; break ;;
        *) echo "unexpected guest-exec arg: $1" >&2; exit 1 ;;
      esac
    done
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    host_shadow_root="$vm_root/worktrees/$workspace_id/$worktree_id/shadow-root"
    guest_root="/ctx/ws/worktrees/$worktree_id"
    case "$cwd" in
      "$guest_root") host_cwd="$host_shadow_root" ;;
      "$guest_root"/*) host_cwd="$host_shadow_root/${{cwd#"$guest_root"/}}" ;;
      *) echo "invalid guest cwd: $cwd" >&2; exit 1 ;;
    esac
    mkdir -p "$host_cwd"
    (cd "$host_cwd" && exec "$guest_command" "$@")
    ;;
  *)
    echo "unexpected helper invocation: $cmd $*" >&2
    exit 1
    ;;
esac
"#
    );
    std::fs::write(&path, script).expect("write AVF lifecycle helper shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF lifecycle helper shim");
    path
}

#[tokio::test]
async fn container_mode_errors_when_podman_unavailable() {
    let _serial = env_var_test_lock().lock().await;
    let _guard = EnvGuard::set("CTX_TEST_PODMAN_AVAILABLE", "0");
    let tmp = tempfile::tempdir().unwrap();
    let manager = runtime_manager(&tmp).await;
    let workspace = sample_workspace(&tmp);
    let worktree = sample_worktree(&tmp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::Podman,
            ..ContainerExecutionSettings::default()
        },
    };

    let err = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:9999")
        .await
        .unwrap_err();
    let message = err.to_string();
    assert!(message.contains("podman unavailable"));
    assert!(message.contains("execution mode is container"));
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_in_host_mode_does_not_refresh_local_sandbox_activity() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let before_prepare = Instant::now() - Duration::from_secs(600);
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = before_prepare;
    }
    let settings = ExecutionSettings {
        mode: ExecutionMode::Host,
        container: ContainerExecutionSettings::default(),
    };

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("host prepare should succeed without touching the local sandbox");
    match plan.runtime {
        HarnessRuntimeKind::Host => {}
        HarnessRuntimeKind::Container { .. } => panic!("expected host runtime"),
        HarnessRuntimeKind::AvfLinuxVm => panic!("expected host runtime"),
    }
    assert!(
        std::fs::read_to_string(&log_path)
            .unwrap_or_default()
            .is_empty(),
        "host prepare should not invoke podman"
    );
    let idle_for = manager.runtime_idle_for();
    assert!(
        idle_for >= Duration::from_secs(540),
        "host prepare should not refresh local sandbox activity; idle_for={idle_for:?}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_reuses_running_workspace_container_without_front_loading_image_readiness() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let container_name = workspace_container_name(workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            ..Default::default()
        },
    };

    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"exists\" ]; then\n  echo 'transient image store failure' >&2\n  exit 125\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"rm\" ] && [ \"$2\" = \"-f\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("running workspace container should be reused without image checks");

    match plan.runtime {
        HarnessRuntimeKind::Container { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
        HarnessRuntimeKind::AvfLinuxVm => panic!("expected podman container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
    assert!(
        log.contains(&format!("container exists {container_name}")),
        "expected running container existence check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected running-container inspect in log:\n{log}"
    );
    assert!(
        !log.contains("image exists"),
        "running-container reuse should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "running-container reuse should not recreate the container:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_starts_cached_workspace_container_when_podman_reports_it_stopped() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    let manager = runtime_manager(&temp).await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let container_name = workspace_container_name(workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            network_mode: ContainerNetworkMode::All,
            allowlist: Vec::new(),
            ..Default::default()
        },
    };
    let mount_plan = build_mounts(
        temp.path(),
        &workspace,
        Some(&worktree),
        &settings.container,
    );

    manager.containers.lock().await.insert(
        workspace.id,
        HarnessContainer {
            name: container_name.clone(),
            mount_mode: settings.container.mount_mode.clone(),
            network_mode: settings.container.network_mode.clone(),
            allowlist: settings.container.allowlist.clone(),
            external_mounts: mount_plan.external_mounts,
            egress_guard: false,
        },
    );

    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"exists\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'false\\n'\n  exit 0\nfi\nif [ \"$1\" = \"start\" ] && [ \"$2\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                container = container_name,
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://127.0.0.1:4399")
        .await
        .expect("stopped cached workspace container should be restarted");

    match plan.runtime {
        HarnessRuntimeKind::Container { name } => assert_eq!(name, container_name),
        HarnessRuntimeKind::Host => panic!("expected container runtime"),
        HarnessRuntimeKind::AvfLinuxVm => panic!("expected podman container runtime"),
    }

    let log = std::fs::read_to_string(&log_path).expect("read podman invocation log");
    assert!(
        log.contains(&format!("container exists {container_name}")),
        "expected container existence check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected stopped-container inspect in log:\n{log}"
    );
    assert!(
        log.contains(&format!("start {container_name}")),
        "expected stopped cached container to be started:\n{log}"
    );
    assert!(
        !log.contains("image exists"),
        "starting a stopped cached container should not front-load image checks:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "starting a stopped cached container should not recreate the container:\n{log}"
    );
}

#[test]
fn podman_binary_path_uses_env_override() {
    let _serial = env_var_test_lock().blocking_lock();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_string_lossy().to_string();
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &path);
    let resolved = podman_binary_path(Path::new("/tmp")).expect("env override should resolve");
    assert_eq!(resolved, tmp.path());
}

#[test]
fn podman_invocation_sets_paths_under_short_runtime_root() {
    let _serial = env_var_test_lock().blocking_lock();
    let tmp = tempfile::tempdir().expect("tempdir");
    let podman_bin = tempfile::NamedTempFile::new().expect("podman bin");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_bin.path().to_string_lossy());
    let inv = podman_invocation(tmp.path()).expect("podman invocation");
    let runtime_dir = inv
        .env
        .get("XDG_RUNTIME_DIR")
        .cloned()
        .expect("runtime dir env");
    let home = inv.env.get("HOME").cloned().expect("home env");
    let tmpdir = inv.env.get("TMPDIR").cloned().expect("tmpdir env");
    assert_eq!(
        PathBuf::from(runtime_dir),
        podman_runtime_root(tmp.path()).join("run")
    );
    assert_eq!(PathBuf::from(home), podman_home_root(tmp.path()));
    assert_eq!(PathBuf::from(tmpdir), podman_temp_root(tmp.path()));
}

#[test]
fn keep_id_userns_is_only_enabled_on_linux() {
    assert_eq!(should_use_keep_id_userns(), cfg!(target_os = "linux"));
}

#[test]
fn rewrite_daemon_url_for_avf_guest_uses_guest_gateway_host() {
    let rewritten = super::container::rewrite_daemon_url_for_avf_guest("http://127.0.0.1:4399");
    assert_eq!(rewritten, "http://192.168.64.1:4399/");
}

#[tokio::test]
async fn ensure_avf_guest_gateway_proxy_forwards_to_loopback_backend() {
    let backend = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind backend listener");
    let backend_addr = backend.local_addr().expect("backend local addr");
    let proxy_port = {
        let temp = std::net::TcpListener::bind("127.0.0.1:0").expect("bind proxy port probe");
        temp.local_addr().expect("proxy port local addr").port()
    };
    let proxy_addr = format!("127.0.0.1:{proxy_port}");
    let backend_addr_str = backend_addr.to_string();

    let backend_task = tokio::spawn(async move {
        let (mut socket, _) = backend.accept().await.expect("accept backend connection");
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 4];
        socket
            .read_exact(&mut buf)
            .await
            .expect("read backend bytes");
        assert_eq!(&buf, b"ping");
        socket
            .write_all(b"pong")
            .await
            .expect("write backend reply");
    });

    super::ensure_avf_guest_gateway_proxy_for_test(&proxy_addr, &backend_addr_str, proxy_port)
        .await
        .expect("start gateway proxy");

    let mut client = tokio::net::TcpStream::connect(&proxy_addr)
        .await
        .expect("connect proxy");
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    client.write_all(b"ping").await.expect("write proxy bytes");
    let mut buf = [0u8; 4];
    client.read_exact(&mut buf).await.expect("read proxy bytes");
    assert_eq!(&buf, b"pong");

    backend_task.await.expect("backend task");
}

#[tokio::test]
async fn seed_shared_podman_machine_cache_populates_local_cache_root() {
    let _serial = env_var_test_lock().lock().await;
    let shared = tempfile::tempdir().expect("shared tempdir");
    let data_root = tempfile::tempdir().expect("data root tempdir");
    let _guard = EnvGuard::set(
        "CTX_PODMAN_MACHINE_CACHE_DIR",
        &shared.path().to_string_lossy(),
    );
    let relpath = PathBuf::from("applehv")
        .join("cache")
        .join("78e5fea350d7.raw.zst");
    let shared_file = shared.path().join(&relpath);
    std::fs::create_dir_all(shared_file.parent().expect("shared cache parent"))
        .expect("create shared cache dir");
    std::fs::write(&shared_file, b"seeded-machine-cache").expect("write shared cache file");

    seed_shared_podman_machine_cache(data_root.path(), None)
        .await
        .expect("seed shared cache");

    let local_file = podman_machine_cache_root(data_root.path()).join(relpath);
    let local_body = std::fs::read(&local_file).expect("read local cache file");
    assert_eq!(local_body, b"seeded-machine-cache");
}

#[tokio::test]
async fn persist_podman_machine_cache_to_shared_does_not_depend_on_local_path() {
    let _serial = env_var_test_lock().lock().await;
    let shared = tempfile::tempdir().expect("shared tempdir");
    let data_root = tempfile::tempdir().expect("data root tempdir");
    let _guard = EnvGuard::set(
        "CTX_PODMAN_MACHINE_CACHE_DIR",
        &shared.path().to_string_lossy(),
    );
    let relpath = PathBuf::from("applehv")
        .join("cache")
        .join("persisted-machine-cache.raw.zst");
    let local_file = podman_machine_cache_root(data_root.path()).join(&relpath);
    std::fs::create_dir_all(local_file.parent().expect("local cache parent"))
        .expect("create local cache dir");
    std::fs::write(&local_file, b"persisted-machine-cache").expect("write local cache file");

    persist_podman_machine_cache_to_shared(data_root.path(), None)
        .await
        .expect("persist shared cache");

    std::fs::remove_file(&local_file).expect("remove local cache file");
    let shared_file = shared.path().join(relpath);
    let shared_body = std::fs::read(&shared_file).expect("read shared cache file");
    assert_eq!(shared_body, b"persisted-machine-cache");
}

#[tokio::test]
async fn ensure_podman_machine_download_skips_when_machine_lock_is_busy() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
            log_path.display()
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    let manager = HarnessRuntimeManager::new(temp.path().to_path_buf());
    let machine_name = ctx_podman_machine_name(temp.path());
    let machine_lock = podman_machine_singleflight_lock(&machine_name);
    let _machine_guard = machine_lock.lock().await;

    manager
        .ensure_podman_machine_download()
        .await
        .expect("prefetch should skip while launch holds the machine lock");

    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(log.trim().is_empty());
    machine_cache_server.abort();
}

#[test]
fn missing_machine_error_detection_matches_expected_shapes() {
    assert!(looks_like_missing_machine_error(
        "error: no machine with this name exists"
    ));
    assert!(looks_like_missing_machine_error(
        "Error: machine ctx not found"
    ));
    assert!(!looks_like_missing_machine_error(
        "error: machine already running"
    ));
}

#[test]
fn recoverable_machine_start_error_detection_matches_expected_shapes() {
    assert!(looks_like_recoverable_machine_start_error(
        "error: machine is already starting"
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "Error: unable to start \"ctx\": already running\nStarting machine \"ctx\""
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "error: resource busy while acquiring lock"
    ));
    assert!(looks_like_recoverable_machine_start_error(
        "error: operation timed out while waiting for vm startup"
    ));
    assert!(looks_like_recoverable_machine_start_error(
            "time=\"2026-03-05T00:23:28-06:00\" level=warning msg=\"detected port conflict on machine ssh port [49401], reassigning\"\nError: vfkit exited unexpectedly with exit code 1"
        ));
    assert!(looks_like_recoverable_machine_start_error(
        "Error: unable to connect to \"gvproxy\" socket at \"/tmp/podman.sock\""
    ));
    assert!(!looks_like_recoverable_machine_start_error(
        "error: unknown vm provider configuration"
    ));
}

#[test]
fn running_but_unreachable_machine_start_error_detection_matches_expected_shapes() {
    assert!(looks_like_running_but_unreachable_machine_start_error(
        "Error: unable to start \"ctx\": already running"
    ));
    assert!(looks_like_running_but_unreachable_machine_start_error(
        "Error: unable to connect to \"gvproxy\" socket at \"/tmp/podman.sock\""
    ));
    assert!(!looks_like_running_but_unreachable_machine_start_error(
        "error: resource busy while acquiring lock"
    ));
    assert!(!looks_like_running_but_unreachable_machine_start_error(
        "error: operation timed out while waiting for vm startup"
    ));
}

#[test]
fn collect_ctx_managed_podman_helper_pids_matches_only_ctx_scoped_helpers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join("macos")
        .join("aarch64")
        .join("podman-5.8.0")
        .join("usr")
        .join("libexec")
        .join("podman");
    let matches = collect_ctx_managed_podman_helper_pids(
        vec![
            (
                42,
                vec![
                    helper_dir.join("gvproxy").to_string_lossy().into_owned(),
                    machine_name.clone(),
                    podman_temp_root(temp.path())
                        .join("podman")
                        .join(format!("{machine_name}-api.sock"))
                        .to_string_lossy()
                        .into_owned(),
                ],
            ),
            (
                77,
                vec![
                    "/opt/homebrew/bin/vfkit".to_string(),
                    temp.path()
                        .join("podman")
                        .join("xdg")
                        .join("data")
                        .join("containers")
                        .join("podman")
                        .join("machine")
                        .join("applehv")
                        .join(format!("{machine_name}-arm64.raw"))
                        .to_string_lossy()
                        .into_owned(),
                    machine_name.clone(),
                ],
            ),
            (
                88,
                vec![
                    "/opt/homebrew/libexec/podman/gvproxy".to_string(),
                    "/tmp/podman/podman-machine-default-api.sock".to_string(),
                    "podman-machine-default".to_string(),
                ],
            ),
        ],
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![42, 77]);
}

#[test]
fn ctx_managed_podman_helper_process_detection_matches_expected_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let matching_gvproxy = vec![
        temp.path()
            .join("managed")
            .join("runtimes")
            .join("podman")
            .join("macos")
            .join("aarch64")
            .join("podman-5.8.0")
            .join("usr")
            .join("libexec")
            .join("podman")
            .join("gvproxy")
            .to_string_lossy()
            .into_owned(),
        podman_temp_root(temp.path())
            .join("podman")
            .join(format!("{machine_name}-api.sock"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_podman_helper_process_command(
        &matching_gvproxy,
        temp.path(),
        &machine_name
    ));

    let matching_vfkit = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("podman")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("podman")
            .join("machine")
            .join("applehv")
            .join(format!("{machine_name}-arm64.raw"))
            .to_string_lossy()
            .into_owned(),
        machine_name.clone(),
    ];
    assert!(is_ctx_managed_podman_helper_process_command(
        &matching_vfkit,
        temp.path(),
        &machine_name
    ));

    let wrong_machine = vec![
        String::from("/opt/homebrew/bin/vfkit"),
        temp.path()
            .join("podman")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("podman")
            .join("machine")
            .join("applehv")
            .join("ctx-someone-else-arm64.raw")
            .to_string_lossy()
            .into_owned(),
    ];
    assert!(!is_ctx_managed_podman_helper_process_command(
        &wrong_machine,
        temp.path(),
        &machine_name
    ));

    let host_helper = vec![
        String::from("/opt/homebrew/libexec/podman/gvproxy"),
        String::from("/tmp/podman/podman-machine-default-api.sock"),
        String::from("podman-machine-default"),
    ];
    assert!(!is_ctx_managed_podman_helper_process_command(
        &host_helper,
        temp.path(),
        &machine_name
    ));
}

#[test]
fn collect_ctx_managed_podman_helper_pids_from_ps_output_matches_real_macos_shapes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join("macos")
        .join("aarch64")
        .join("podman-5.8.0")
        .join("usr")
        .join("libexec")
        .join("podman");
    let gvproxy_line = format!(
            " 6622 {} -mtu 1500 -listen-vfkit unixgram://{} -forward-sock {} -forward-identity {} -pid-file {}/gvproxy.pid",
            helper_dir.join("gvproxy").display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-api.sock"))
                .display(),
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("machine")
                .display(),
            podman_temp_root(temp.path()).join("podman").display(),
        );
    let vfkit_line = format!(
            "12484 /Users/example-user/Library/Application Support/vfkit --device virtio-blk,path={} --device virtio-vsock,port=1025,socketURL={} --device virtio-net,unixSocketPath={}",
            temp.path()
                .join("podman")
                .join("xdg")
                .join("data")
                .join("containers")
                .join("podman")
                .join("machine")
                .join("applehv")
                .join(format!("{machine_name}-arm64.raw"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}.sock"))
                .display(),
            podman_temp_root(temp.path())
                .join("podman")
                .join(format!("{machine_name}-gvproxy.sock"))
                .display(),
        );
    let host_line = String::from(
            "88 /opt/homebrew/libexec/podman/gvproxy -forward-sock /tmp/podman/podman-machine-default-api.sock podman-machine-default",
        );
    let ps_output = format!("{gvproxy_line}\n{vfkit_line}\n{host_line}\n");

    let matches = collect_ctx_managed_podman_helper_pids_from_ps_output(
        &ps_output,
        temp.path(),
        &machine_name,
    );
    assert_eq!(matches, vec![6622, 12484]);
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test]
async fn ensure_podman_machine_running_recreates_immediately_for_already_running_unreachable_machine(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-ready");
    let start_count_path = temp.path().join("podman-start-count");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nSTART_COUNT=\"{start_count}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STATE\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  count=0\n  if [ -f \"$START_COUNT\" ]; then\n    count=$(cat \"$START_COUNT\")\n  fi\n  count=$((count + 1))\n  printf '%s' \"$count\" > \"$START_COUNT\"\n  if [ \"$count\" -eq 1 ]; then\n    echo 'Error: unable to start \"ctx\": already running' >&2\n    exit 125\n  fi\n  touch \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  rm -f \"$STATE\"\n  exit 0\nfi\nexit 0\n",
                log = log_path.display(),
                state = state_path.display(),
                start_count = start_count_path.display(),
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    ensure_podman_machine_running_with_observer(temp.path(), None)
        .await
        .expect("already-running unreachable machine should recover");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("info"));
    assert!(log.contains("machine start "));
    assert!(log.contains("machine inspect "));
    assert!(log.contains("machine rm -f "));
    assert!(log.contains("machine init "));
    assert!(!log.contains("machine stop "));
    machine_cache_server.abort();
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::test]
async fn ensure_podman_machine_running_fails_fast_on_unknown_start_error() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'podman socket unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  echo 'error: unknown vm provider configuration' >&2\n  exit 125\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let err = ensure_podman_machine_running_with_observer(temp.path(), None)
        .await
        .expect_err("unknown start error should fail");
    let message = format!("{err:#}");
    assert!(message.contains("unknown vm provider configuration"));

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("machine start "));
    assert!(!log.contains("machine stop "));
    assert!(!log.contains("machine rm -f "));
}

#[tokio::test]
async fn initialize_podman_machine_uses_init_then_start_without_now() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{}\"\nexit 0\n",
            log_path.display()
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    let mut last_err = String::new();
    initialize_podman_machine(temp.path(), "ctx-test-machine", None, None, &mut last_err)
        .await
        .expect("initialize machine");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("machine init ctx-test-machine"));
    assert!(log.contains("machine start ctx-test-machine"));
    assert!(!log.contains("--now"));
    assert!(last_err.is_empty());
    machine_cache_server.abort();
}

#[tokio::test]
async fn initialize_podman_machine_terminates_stuck_init_when_machine_is_present() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exec sleep 30\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  exit 0\nfi\nexit 0\n",
                log_path.display()
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;

    let mut last_err = String::new();
    // Keep the outer test timeout comfortably above a single inspect timeout so the test
    // validates the kill-and-continue recovery path instead of host scheduling variance.
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        initialize_podman_machine(temp.path(), "ctx-test-machine", None, None, &mut last_err),
    )
    .await;
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    let init_result = result
        .unwrap_or_else(|_| panic!("initialize_podman_machine timed out; invocation log:\n{log}"));
    init_result.expect("initialize machine");

    assert!(log.contains("machine init ctx-test-machine"));
    assert!(log.contains("machine inspect ctx-test-machine"));
    assert!(log.contains("machine start ctx-test-machine"));
    assert!(!log.contains("--now"));
    machine_cache_server.abort();
}

#[tokio::test]
async fn ensure_podman_machine_materialized_recreates_machine_for_memory_profile_change_when_engine_is_down(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'podman machine is stopped' >&2\n  exit 125\nfi\nif [ \"$1\" = \"ps\" ]; then\n  echo 'podman machine is stopped' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"State\":\"stopped\",\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_podman_machine_materialized(&settings, None)
        .await
        .expect("machine should be recreated with desired memory");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains("ps --format {{.Names}}"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(log.contains(&format!("machine stop {machine_name}")));
    assert!(log.contains(&format!("machine rm -f {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(log.contains("--memory 12288"));
    machine_cache_server.abort();
}

#[tokio::test]
async fn ensure_podman_machine_materialized_defers_reconfiguration_when_machine_is_running_but_engine_unreachable(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"ps\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"State\":\"running\",\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_podman_machine_materialized(&settings, None)
        .await
        .expect("running-but-unreachable machine should defer destructive reconfiguration");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains("ps --format {{.Names}}"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
}

#[tokio::test]
async fn ensure_podman_machine_materialized_defers_reconfiguration_when_machine_state_is_unknown_and_engine_unreachable(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"ps\" ]; then\n  echo 'unable to connect to \"gvproxy\" socket' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_podman_machine_materialized(&settings, None)
        .await
        .expect("unknown runtime state should defer destructive reconfiguration");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.lines().any(|line| line == "info"));
    assert!(log.contains("ps --format {{.Names}}"));
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
}

#[tokio::test]
async fn maybe_reclaim_podman_machine_stops_idle_machine() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            idle_shutdown_seconds: 60,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let running_sessions = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let terminals = crate::terminals::TerminalManager::default();
    let snapshot = SystemSnapshot {
        cpu_pct: 0.0,
        memory_total_bytes: 32 * 1024 * 1024 * 1024,
        memory_used_bytes: 8 * 1024 * 1024 * 1024,
        swap_total_bytes: 4 * 1024 * 1024 * 1024,
        swap_used_bytes: 0,
    };

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            &terminals,
        )
        .await
        .expect("idle reclaim should succeed");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(stopped);
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(log.contains(&format!("machine stop {machine_name}")));
}

#[tokio::test]
async fn maybe_reclaim_podman_machine_clamps_short_idle_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(30);
    }
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            idle_shutdown_seconds: 5,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let running_sessions = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let terminals = crate::terminals::TerminalManager::default();
    let snapshot = SystemSnapshot {
        cpu_pct: 0.0,
        memory_total_bytes: 32 * 1024 * 1024 * 1024,
        memory_used_bytes: 8 * 1024 * 1024 * 1024,
        swap_total_bytes: 4 * 1024 * 1024 * 1024,
        swap_used_bytes: 0,
    };

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            &terminals,
        )
        .await
        .expect("reclaim check should succeed");

    assert!(!stopped);
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(!log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
}

#[tokio::test]
async fn maybe_reclaim_podman_machine_stops_idle_runtime_with_running_workspace_containers() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"ps\" ]; then\n  printf 'ctx-harness-running\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            idle_shutdown_seconds: 60,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let running_sessions = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let terminals = crate::terminals::TerminalManager::default();
    let snapshot = SystemSnapshot {
        cpu_pct: 0.0,
        memory_total_bytes: 32 * 1024 * 1024 * 1024,
        memory_used_bytes: 8 * 1024 * 1024 * 1024,
        swap_total_bytes: 4 * 1024 * 1024 * 1024,
        swap_used_bytes: 0,
    };

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            &terminals,
        )
        .await
        .expect("reclaim check should succeed");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(stopped);
    assert!(log.contains(&format!("machine stop {machine_name}")));
}

#[tokio::test]
async fn maybe_reclaim_podman_machine_skips_active_container_sessions() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let running_sessions = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let session_id = create_session_with_environment(
        &stores,
        temp.path(),
        ExecutionEnvironment::ContainerHostMounted,
    )
    .await;
    running_sessions.lock().await.insert(session_id);
    let terminals = crate::terminals::TerminalManager::default();
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            idle_shutdown_seconds: 60,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };
    let snapshot = SystemSnapshot {
        cpu_pct: 0.0,
        memory_total_bytes: 32 * 1024 * 1024 * 1024,
        memory_used_bytes: 8 * 1024 * 1024 * 1024,
        swap_total_bytes: 4 * 1024 * 1024 * 1024,
        swap_used_bytes: 0,
    };

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            &terminals,
        )
        .await
        .expect("reclaim check should succeed");

    assert!(!stopped);
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(!log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));

    running_sessions.lock().await.clear();
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            &terminals,
        )
        .await
        .expect("reclaim should resume once session stops");
    assert!(stopped);
}

#[tokio::test]
async fn maybe_reclaim_podman_machine_skips_running_container_terminals() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  sleep 60\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }
    let stores = StoreManager::open(temp.path()).await.expect("open stores");
    let running_sessions = Arc::new(tokio::sync::Mutex::new(HashSet::new()));
    let terminals = Arc::new(crate::terminals::TerminalManager::default());
    let terminal = terminals
        .create(crate::terminals::TerminalCreateRequest {
            workspace_id: WorkspaceId::new(),
            task_id: Some(TaskId::new()),
            session_id: None,
            worktree_id: Some(WorktreeId::new()),
            cwd: temp.path().to_path_buf(),
            shell: "/bin/sh".to_string(),
            cols: None,
            rows: None,
            env: HashMap::new(),
            podman: Some(crate::terminals::PodmanTerminalSpec {
                podman_bin: podman_path.clone(),
                podman_env: HashMap::new(),
                container_name: "ctx-harness-terminal".to_string(),
                workdir: "/workspace".to_string(),
            }),
            avf_linux_vm: None,
        })
        .await
        .expect("create terminal");
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            idle_shutdown_seconds: 60,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };
    let snapshot = SystemSnapshot {
        cpu_pct: 0.0,
        memory_total_bytes: 32 * 1024 * 1024 * 1024,
        memory_used_bytes: 8 * 1024 * 1024 * 1024,
        swap_total_bytes: 4 * 1024 * 1024 * 1024,
        swap_used_bytes: 0,
    };

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            terminals.as_ref(),
        )
        .await
        .expect("reclaim check should succeed");

    assert!(!stopped);
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    assert!(!log.contains(&format!("machine inspect {machine_name}")));
    assert!(!log.contains(&format!("machine stop {machine_name}")));

    terminal.kill().expect("kill terminal");
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let mut last_activity = manager
            .last_activity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *last_activity = Instant::now() - Duration::from_secs(600);
    }

    let stopped = manager
        .maybe_reclaim_podman_machine(
            &settings,
            &snapshot,
            None,
            &stores,
            &running_sessions,
            terminals.as_ref(),
        )
        .await
        .expect("reclaim should resume once terminal exits");
    assert!(stopped);
}

#[tokio::test]
async fn ensure_container_machine_ready_reconfigures_running_machine_when_idle() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-state.txt");
    std::fs::write(&state_path, "running\n").expect("seed podman state");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nstate=$(cat \"$STATE\" 2>/dev/null || true)\nif [ \"$1\" = \"info\" ]; then\n  [ \"$state\" = \"running\" ] && exit 0\n  exit 1\nfi\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  printf 'stopped\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  printf 'absent\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  printf 'initialized\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  printf 'running\\n' > \"$STATE\"\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let (_machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("idle running machine should be reconfigured and restarted");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(log.contains("ps --format {{.Names}}"));
    assert!(log.contains(&format!("machine stop {machine_name}")));
    assert!(log.contains(&format!("machine rm -f {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(log.contains("--memory 12288"));
    assert!(log.contains(&format!("machine start {machine_name}")));
    machine_cache_server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_container_machine_ready_prefetches_avf_runtime_without_starting_a_global_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let settings = ContainerExecutionSettings {
        runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("AVF runtime prefetch should succeed");

    let runtime_state = super::selected_runtime_state(temp.path(), &settings)
        .await
        .expect("read AVF runtime state");
    assert_eq!(runtime_state, (true, true));

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn prepare_returns_avf_linux_vm_plan_after_workspace_vm_and_guest_worktree_ready() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    let plan = manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should now return a guest-exec plan");
    match &plan.runtime {
        HarnessRuntimeKind::AvfLinuxVm => {}
        HarnessRuntimeKind::Host => panic!("expected AVF Linux VM runtime"),
        HarnessRuntimeKind::Container { .. } => panic!("expected AVF Linux VM runtime"),
    }
    assert_eq!(
        plan.env_overrides
            .get(crate::workspace_runtime::CTX_HARNESS_RUNTIME_KIND_ENV)
            .map(String::as_str),
        Some("avf_linux_vm")
    );
    let workspace_id = workspace.id.0.to_string();
    let worktree_id = worktree.id.0.to_string();
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_WORKSPACE_ID")
            .map(String::as_str),
        Some(workspace_id.as_str())
    );
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_WORKTREE_ID")
            .map(String::as_str),
        Some(worktree_id.as_str())
    );
    assert_eq!(
        plan.env_overrides
            .get("CTX_AVF_HOST_WORKTREE_ROOT")
            .map(String::as_str),
        Some(worktree.root_path.as_str())
    );
    assert_eq!(
        plan.env_overrides.get("CTX_DAEMON_URL").map(String::as_str),
        Some("http://192.168.64.1:4399")
    );
    assert!(plan
        .env_overrides
        .contains_key(super::avf_linux_vm::AVF_LINUX_HELPER_PATH_ENV));

    let state = super::avf_linux_vm::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        super::avf_linux_vm::AvfLinuxSharedVmLifecycleState::Running
    );
    let guest_worktree = super::avf_linux_vm::prepare_guest_worktree(
        temp.path(),
        workspace.id,
        worktree.id,
        Path::new(&workspace.root_path),
        &worktree.base_commit_sha,
        worktree.git_branch.as_deref().expect("git branch"),
    )
    .expect("guest worktree state");
    assert_eq!(
        guest_worktree.status,
        super::avf_linux_vm::AvfLinuxGuestWorktreeStatus::AlreadyPresent
    );
    assert!(guest_worktree.host_shadow_root.exists());

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn container_status_reports_running_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should start the workspace VM");

    let status = manager
        .container_status(workspace.id)
        .await
        .expect("AVF workspace status")
        .expect("AVF workspace VM status should exist");
    assert_eq!(status.name, format!("ctx-avf-linux-vm-{}", workspace.id.0));
    assert!(status.running);
    assert!(status.known);
    assert_eq!(status.mount_mode, Some(ContainerMountMode::DiskIsolated));

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_starts_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_workspace_container(&workspace, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF workspace VM should be started for workspace container callers");

    let state = super::avf_linux_vm::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        super::avf_linux_vm::AvfLinuxSharedVmLifecycleState::Running
    );

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_for_worktree_prepares_avf_guest_worktree() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_workspace_container_for_worktree(
            &workspace,
            &worktree,
            &settings,
            "http://192.168.64.1:4399",
        )
        .await
        .expect("AVF worktree ensure should prepare the guest worktree");

    let prepared = super::avf_linux_vm::prepare_guest_worktree(
        temp.path(),
        workspace.id,
        worktree.id,
        Path::new(&workspace.root_path),
        &worktree.base_commit_sha,
        &format!("ctx/{}/{}", workspace.id.0, worktree.id.0),
    )
    .expect("AVF guest worktree metadata should already exist");
    assert_eq!(
        prepared.status,
        super::avf_linux_vm::AvfLinuxGuestWorktreeStatus::AlreadyPresent
    );

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_workspace_container_after_runtime_ready_starts_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .ensure_container_machine_ready(&settings.container, None)
        .await
        .expect("AVF runtime artifacts should prewarm");
    manager
        .ensure_workspace_container_after_runtime_ready_with_observer(
            &workspace,
            &settings,
            "http://192.168.64.1:4399",
            None,
        )
        .await
        .expect("AVF workspace VM should start from runtime-ready path");

    let state = super::avf_linux_vm::workspace_vm_state(temp.path(), workspace.id)
        .expect("workspace VM state");
    assert_eq!(
        state.state,
        super::avf_linux_vm::AvfLinuxSharedVmLifecycleState::Running
    );

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn stop_container_stops_avf_workspace_vm() {
    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let _helper_guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, &helper_path.to_string_lossy());
    let (_runtime_guard, servers) = install_test_managed_avf_linux_runtime_source().await;
    let workspace = sample_workspace(&temp);
    let worktree = sample_worktree(&temp, workspace.id);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Container,
        container: ContainerExecutionSettings {
            runtime: crate::settings::ContainerRuntimeKind::AvfLinuxVm,
            mount_mode: ContainerMountMode::DiskIsolated,
            ..ContainerExecutionSettings::default()
        },
    };

    manager
        .prepare(&workspace, &worktree, &settings, "http://192.168.64.1:4399")
        .await
        .expect("AVF prepare should start the workspace VM");

    assert!(manager
        .stop_container(workspace.id)
        .await
        .expect("stop AVF workspace VM"));

    let status = manager
        .container_status(workspace.id)
        .await
        .expect("read stopped AVF workspace VM status")
        .expect("AVF workspace VM status should still exist after stop");
    assert!(!status.running);

    for server in servers {
        server.abort();
    }
}

#[tokio::test]
async fn ensure_container_machine_ready_defers_running_machine_reconfiguration_for_active_workspace_containers(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-state.txt");
    std::fs::write(&state_path, "running\n").expect("seed podman state");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nstate=$(cat \"$STATE\" 2>/dev/null || true)\nif [ \"$1\" = \"info\" ]; then\n  [ \"$state\" = \"running\" ] && exit 0\n  exit 1\nfi\nif [ \"$1\" = \"ps\" ]; then\n  printf 'ctx-harness-active\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  printf 'stopped\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  printf 'absent\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  printf 'initialized\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  printf 'running\\n' > \"$STATE\"\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let settings = ContainerExecutionSettings {
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("active running containers should defer reconfiguration without failing launch");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains(&format!("machine inspect {machine_name}")));
    assert!(log.contains("ps --format {{.Names}}"));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
    assert!(!log.contains(&format!("machine start {machine_name}")));
}

#[tokio::test]
async fn ensure_container_machine_ready_reconfigures_disk_isolated_machine_without_workspace_volumes(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-state.txt");
    std::fs::write(&state_path, "running\n").expect("seed podman state");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nstate=$(cat \"$STATE\" 2>/dev/null || true)\nif [ \"$1\" = \"info\" ]; then\n  [ \"$state\" = \"running\" ] && exit 0\n  exit 1\nfi\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"ls\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  printf 'stopped\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  printf 'absent\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  printf 'initialized\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  printf 'running\\n' > \"$STATE\"\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let (machine_cache_guard, machine_cache_server) =
        install_test_managed_machine_cache_source(b"machine-cache".to_vec()).await;
    assert!(
        crate::bundled_assets::managed_podman_machine_cache_source().is_some(),
        "test machine cache override should be installed before reconfiguration"
    );
    let _keep_machine_cache_guard_alive = &machine_cache_guard;
    let settings = ContainerExecutionSettings {
        mount_mode: ContainerMountMode::DiskIsolated,
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("disk-isolated machine without workspace volumes should be reconfigured");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("volume ls --format {{.Name}}"));
    assert!(log.contains(&format!("machine stop {machine_name}")));
    assert!(log.contains(&format!("machine rm -f {machine_name}")));
    assert!(log.contains(&format!("machine init {machine_name}")));
    assert!(log.contains(&format!("machine start {machine_name}")));
    machine_cache_server.abort();
}

#[tokio::test]
async fn ensure_container_machine_ready_defers_disk_isolated_reconfiguration_when_workspace_volumes_exist(
) {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let manager = runtime_manager(&temp).await;
    let machine_name = ctx_podman_machine_name(temp.path());
    let log_path = temp.path().join("podman-invocations.log");
    let state_path = temp.path().join("podman-state.txt");
    std::fs::write(&state_path, "running\n").expect("seed podman state");
    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
        &podman_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTATE=\"{state}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nstate=$(cat \"$STATE\" 2>/dev/null || true)\nif [ \"$1\" = \"info\" ]; then\n  [ \"$state\" = \"running\" ] && exit 0\n  exit 1\nfi\nif [ \"$1\" = \"ps\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"ls\" ]; then\n  printf 'ctx-ws-existing\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  printf '[{{\"Resources\":{{\"Memory\":2048}}}}]\\n'\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"stop\" ]; then\n  printf 'stopped\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"rm\" ] && [ \"$3\" = \"-f\" ]; then\n  printf 'absent\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  printf 'initialized\\n' > \"$STATE\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  printf 'running\\n' > \"$STATE\"\n  exit 0\nfi\necho \"unexpected podman invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            state = state_path.display(),
        ),
    )
    .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());
    let _host_memory = EnvGuard::set("CTX_TEST_HOST_MEMORY_MB", "49152");
    let settings = ContainerExecutionSettings {
        mount_mode: ContainerMountMode::DiskIsolated,
        machine: crate::settings::ContainerMachineSettings {
            memory_profile: crate::settings::ContainerMachineMemoryProfile::Balanced,
            ..crate::settings::ContainerMachineSettings::default()
        },
        ..ContainerExecutionSettings::default()
    };

    manager
        .ensure_container_machine_ready(&settings, None)
        .await
        .expect("existing disk-isolated workspace volumes should defer reconfiguration");

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert!(log.contains("volume ls --format {{.Name}}"));
    assert!(!log.contains(&format!("machine stop {machine_name}")));
    assert!(!log.contains(&format!("machine rm -f {machine_name}")));
    assert!(!log.contains(&format!("machine init {machine_name}")));
}

#[test]
fn podman_machine_temp_state_paths_match_expected_names() {
    let data_root = tempfile::tempdir().expect("tempdir");
    let paths = podman_machine_temp_state_paths(data_root.path(), "ctx");
    let rendered: Vec<String> = paths
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let expected_tmp_prefix = podman_temp_root(data_root.path())
        .join("podman")
        .to_string_lossy()
        .to_string();
    assert!(rendered.iter().any(|p| p.starts_with(&expected_tmp_prefix)));
    assert!(rendered.iter().any(|p| p.ends_with("podman/gvproxy.pid")));
    assert!(rendered.iter().any(|p| p.ends_with("podman/ctx-api.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("podman/ctx-gvproxy.sock")));
    assert!(rendered.iter().any(|p| p.ends_with("podman/ctx.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("home/.podman/ctx-api.sock")));
    assert!(rendered
        .iter()
        .any(|p| p.ends_with("home/.podman/ctx-gvproxy.sock")));
}

#[tokio::test]
async fn podman_machine_singleflight_lock_reuses_lock_for_same_machine() {
    let first = podman_machine_singleflight_lock("ctx-machine-a");
    let second = podman_machine_singleflight_lock("ctx-machine-a");
    assert!(Arc::ptr_eq(&first, &second));

    let guard = first.lock().await;
    assert!(second.try_lock().is_err());
    drop(guard);
    assert!(second.try_lock().is_ok());
}

#[tokio::test]
async fn podman_machine_singleflight_lock_isolated_by_machine_name() {
    let first = podman_machine_singleflight_lock("ctx-machine-b");
    let second = podman_machine_singleflight_lock("ctx-machine-c");
    assert!(!Arc::ptr_eq(&first, &second));

    let _guard = first.lock().await;
    assert!(second.try_lock().is_ok());
}

#[test]
fn transparent_proxy_policy_maps_llm_only_to_explicit_allowlist_entries() {
    let settings = ContainerExecutionSettings::default();
    let (mode, allowlist) = transparent_proxy_policy(&settings);
    assert_eq!(mode, ContainerNetworkMode::Allowlist);
    assert!(allowlist.iter().any(|entry| entry == "openrouter.ai"));
    assert!(allowlist.iter().any(|entry| entry == "api.openai.com"));
}

#[test]
fn transparent_proxy_policy_preserves_custom_allowlist_mode() {
    let settings = ContainerExecutionSettings {
        network_mode: ContainerNetworkMode::Allowlist,
        allowlist: vec!["example.com".to_string(), "api.example.com".to_string()],
        ..Default::default()
    };
    let (mode, allowlist) = transparent_proxy_policy(&settings);
    assert_eq!(mode, ContainerNetworkMode::Allowlist);
    assert_eq!(allowlist, settings.allowlist);
}

#[cfg(unix)]
#[tokio::test]
async fn unrestricted_network_transition_surfaces_teardown_failures() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_log_path = temp.path().join("cleanup-helpers.log");
    let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
    std::fs::write(&pid_file_path, b"\n").expect("write fake proxy pid file");
    let _pid_file_guard = EnvGuard::set(
        "CTX_EGRESS_PROXY_PID_FILE",
        &pid_file_path.to_string_lossy(),
    );

    let rm_path = fakebin.join("rm");
    std::fs::write(
            &rm_path,
            format!(
                "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\necho 'failed to remove proxy pid file' >&2\nexit 23\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake rm");
    std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake rm");

    let iptables_path = fakebin.join("iptables");
    std::fs::write(
            &iptables_path,
            format!(
                "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nif [ \"$1\" = \"-P\" ] && [ \"$2\" = \"OUTPUT\" ] && [ \"$3\" = \"ACCEPT\" ]; then\n  echo 'failed to reset output policy' >&2\n  exit 42\nfi\nexit 0\n",
                log = helper_log_path.display(),
            ),
        )
        .expect("write fake iptables");
    std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake iptables");

    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let settings = ContainerExecutionSettings {
        network_mode: ContainerNetworkMode::All,
        ..Default::default()
    };
    let err = apply_container_network_policy(
        temp.path(),
        WorkspaceId::new(),
        "ctx-harness-test",
        &settings,
        "127.0.0.1",
        4399,
    )
    .await
    .expect_err("teardown failure should be explicit");

    let message = format!("{err:#}");
    assert!(message.contains("failed to tear down restricted container network policy"));
    assert!(message.contains("stop transparent proxy"));
    assert!(message.contains("failed to remove proxy pid file"));
    assert!(message.contains("clear egress guard"));
    assert!(message.contains("failed to reset output policy"));

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert_eq!(
        log.lines().filter(|line| line.starts_with("exec ")).count(),
        2,
        "expected both teardown steps to run before surfacing the failure"
    );

    let helper_log =
        std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
    assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
    assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
    assert!(helper_log.contains("iptables -F OUTPUT"));
    assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
}

#[cfg(unix)]
#[tokio::test]
async fn unrestricted_network_transition_ignores_stale_proxy_pid_file() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let log_path = temp.path().join("podman-invocations.log");
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_log_path = temp.path().join("cleanup-helpers.log");
    let pid_file_path = temp.path().join("ctx-egress-proxy.pid");
    std::fs::write(&pid_file_path, b"999999\n").expect("write stale proxy pid file");
    let _pid_file_guard = EnvGuard::set(
        "CTX_EGRESS_PROXY_PID_FILE",
        &pid_file_path.to_string_lossy(),
    );

    let rm_path = fakebin.join("rm");
    std::fs::write(
        &rm_path,
        format!(
            "#!/bin/sh\nprintf 'rm %s\\n' \"$*\" >> \"{log}\"\nexec /bin/rm \"$@\"\n",
            log = helper_log_path.display(),
        ),
    )
    .expect("write fake rm");
    std::fs::set_permissions(&rm_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake rm");

    let iptables_path = fakebin.join("iptables");
    std::fs::write(
        &iptables_path,
        format!(
            "#!/bin/sh\nprintf 'iptables %s\\n' \"$*\" >> \"{log}\"\nexit 0\n",
            log = helper_log_path.display(),
        ),
    )
    .expect("write fake iptables");
    std::fs::set_permissions(&iptables_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake iptables");

    let podman_path = temp.path().join("podman.sh");
    std::fs::write(
            &podman_path,
            format!(
                "#!/bin/sh\nLOG=\"{log}\"\nFAKEBIN=\"{fakebin}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"exec\" ]; then\n  PATH=\"$FAKEBIN:$PATH\" /bin/sh -c \"$7\"\n  exit $?\nfi\nexit 0\n",
                log = log_path.display(),
                fakebin = fakebin.display(),
            ),
        )
        .expect("write podman shim");
    std::fs::set_permissions(&podman_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod podman shim");
    let _guard = EnvGuard::set("CTX_PODMAN_PATH", &podman_path.to_string_lossy());

    let settings = ContainerExecutionSettings {
        network_mode: ContainerNetworkMode::All,
        ..Default::default()
    };
    let applied = apply_container_network_policy(
        temp.path(),
        WorkspaceId::new(),
        "ctx-harness-test",
        &settings,
        "127.0.0.1",
        4399,
    )
    .await
    .expect("stale proxy pid should be ignored during unrestricted teardown");

    assert!(!applied.egress_guard);
    assert!(
        !pid_file_path.exists(),
        "stale proxy pid file should be removed during teardown"
    );

    let log = std::fs::read_to_string(&log_path).expect("read invocation log");
    assert_eq!(
        log.lines().filter(|line| line.starts_with("exec ")).count(),
        2,
        "expected both unrestricted teardown steps to run"
    );

    let helper_log =
        std::fs::read_to_string(&helper_log_path).expect("read cleanup helper invocation log");
    assert!(helper_log.contains(&format!("rm -f {}", pid_file_path.display())));
    assert!(helper_log.contains("iptables -t nat -F OUTPUT"));
    assert!(helper_log.contains("iptables -F OUTPUT"));
    assert!(helper_log.contains("iptables -P OUTPUT ACCEPT"));
}

#[cfg(unix)]
#[test]
fn kill_ctx_managed_podman_helper_processes_reports_only_successful_kills() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join("macos")
        .join("aarch64")
        .join("podman-5.8.0")
        .join("usr")
        .join("libexec")
        .join("podman");
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        podman_temp_root(temp.path())
            .join("podman")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );
    let vfkit = format!(
        "/opt/homebrew/bin/vfkit --device virtio-blk,path={} --device virtio-net,unixSocketPath={}",
        temp.path()
            .join("podman")
            .join("xdg")
            .join("data")
            .join("containers")
            .join("podman")
            .join("machine")
            .join("applehv")
            .join(format!("{machine_name}-arm64.raw"))
            .display(),
        podman_temp_root(temp.path())
            .join("podman")
            .join(format!("{machine_name}-gvproxy.sock"))
            .display(),
    );
    let escaped_gvproxy = literal_pkill_pattern(&gvproxy);
    let escaped_vfkit = literal_pkill_pattern(&vfkit);

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n12484 {vfkit}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf '12484 {vfkit}\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
                vfkit = vfkit,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
            &kill_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nif [ \"$4\" = '{vfkit}' ]; then\n  exit 1\nfi\nexit 0\n",
                log = pkill_log_path.display(),
                vfkit = escaped_vfkit,
            ),
        )
        .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
    assert_eq!(outcome.killed, vec![6622]);
    assert_eq!(outcome.failed, vec![12484]);
    assert!(outcome.skipped.is_empty());

    let kill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert!(kill_log.contains(&format!("-9 -f -x {escaped_gvproxy}")));
    assert!(kill_log.contains(&format!("-9 -f -x {escaped_vfkit}")));
}

#[cfg(unix)]
#[test]
fn kill_ctx_managed_podman_helper_processes_escapes_regex_metacharacters_for_pkill() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join("macos")
        .join("aarch64")
        .join("podman-5.8.0")
        .join("usr")
        .join("libexec")
        .join("podman");
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        podman_temp_root(temp.path())
            .join("podman")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
        &kill_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$4\" >> \"{log}\"\nexit 0\n",
            log = pkill_log_path.display(),
        ),
    )
    .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
    assert_eq!(outcome.killed, vec![6622]);
    assert!(outcome.failed.is_empty());
    assert!(outcome.skipped.is_empty());

    let pkill_pattern = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert_eq!(pkill_pattern.trim(), literal_pkill_pattern(&gvproxy));
}

#[cfg(unix)]
#[test]
fn kill_ctx_managed_podman_helper_processes_skips_reused_pid_after_command_scoped_kill() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().blocking_lock();
    let temp = tempfile::tempdir().expect("tempdir");
    let machine_name = ctx_podman_machine_name(temp.path());
    let fakebin = temp.path().join("fakebin");
    std::fs::create_dir_all(&fakebin).expect("create fakebin");
    let helper_dir = temp
        .path()
        .join("managed")
        .join("runtimes")
        .join("podman")
        .join("macos")
        .join("aarch64")
        .join("podman-5.8.0")
        .join("usr")
        .join("libexec")
        .join("podman");
    let pkill_log_path = temp.path().join("pkill-invocations.log");
    let ps_count_path = temp.path().join("ps-count");

    let gvproxy = format!(
        "{} -forward-sock {} {}",
        helper_dir.join("gvproxy").display(),
        podman_temp_root(temp.path())
            .join("podman")
            .join(format!("{machine_name}-api.sock"))
            .display(),
        machine_name,
    );

    let ps_path = fakebin.join("ps");
    std::fs::write(
            &ps_path,
            format!(
                "#!/bin/sh\ncount=0\nif [ -f \"{count_path}\" ]; then\n  count=$(cat \"{count_path}\")\nfi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"{count_path}\"\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 1 ]; then\n  printf ' 6622 {gvproxy}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-axo\" ] && [ \"$count\" -eq 2 ]; then\n  printf ' 6622 /usr/bin/python3 /tmp/not-ctx-helper.py\\n'\n  exit 0\nfi\nexit 1\n",
                count_path = ps_count_path.display(),
                gvproxy = gvproxy,
            ),
        )
        .expect("write fake ps");
    std::fs::set_permissions(&ps_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake ps");

    let kill_path = fakebin.join("pkill");
    std::fs::write(
        &kill_path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"{log}\"\nexit 1\n",
            log = pkill_log_path.display(),
        ),
    )
    .expect("write fake pkill");
    std::fs::set_permissions(&kill_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fake pkill");

    let prior_path = std::env::var("PATH").unwrap_or_default();
    let path_value = format!("{}:{prior_path}", fakebin.display());
    let _guard = EnvGuard::set("PATH", &path_value);

    let outcome = kill_ctx_managed_podman_helper_processes(temp.path(), &machine_name);
    assert!(outcome.killed.is_empty());
    assert!(outcome.failed.is_empty());
    assert_eq!(outcome.skipped, vec![6622]);
    let pkill_log = std::fs::read_to_string(&pkill_log_path).expect("read pkill log");
    assert!(pkill_log.contains(&format!("-9 -f -x {}", literal_pkill_pattern(&gvproxy))));
}

#[test]
fn cached_container_action_reuses_when_mounts_and_network_match() {
    let cached = sample_cached_container();
    let settings = sample_container_settings();
    let action = cached_container_action(&cached, &settings, &cached.external_mounts);
    assert_eq!(action, CachedContainerAction::Reuse);
}

#[test]
fn cached_container_action_recreates_when_mount_mode_changes() {
    let cached = sample_cached_container();
    let mut settings = sample_container_settings();
    settings.mount_mode = ContainerMountMode::DiskIsolated;
    let action = cached_container_action(&cached, &settings, &cached.external_mounts);
    assert_eq!(action, CachedContainerAction::Recreate);
}

#[test]
fn cached_container_action_recreates_when_external_mounts_change() {
    let cached = sample_cached_container();
    let settings = sample_container_settings();
    let mut changed_mounts = cached.external_mounts.clone();
    changed_mounts.insert("/tmp/another".to_string());
    let action = cached_container_action(&cached, &settings, &changed_mounts);
    assert_eq!(action, CachedContainerAction::Recreate);
}

#[test]
fn cached_container_action_reconfigures_when_network_mode_changes() {
    let cached = sample_cached_container();
    let mut settings = sample_container_settings();
    settings.network_mode = ContainerNetworkMode::All;
    let action = cached_container_action(&cached, &settings, &cached.external_mounts);
    assert_eq!(action, CachedContainerAction::Reconfigure);
}

#[test]
fn cached_container_action_reconfigures_when_allowlist_changes() {
    let cached = sample_cached_container();
    let mut settings = sample_container_settings();
    settings.allowlist = vec!["example.com".to_string()];
    let action = cached_container_action(&cached, &settings, &cached.external_mounts);
    assert_eq!(action, CachedContainerAction::Reconfigure);
}

#[test]
fn bundle_dir_mount_policy_matches_platform_expectations() {
    // Linux runtime is host-native; bundle mounts are always reachable.
    if cfg!(target_os = "linux") {
        assert!(should_mount_bundle_dir_in_container(Path::new(
            "/Applications/ctx.app/Contents/Resources/bundles"
        )));
        return;
    }

    // Podman-machine platforms cannot reliably mount non-home host paths (for example
    // /Applications in macOS release installs).
    if cfg!(target_os = "macos") || cfg!(target_os = "windows") {
        assert!(!should_mount_bundle_dir_in_container(Path::new(
            "/Applications/ctx.app/Contents/Resources/bundles"
        )));
        let home_var = if cfg!(target_os = "windows") {
            "USERPROFILE"
        } else {
            "HOME"
        };
        if let Some(home) = std::env::var_os(home_var).map(PathBuf::from) {
            assert!(should_mount_bundle_dir_in_container(
                &home.join("ctx-bundles")
            ));
        }
    }
}

#[test]
fn build_mounts_only_includes_bundle_dir_when_shareable() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bundle_dir = tmp.path().join("bundles");
    std::fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    let _guard = EnvGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

    let workspace = sample_workspace(&tmp);
    let mounts = build_mounts(
        tmp.path(),
        &workspace,
        None,
        &ContainerExecutionSettings::default(),
    )
    .mounts;
    let expected = bind_mount(&bundle_dir, &bundle_dir, true);
    let has_bundle_mount = mounts.iter().any(|mount| mount == &expected);
    assert_eq!(
        has_bundle_mount,
        should_mount_bundle_dir_in_container(&bundle_dir)
    );
}

#[tokio::test]
async fn managed_default_image_install_lock_serializes_callers() {
    let lock = managed_default_image_install_lock();
    let guard = lock.lock().await;
    let acquired = Arc::new(AtomicBool::new(false));
    let acquired_clone = Arc::clone(&acquired);

    let waiter = tokio::spawn(async move {
        let _wait_guard = lock.lock().await;
        acquired_clone.store(true, Ordering::SeqCst);
    });

    sleep(Duration::from_millis(30)).await;
    assert!(
        !acquired.load(Ordering::SeqCst),
        "second caller should still be blocked while first holds the lock"
    );
    drop(guard);

    waiter.await.expect("waiter task");
    assert!(
        acquired.load(Ordering::SeqCst),
        "second caller should acquire lock after first releases it"
    );
}

#[tokio::test]
async fn managed_default_image_ensure_is_concurrency_safe() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let body = b"ctx-test-managed-image".to_vec();
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server(body.clone()).await;
    let source = bundled_assets::ManagedArtifactSource {
        uri: url,
        sha256: digest,
    };
    let data_root = tmp.path().to_path_buf();

    let root_a = data_root.clone();
    let root_b = data_root.clone();
    let source_a = source.clone();
    let source_b = source.clone();
    let (res_a, res_b) = tokio::join!(
        tokio::spawn(async move {
            ensure_managed_default_container_image_tar_with_source(&root_a, &source_a, None, None)
                .await
        }),
        tokio::spawn(async move {
            ensure_managed_default_container_image_tar_with_source(&root_b, &source_b, None, None)
                .await
        })
    );
    server.abort();

    let path_a = res_a.expect("join a").expect("ensure a");
    let path_b = res_b.expect("join b").expect("ensure b");
    assert_eq!(path_a, path_b);
    let cached = tokio::fs::read(&path_a).await.expect("read cached tar");
    assert_eq!(cached, body);
}
