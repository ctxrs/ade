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

#[cfg(unix)]
pub(crate) fn write_avf_linux_lifecycle_helper(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("ctx-avf-linux-helper-runtime-manager-test.sh");
    let sandbox_cli_path = avf_linux_runtime_manager_test_sandbox_cli_path(dir);
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let sandbox_cli_script = format!(
        r#"#!/bin/sh
data_root="$1"
shift
vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
containers_root="$vm_root/test-containers"
volumes_root="$vm_root/test-volumes"
images_root="$vm_root/test-images"
log_path="$vm_root/sandbox-cli-invocations.log"
mkdir -p "$containers_root" "$volumes_root" "$images_root" "$(dirname "$log_path")"
printf '%s\n' "$*" >> "$log_path"

container_dir() {{
  printf '%s' "$containers_root/$1"
}}

container_rootfs() {{
  printf '%s' "$(container_dir "$1")/rootfs"
}}

container_mounts_file() {{
  printf '%s' "$(container_dir "$1")/mounts"
}}

container_state_file() {{
  printf '%s' "$(container_dir "$1")/state"
}}

ensure_container_dir() {{
  mkdir -p "$(container_dir "$1")"
  mkdir -p "$(container_rootfs "$1")"
}}

map_container_path() {{
  container_name="$1"
  guest_path="$2"
  mounts_file="$(container_mounts_file "$container_name")"
  if [ ! -f "$mounts_file" ]; then
    printf '%s\n' "$guest_path"
    return 0
  fi
  while IFS='|' read -r mount_type mount_src mount_dst mount_mode; do
    [ -n "$mount_type" ] || continue
    host_src="$mount_src"
    if [ "$mount_type" = "volume" ]; then
      host_src="$volumes_root/$mount_src"
    fi
    case "$guest_path" in
      "$mount_dst")
        printf '%s\n' "$host_src"
        return 0
        ;;
      "$mount_dst"/*)
        rel=$(printf '%s' "$guest_path" | sed "s#^$mount_dst/##")
        printf '%s\n' "$host_src/$rel"
        return 0
        ;;
    esac
  done < "$mounts_file"
  printf '%s\n' "$guest_path"
}}

map_non_shell_args() {{
  container_name="$1"
  shift
  for arg in "$@"; do
    case "$arg" in
      /*)
        map_container_path "$container_name" "$arg"
        ;;
      *)
        printf '%s\n' "$arg"
        ;;
    esac
  done
}}

run_shell_script() {{
  container_name="$1"
  script="$2"
  shift 2
  translated_script="$script"
  while IFS= read -r path_arg; do
    translated_path=$(map_container_path "$container_name" "$path_arg")
    translated_script=$(printf '%s' "$translated_script" | sed "s#${{path_arg}}#${{translated_path}}#g")
  done <<'PATHS'
/ctx/ws
/tmp
PATHS
  exec /bin/sh -c "$translated_script" "$@"
}}

should_short_circuit_network_policy_script() {{
  script="$1"
  case "$script" in
    *ctx-egress-proxy*|*iptables*|*pid_file=*)
      return 0
      ;;
  esac
  return 1
}}

if [ "$1" = "info" ]; then
  printf '{{}}\\n'
  exit 0
fi

if [ "$1" = "image" ] && [ "$2" = "inspect" ]; then
  find "$images_root" -mindepth 1 -maxdepth 1 | grep -q .
  if [ $? -eq 0 ]; then
    printf '[{{}}]\n'
    exit 0
  fi
  exit 1
fi

if [ "$1" = "load" ] && [ "$2" = "-i" ]; then
  shift 2
  : > "$images_root/default-image"
  printf 'Loaded image: ctx-harness\n'
  exit 0
fi

if [ "$1" = "volume" ] && [ "$2" = "inspect" ]; then
  volume="$3"
  [ -d "$volumes_root/$volume" ] || exit 1
  printf '[{{}}]\n'
  exit 0
fi

if [ "$1" = "volume" ] && [ "$2" = "create" ]; then
  volume="$3"
  mkdir -p "$volumes_root/$volume"
  printf '%s\n' "$volume"
  exit 0
fi

if [ "$1" = "inspect" ]; then
  container="$2"
  mounts_file="$(container_mounts_file "$container")"
  if [ ! -f "$mounts_file" ]; then
    printf '[]\n'
    exit 0
  fi
  {{
    printf '[{{"Mounts":['
    first=1
    while IFS='|' read -r mount_type mount_src mount_dst mount_mode; do
      [ -n "$mount_type" ] || continue
      if [ $first -eq 0 ]; then
        printf ','
      fi
      first=0
      printf '{{"Type":"%s","Name":"%s","Destination":"%s"}}' \
        "$mount_type" "$mount_src" "$mount_dst"
    done < "$mounts_file"
    printf ']}}]\n'
  }}
  exit 0
fi

if [ "$1" = "container" ] && [ "$2" = "inspect" ] && [ $# -eq 3 ]; then
  container="$3"
  [ -d "$(container_dir "$container")" ] || exit 1
  printf '[{{}}]\n'
  exit 0
fi

if [ "$1" = "container" ] && [ "$2" = "inspect" ] && [ "$3" = "--format" ]; then
  container="$5"
  [ -d "$(container_dir "$container")" ] || exit 1
  state=$(cat "$(container_state_file "$container")" 2>/dev/null || printf 'false')
  printf '%s\n' "$state"
  exit 0
fi

if [ "$1" = "run" ]; then
  shift
  container=""
  workdir=""
  env_file=""
  env_args=""
  mounts=""
  shell_script=""
  image=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --name)
        container="$2"
        shift 2
        ;;
      --workdir)
        workdir="$2"
        shift 2
        ;;
      --env)
        env_args="$env_args
$2"
        shift 2
        ;;
      --mount)
        mounts="$mounts
$2"
        shift 2
        ;;
      --detach|-d)
        shift
        ;;
      --tty|-t|--interactive|-i|--rm)
        shift
        ;;
      --network|--cpus|--memory|--memory-reservation|--memory-swap|--user)
        shift 2
        ;;
      --)
        shift
        image="$1"
        shift
        if [ "$1" = "/bin/sh" ] && [ "$2" = "-lc" ]; then
          shell_script="$3"
        fi
        break
        ;;
      *)
        image="$1"
        shift
        if [ "$1" = "/bin/sh" ] && [ "$2" = "-lc" ]; then
          shell_script="$3"
        fi
        break
        ;;
    esac
  done
  ensure_container_dir "$container"
  printf 'true' > "$(container_state_file "$container")"
  if [ -n "$mounts" ]; then
    mounts_file="$(container_mounts_file "$container")"
    : > "$mounts_file"
    printf '%s\n' "$mounts" | while IFS= read -r mount; do
      [ -n "$mount" ] || continue
      type=$(printf '%s' "$mount" | sed -n 's/.*type=\([^,]*\).*/\1/p')
      src=$(printf '%s' "$mount" | sed -n 's/.*src=\([^,]*\).*/\1/p')
      dst=$(printf '%s' "$mount" | sed -n 's/.*dst=\([^,]*\).*/\1/p')
      mode=$(printf '%s' "$mount" | sed -n 's/.*readonly=\([^,]*\).*/\1/p')
      [ -n "$mode" ] || mode="false"
      printf '%s|%s|%s|%s\n' "$type" "$src" "$dst" "$mode" >> "$mounts_file"
      if [ "$type" = "volume" ]; then
        mkdir -p "$volumes_root/$src"
      fi
    done
  fi
  if [ -n "$shell_script" ]; then
    mkdir -p "$(map_container_path "$container" "$workdir")"
    (
      cd "$(map_container_path "$container" "$workdir")" || exit 1
      if [ -n "$env_args" ]; then
        while IFS= read -r kv; do
          [ -n "$kv" ] || continue
          export "$kv"
        done <<EOF
$env_args
EOF
      fi
      run_shell_script "$container" "$shell_script"
    )
  fi
  printf 'fake-container-id\n'
  exit 0
fi

if [ "$1" = "exec" ]; then
  shift
  container=""
  workdir=""
  env_args=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --workdir)
        workdir="$2"
        shift 2
        ;;
      --env)
        env_args="$env_args
$2"
        shift 2
        ;;
      --user)
        shift 2
        ;;
      --interactive|-i|--tty|-t)
        shift
        ;;
      *)
        container="$1"
        shift
        break
        ;;
    esac
  done
  [ -n "$container" ] || exit 1
  command="$1"
  shift
  if [ -n "$workdir" ]; then
    host_workdir=$(map_container_path "$container" "$workdir")
  else
    host_workdir="$(container_rootfs "$container")"
  fi
  mkdir -p "$host_workdir"
  case "$command" in
    sh|/bin/sh|bash|/bin/bash)
      if ( [ "$1" = "-c" ] || [ "$1" = "-lc" ] ) && should_short_circuit_network_policy_script "$2"; then
        exit 0
      fi
      ;;
  esac
  if [ "$command" = "/bin/sh" ] && [ "$1" = "-lc" ]; then
    script="$2"
    (
      cd "$host_workdir" || exit 1
      if [ -n "$env_args" ]; then
        while IFS= read -r kv; do
          [ -n "$kv" ] || continue
          export "$kv"
        done <<EOF
$env_args
EOF
      fi
      run_shell_script "$container" "$script"
    )
    exit $?
  fi
  translated_args=$(map_non_shell_args "$container" "$@")
  set -- $translated_args
  (
    cd "$host_workdir" || exit 1
    if [ -n "$env_args" ]; then
      while IFS= read -r kv; do
        [ -n "$kv" ] || continue
        export "$kv"
      done <<EOF
$env_args
EOF
    fi
    "$command" "$@"
  )
  exit $?
fi

if [ "$1" = "rm" ] && [ "$2" = "-f" ]; then
  container="$3"
  rm -rf "$(container_dir "$container")"
  exit 0
fi

echo "unexpected sandbox CLI invocation: $*" >&2
exit 1
"#
    );
    std::fs::write(&sandbox_cli_path, sandbox_cli_script).expect("write AVF sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF sandbox CLI shim");
    let script = format!(
        "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\nprobe)\n  printf '%s\\n' '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}}'\n  exit 0\n  ;;\nprepare-runtime-layout)\n  data_root=\"$1\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  mkdir -p \"$logs_root\"\n  printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"layout_status\":\"prepared\",\"notes\":[\"layout ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\"\n  exit 0\n  ;;\nshared-vm-state|workspace-vm-state)\n  data_root=\"$1\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  log_path=\"$logs_root/shared-vm.log\"\n  status_file=\"$vm_root/helper-status.txt\"\n  version_file=\"$vm_root/runtime-version.txt\"\n  state=$(cat \"$status_file\" 2>/dev/null || printf 'stopped')\n  runtime_version=$(cat \"$version_file\" 2>/dev/null || true)\n  if [ \"$state\" = \"running\" ]; then\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"runtime_version\":\"%s\",\"transition_status\":\"ready\",\"last_start_outcome\":\"already_running\",\"simulated\":true,\"notes\":[\"state ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\" \"$runtime_version\"\n  else\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"%s\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"simulated\":true,\"notes\":[\"state ready\"]}}\\n' \"$state\" \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\"\n  fi\n  exit 0\n  ;;\nstart-shared-vm|start-workspace-vm)\n  data_root=\"$1\"\n  runtime_root=\"$2\"\n  rootfs_image=\"$3\"\n  kernel_path=\"$4\"\n  initrd_path=\"$5\"\n  runtime_version=\"$6\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  log_path=\"$logs_root/shared-vm.log\"\n  mkdir -p \"$logs_root\"\n  printf 'running' > \"$vm_root/helper-status.txt\"\n  printf '%s' \"$runtime_version\" > \"$vm_root/runtime-version.txt\"\n  printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"runtime_root\":\"%s\",\"rootfs_image\":\"%s\",\"kernel_path\":\"%s\",\"initrd_path\":\"%s\",\"runtime_version\":\"%s\",\"transition_status\":\"ready\",\"last_start_outcome\":\"cold_boot\",\"simulated\":true,\"notes\":[\"launch ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\" \"$runtime_root\" \"$rootfs_image\" \"$kernel_path\" \"$initrd_path\" \"$runtime_version\"\n  exit 0\n  ;;\nshared-vm-exec)\n  data_root=\"\"\n  shared_command=\"\"\n  while [ $# -gt 0 ]; do\n    case \"$1\" in\n      --data-root) data_root=\"$2\"; shift 2 ;;\n      --command) shared_command=\"$2\"; shift 2 ;;\n      --cwd) shift 2 ;;\n      --user) shift 2 ;;\n      --env)\n        kv=\"$2\"\n        key=$(printf '%s' \"$kv\" | sed 's/=.*//')\n        value=$(printf '%s' \"$kv\" | sed 's/^[^=]*=//')\n        export \"$key=$value\"\n        shift 2\n        ;;\n      --) shift; break ;;\n      *) echo \"unexpected shared-vm-exec arg: $1\" >&2; exit 1 ;;\n    esac\n  done\n  if [ \"$shared_command\" = \"sandbox-cli\" ] || [ \"$shared_command\" = \"/usr/local/bin/nerdctl\" ]; then\n    exec \"{sandbox_cli_path}\" \"$data_root\" \"$@\"\n  fi\n  exec \"$shared_command\" \"$@\"\n  ;;\nesac\necho \"unexpected helper invocation: $cmd $*\" >&2\nexit 1\n",
        host_os = std::env::consts::OS,
        host_arch = std::env::consts::ARCH,
        sandbox_cli_path = sandbox_cli_path.display(),
    );
    std::fs::write(&path, script).expect("write AVF Linux helper shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF Linux helper shim");
    path
}

pub(crate) fn avf_linux_runtime_manager_test_sandbox_cli_path(dir: &Path) -> PathBuf {
    dir.join("ctx-avf-linux-sandbox-cli-runtime-manager-test.sh")
}
