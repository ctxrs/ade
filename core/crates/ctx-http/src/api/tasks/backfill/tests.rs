use super::*;
use std::collections::HashMap;
use std::process::Command as StdCommand;
use std::sync::Arc;

use ctx_core::models::VcsKind;
use ctx_store::{Store, StoreManager};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

struct EnvVarGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            std::env::set_var(self.key, prev);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

async fn spawn_static_http_server_with_suffix(
    body: Vec<u8>,
    suffix: &str,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind static server");
    let addr = listener.local_addr().expect("local addr");
    let suffix = suffix.to_string();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(parts) => parts,
                Err(_) => break,
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0_u8; 1024];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{addr}/{suffix}"), task)
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

#[cfg(unix)]
fn write_avf_linux_lifecycle_helper(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("ctx-avf-linux-helper-backfill-test.sh");
    let sandbox_cli_path = dir.join("ctx-avf-linux-sandbox-cli-backfill-test.sh");
    let host_os = std::env::consts::OS;
    let host_arch = std::env::consts::ARCH;
    let sandbox_cli_script = format!(
        r#"#!/bin/sh
data_root="${{HOME%/sandbox/home}}"
if [ -z "$data_root" ] || [ "$data_root" = "$HOME" ]; then
  echo "sandbox CLI test shim expected HOME under <data-root>/sandbox/home" >&2
  exit 1
fi
vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
containers_root="$vm_root/test-containers"
volumes_root="$vm_root/test-volumes"
log_path="$vm_root/sandbox-cli-invocations.log"
mkdir -p "$containers_root" "$volumes_root" "$(dirname "$log_path")"
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
    mapped="$arg"
    case "$arg" in
      /*) mapped="$(map_container_path "$container_name" "$arg")" ;;
    esac
    printf '%s\n' "$mapped"
  done
}}

run_exec() {{
  workdir="/"
  while [ $# -gt 0 ]; do
    case "$1" in
      --interactive|--tty) shift ;;
      --user) shift 2 ;;
      --env)
        kv="$2"
        key=$(printf '%s' "$kv" | sed 's/=.*//')
        value=$(printf '%s' "$kv" | sed 's/^[^=]*=//')
        export "$key=$value"
        shift 2
        ;;
      --workdir) workdir="$2"; shift 2 ;;
      *) break ;;
    esac
  done
  container_name="$1"
  shift
  command_name="$1"
  shift
  host_cwd="$(map_container_path "$container_name" "$workdir")"
  mkdir -p "$host_cwd"
  case "$command_name" in
    sh|/bin/sh|bash|/bin/bash)
      (cd "$host_cwd" && exec "$command_name" "$@")
      ;;
    *)
      mapped_lines="$(map_non_shell_args "$container_name" "$@")"
      set --
      while IFS= read -r arg; do
        set -- "$@" "$arg"
      done <<EOF
$mapped_lines
EOF
      (cd "$host_cwd" && exec "$command_name" "$@")
      ;;
  esac
}}

run_cp() {{
  src="$1"
  dest_spec="$2"
  container_name=$(printf '%s' "$dest_spec" | sed 's/:.*$//')
  guest_dest=$(printf '%s' "$dest_spec" | sed 's/^[^:]*://')
  host_dest="$(map_container_path "$container_name" "$guest_dest")"
  mkdir -p "$host_dest"
  printf 'cp %s -> %s\n' "$src" "$host_dest" >> "$log_path"
  case "$src" in
    */.)
      src_dir=$(dirname "$src")
      cp -R "$src_dir"/. "$host_dest"
      ;;
    *)
      cp -R "$src" "$host_dest"
      ;;
  esac
  find "$host_dest" -maxdepth 2 -mindepth 1 -print >> "$log_path" 2>/dev/null || true
}}

subcmd="$1"
shift
case "$subcmd" in
  info)
    printf '{{}}\n'
    ;;
  image)
    [ "$1" = "exists" ] || exit 1
    exit 0
    ;;
  volume)
    volume_cmd="$1"
    shift
    case "$volume_cmd" in
      inspect)
        volume_name="$1"
        [ -d "$volumes_root/$volume_name" ] || exit 1
        printf '[]\n'
        ;;
      create)
        volume_name="$1"
        mkdir -p "$volumes_root/$volume_name"
        printf '%s\n' "$volume_name"
        ;;
      *) exit 1 ;;
    esac
    ;;
  inspect)
    container_name="$1"
    mounts_file="$(container_mounts_file "$container_name")"
    [ -f "$mounts_file" ] || exit 1
    printf '['
    printf '{{"Mounts":['
    first=1
    while IFS='|' read -r mount_type mount_src mount_dst mount_mode; do
      [ -n "$mount_type" ] || continue
      if [ "$first" -eq 0 ]; then
        printf ','
      fi
      if [ "$mount_type" = "volume" ]; then
        printf '{{"Type":"volume","Name":"%s","Destination":"%s"}}' "$mount_src" "$mount_dst"
      else
        printf '{{"Type":"bind","Destination":"%s"}}' "$mount_dst"
      fi
      first=0
    done < "$mounts_file"
    printf ']}}]\n'
    ;;
  container)
    container_cmd="$1"
    shift
    case "$container_cmd" in
      exists)
        container_name="$1"
        [ -d "$(container_dir "$container_name")" ]
        ;;
      inspect)
        [ "$1" = "--format" ] || exit 1
        container_name="$3"
        [ -d "$(container_dir "$container_name")" ] || exit 1
        state=$(cat "$(container_state_file "$container_name")" 2>/dev/null || printf 'false')
        printf '%s\n' "$state"
        ;;
      *) exit 1 ;;
    esac
    ;;
  run)
    container_name=""
    mounts_file_tmp="$vm_root/run-mounts.$$"
    : > "$mounts_file_tmp"
    while [ $# -gt 0 ]; do
      case "$1" in
        -d) shift ;;
        --name) container_name="$2"; shift 2 ;;
        --userns=*|--network|--cap-add|--add-host|--user) shift 2 ;;
        --mount)
          printf '%s\n' "$2" >> "$mounts_file_tmp"
          shift 2
          ;;
        *) break ;;
      esac
    done
    [ -n "$container_name" ] || exit 1
    ensure_container_dir "$container_name"
    mounts_file="$(container_mounts_file "$container_name")"
    : > "$mounts_file"
    while IFS= read -r mount_entry; do
      [ -n "$mount_entry" ] || continue
      mount_type=$(printf '%s' "$mount_entry" | tr ',' '\n' | awk -F= '$1=="type"{{print $2}}')
      mount_src=$(printf '%s' "$mount_entry" | tr ',' '\n' | awk -F= '$1=="src"{{print $2}}')
      mount_dst=$(printf '%s' "$mount_entry" | tr ',' '\n' | awk -F= '$1=="dst"{{print $2}}')
      mount_mode=$(printf '%s' "$mount_entry" | tr ',' '\n' | awk 'NF==1{{print $1}}')
      [ -n "$mount_type" ] || continue
      printf '%s|%s|%s|%s\n' "$mount_type" "$mount_src" "$mount_dst" "$mount_mode" >> "$mounts_file"
      if [ "$mount_type" = "volume" ]; then
        mkdir -p "$volumes_root/$mount_src"
      elif [ "$mount_type" = "bind" ]; then
        mkdir -p "$mount_src"
      fi
    done < "$mounts_file_tmp"
    rm -f "$mounts_file_tmp"
    printf 'true' > "$(container_state_file "$container_name")"
    printf '%s\n' "$container_name"
    ;;
  exec)
    run_exec "$@"
    ;;
  cp)
    run_cp "$1" "$2"
    ;;
  *)
    echo "unexpected sandbox CLI command: $subcmd $*" >&2
    exit 1
    ;;
esac
"#
    );
    std::fs::write(&sandbox_cli_path, sandbox_cli_script).expect("write AVF sandbox helper shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF sandbox helper shim");
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
    state=$(cat "$status_file" 2>/dev/null || printf 'stopped')
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
  prepare-guest-worktree)
    data_root="$1"
    workspace_id="$2"
    worktree_id="$3"
    host_workspace_root="$4"
    branch_name="$6"
    vm_root="$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared"
    host_shadow_root="$vm_root/worktrees/$workspace_id/$worktree_id/shadow-root"
    metadata_path="$vm_root/worktrees/$workspace_id/$worktree_id/worktree.json"
    guest_root="/ctx/ws/worktrees/$worktree_id"
    mkdir -p "$host_shadow_root"
    if [ ! -f "$metadata_path" ]; then
      mkdir -p "$(dirname "$metadata_path")"
      cp -R "$host_workspace_root"/. "$host_shadow_root"
      printf '{{"workspace_id":"%s","worktree_id":"%s"}}\n' "$workspace_id" "$worktree_id" > "$metadata_path"
      status="prepared"
      note="$branch_name"
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
  shared-vm-exec)
    data_root=""
    shared_command=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --data-root) data_root="$2"; shift 2 ;;
        --cwd) shift 2 ;;
        --command) shared_command="$2"; shift 2 ;;
        --user) shift 2 ;;
        --pty) shift ;;
        --env)
          kv="$2"
          key=$(printf '%s' "$kv" | sed 's/=.*//')
          value=$(printf '%s' "$kv" | sed 's/^[^=]*=//')
          export "$key=$value"
          shift 2
          ;;
        --) shift; break ;;
        *) echo "unexpected shared-vm-exec arg: $1" >&2; exit 1 ;;
      esac
    done
    if [ "$shared_command" = "sandbox-cli" ] || [ "$shared_command" = "nerdctl" ]; then
      exec "{sandbox_cli_shim}" "$data_root" "$@"
    fi
    exec "$shared_command" "$@"
    ;;
  *)
    echo "unexpected helper invocation: $cmd $*" >&2
    exit 1
    ;;
esac
"#,
        sandbox_cli_shim = sandbox_cli_path.display(),
    );
    std::fs::write(&path, script).expect("write AVF lifecycle helper shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF lifecycle helper shim");
    path
}

fn git(args: &[&str], cwd: &Path) {
    let status = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(
        status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
}

fn git_output(args: &[&str], cwd: &Path) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git output");
    assert!(
        output.status.success(),
        "git command failed: git {}",
        args.join(" ")
    );
    String::from_utf8(output.stdout)
        .expect("utf8 git output")
        .trim()
        .to_string()
}

async fn save_test_execution_settings(data_root: &Path) {
    let db_path = data_root.join("db").join("db.sqlite");
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create settings db dir");
    }
    let store = Store::open_sqlite(&db_path, Some(1))
        .await
        .expect("open settings store");
    crate::settings::save_settings(
        &store,
        &crate::settings::Settings {
            execution: Some(crate::settings::ExecutionSettings {
                mode: ExecutionMode::Host,
                container: crate::settings::ContainerExecutionSettings {
                    runtime: crate::settings::ContainerRuntimeKind::NativeContainer,
                    mount_mode: crate::settings::ContainerMountMode::DiskIsolated,
                    network_mode: crate::settings::ContainerNetworkMode::All,
                    allowlist: Vec::new(),
                    image: None,
                    machine: crate::settings::ContainerMachineSettings::default(),
                },
            }),
            ..crate::settings::Settings::default()
        },
    )
    .await
    .expect("save test settings");
    store.close().await;
}

fn init_git_workspace(root: &Path) -> String {
    git(&["init", "-b", "main"], root);
    git(&["config", "user.email", "ctx@example.com"], root);
    git(&["config", "user.name", "Ctx Test"], root);
    std::fs::write(root.join("README.md"), "hello\n").expect("write readme");
    git(&["add", "README.md"], root);
    git(&["commit", "-m", "initial"], root);
    git_output(&["rev-parse", "HEAD"], root)
}

async fn test_state(data_root: &Path) -> Arc<AppState> {
    Arc::new(AppState::new(
        data_root.to_path_buf(),
        StoreManager::open(data_root).await.expect("open stores"),
        HashMap::new(),
        "http://127.0.0.1:4310".to_string(),
        None,
    ))
}

#[cfg(unix)]
#[tokio::test]
async fn lookup_workspace_store_backfills_legacy_sandbox_worktree_on_cold_open() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    save_test_execution_settings(temp.path()).await;

    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit_sha = init_git_workspace(&repo_root);

    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let workspace_db_path = temp
        .path()
        .join("db")
        .join("workspaces")
        .join(workspace.id.0.to_string())
        .join("db.sqlite");
    if let Some(parent) = workspace_db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create workspace db dir");
    }
    let store = Store::open_sqlite(&workspace_db_path, Some(1))
        .await
        .expect("open workspace store");
    store
        .upsert_workspace(&workspace)
        .await
        .expect("seed workspace into workspace store");

    let legacy_root = temp.path().join("legacy-shadow");
    let task_id = ctx_core::ids::TaskId::new();
    let worktree_id = ctx_core::ids::WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task_id.0, worktree_id.0);
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            legacy_root.to_string_lossy().as_ref(),
            &base_commit_sha,
        ],
        &repo_root,
    );

    let task = store
        .create_task_with_id(workspace.id, task_id, "task".to_string(), None)
        .await
        .expect("create task");
    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: legacy_root.to_string_lossy().to_string(),
        base_commit_sha: base_commit_sha.clone(),
        git_branch: Some(branch_name.clone()),
        vcs_kind: Some(VcsKind::Git),
        base_revision: Some(base_commit_sha.clone()),
        vcs_ref: Some(branch_name.clone()),
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
    };
    let worktree = store
        .insert_worktree(worktree)
        .await
        .expect("create legacy worktree");
    store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create sandbox session");

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
    let _runtime_guard =
        crate::workspace_runtime::override_managed_avf_linux_runtime_source_for_test(
            crate::bundled_assets::ManagedRuntimeSource {
                uri: archive_url,
                sha256: hex::encode(Sha256::digest(&archive_bytes)),
                version: "ubuntu-minimal-test".to_string(),
                bin: "rootfs.img".to_string(),
                helpers: [
                    (
                        "kernel".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: kernel_url,
                            sha256: hex::encode(Sha256::digest(&kernel_bytes)),
                        },
                    ),
                    (
                        "initrd".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: initrd_url,
                            sha256: hex::encode(Sha256::digest(&initrd_bytes)),
                        },
                    ),
                    (
                        "guest-agent".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: guest_agent_url,
                            sha256: hex::encode(Sha256::digest(&guest_agent_bytes)),
                        },
                    ),
                    (
                        "egress-proxy".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: egress_proxy_url,
                            sha256: hex::encode(Sha256::digest(&egress_proxy_bytes)),
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
        );
    let _runtime_servers = [
        archive_server,
        kernel_server,
        initrd_server,
        guest_agent_server,
        egress_proxy_server,
    ];

    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = temp
        .path()
        .join("ctx-avf-linux-sandbox-cli-backfill-test.sh");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _avf_helper = EnvVarGuard::set("CTX_AVF_LINUX_HELPER_PATH", &helper_path.to_string_lossy());

    store.close().await;
    let lookup = state.lookup_workspace_store(workspace.id).await;
    let sandbox_log = temp
        .path()
        .join("managed")
        .join("vms")
        .join("avf-linux")
        .join(std::env::consts::OS)
        .join(std::env::consts::ARCH)
        .join("shared")
        .join("sandbox-cli-invocations.log");
    let store = match lookup {
        crate::daemon::StoreLookup::Found(store) => store,
        crate::daemon::StoreLookup::Missing => {
            panic!("expected found workspace store after backfill, got Missing")
        }
        crate::daemon::StoreLookup::Deleting => {
            panic!("expected found workspace store after backfill, got Deleting")
        }
        crate::daemon::StoreLookup::Unavailable(err) => {
            let log = std::fs::read_to_string(&sandbox_log).unwrap_or_default();
            panic!(
                "expected found workspace store after backfill, got error: {err:#}\nshim log:\n{log}"
            )
        }
    };

    let repaired = store
        .get_worktree(worktree.id)
        .await
        .expect("load repaired worktree")
        .expect("repaired worktree");
    let expected_root = managed_worktree_path(temp.path(), workspace.id, worktree.id);
    assert_eq!(PathBuf::from(&repaired.root_path), expected_root);
    assert!(
        expected_root.exists(),
        "canonical managed worktree should exist"
    );
    assert!(
        !legacy_root.exists(),
        "legacy shadow worktree should have been moved into canonical root"
    );

    let binding = store
        .get_sandbox_binding(worktree.id)
        .await
        .expect("load binding")
        .expect("backfilled binding");
    assert_eq!(binding.worktree_id, worktree.id);
    assert_eq!(
        binding.live_worktree_root,
        crate::disk_isolated::container_worktree_root(worktree.id)
            .to_string_lossy()
            .to_string()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn lookup_workspace_store_backfills_legacy_avf_shadow_worktree_without_sandbox_sessions() {
    let _serial = crate::test_support::sandbox_cli_env_test_lock()
        .lock()
        .await;
    let temp = tempfile::tempdir().expect("tempdir");
    save_test_execution_settings(temp.path()).await;

    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit_sha = init_git_workspace(&repo_root);

    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let workspace_db_path = temp
        .path()
        .join("db")
        .join("workspaces")
        .join(workspace.id.0.to_string())
        .join("db.sqlite");
    if let Some(parent) = workspace_db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create workspace db dir");
    }
    let store = Store::open_sqlite(&workspace_db_path, Some(1))
        .await
        .expect("open workspace store");
    store
        .upsert_workspace(&workspace)
        .await
        .expect("seed workspace into workspace store");

    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    let worktree_id = ctx_core::ids::WorktreeId::new();
    let branch_name = format!("ctx/{}/{}", task.id.0, worktree_id.0);
    let legacy_shadow_root = temp
        .path()
        .join("managed")
        .join("vms")
        .join("avf-linux")
        .join(std::env::consts::OS)
        .join(std::env::consts::ARCH)
        .join("shared")
        .join("worktrees")
        .join(workspace.id.0.to_string())
        .join(worktree_id.0.to_string())
        .join("shadow-root");
    if let Some(parent) = legacy_shadow_root.parent() {
        std::fs::create_dir_all(parent).expect("create legacy shadow parent");
    }
    git(
        &[
            "worktree",
            "add",
            "-b",
            &branch_name,
            legacy_shadow_root.to_string_lossy().as_ref(),
            &base_commit_sha,
        ],
        &repo_root,
    );

    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: legacy_shadow_root.to_string_lossy().to_string(),
            base_commit_sha: base_commit_sha.clone(),
            git_branch: Some(branch_name.clone()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit_sha.clone()),
            vcs_ref: Some(branch_name),
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
        })
        .await
        .expect("insert legacy shadow worktree");

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
    let _runtime_guard =
        crate::workspace_runtime::override_managed_avf_linux_runtime_source_for_test(
            crate::bundled_assets::ManagedRuntimeSource {
                uri: archive_url,
                sha256: hex::encode(Sha256::digest(&archive_bytes)),
                version: "ubuntu-minimal-test".to_string(),
                bin: "rootfs.img".to_string(),
                helpers: [
                    (
                        "kernel".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: kernel_url,
                            sha256: hex::encode(Sha256::digest(&kernel_bytes)),
                        },
                    ),
                    (
                        "initrd".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: initrd_url,
                            sha256: hex::encode(Sha256::digest(&initrd_bytes)),
                        },
                    ),
                    (
                        "guest-agent".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: guest_agent_url,
                            sha256: hex::encode(Sha256::digest(&guest_agent_bytes)),
                        },
                    ),
                    (
                        "egress-proxy".to_string(),
                        crate::bundled_assets::ManagedArtifactSource {
                            uri: egress_proxy_url,
                            sha256: hex::encode(Sha256::digest(&egress_proxy_bytes)),
                        },
                    ),
                ]
                .into_iter()
                .collect(),
            },
        );
    let _runtime_servers = [
        archive_server,
        kernel_server,
        initrd_server,
        guest_agent_server,
        egress_proxy_server,
    ];

    let helper_path = write_avf_linux_lifecycle_helper(temp.path());
    let sandbox_cli_path = temp
        .path()
        .join("ctx-avf-linux-sandbox-cli-backfill-test.sh");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _avf_helper = EnvVarGuard::set("CTX_AVF_LINUX_HELPER_PATH", &helper_path.to_string_lossy());

    store.close().await;
    let lookup = state.lookup_workspace_store(workspace.id).await;
    let store = match lookup {
        crate::daemon::StoreLookup::Found(store) => store,
        _ => panic!("expected found workspace store after AVF shadow backfill"),
    };
    let repaired = store
        .get_worktree(worktree.id)
        .await
        .expect("load repaired worktree")
        .expect("repaired worktree");
    let expected_root = managed_worktree_path(temp.path(), workspace.id, worktree.id);
    assert_eq!(PathBuf::from(&repaired.root_path), expected_root);
    assert!(
        !legacy_shadow_root.exists(),
        "legacy AVF shadow worktree should be removed after canonical reattach"
    );
    assert!(
        store
            .get_sandbox_binding(worktree.id)
            .await
            .expect("load binding")
            .is_some(),
        "legacy AVF shadow worktree should be backfilled into a sandbox binding"
    );
}

#[tokio::test]
async fn lookup_workspace_store_repairs_session_metadata_for_bound_sandbox_worktree() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace("ws".to_string(), "/tmp/ws".to_string(), VcsKind::Git)
        .await
        .expect("create workspace");
    let store = state
        .store_for_workspace(workspace.id)
        .await
        .expect("workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    let worktree_id = ctx_core::ids::WorktreeId::new();
    let worktree = store
        .insert_worktree(Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: managed_worktree_path(temp.path(), workspace.id, worktree_id)
                .to_string_lossy()
                .to_string(),
            base_commit_sha: "deadbeef".to_string(),
            git_branch: Some(format!("ctx/{}/{}", task.id.0, worktree_id.0)),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some("deadbeef".to_string()),
            vcs_ref: Some(format!("ctx/{}/{}", task.id.0, worktree_id.0)),
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
        })
        .await
        .expect("insert worktree");
    let session = store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create host-labeled session");
    store
        .upsert_sandbox_binding(ctx_core::models::SandboxBinding {
            worktree_id: worktree.id,
            workspace_id: workspace.id,
            runtime_family: ctx_core::models::SandboxRuntimeFamily::NativeContainer,
            profile: ctx_core::models::SandboxProfile::Standard,
            live_workspace_root: "/ctx/ws".to_string(),
            live_worktree_root: crate::disk_isolated::container_worktree_root(worktree.id)
                .to_string_lossy()
                .to_string(),
            execution_settings_json: Some(
                serde_json::to_string(&crate::settings::ExecutionSettings {
                    mode: crate::settings::ExecutionMode::Sandbox,
                    ..crate::settings::ExecutionSettings::default()
                })
                .expect("serialize settings"),
            ),
            container_name: Some(format!("ctx-harness-{}", workspace.id.0)),
            host_projection_root: None,
            created_at: Utc::now(),
        })
        .await
        .expect("upsert binding");

    let reopened = state.lookup_workspace_store(workspace.id).await;
    let store = match reopened {
        crate::daemon::StoreLookup::Found(store) => store,
        _ => panic!("expected found workspace store after binding reconcile"),
    };
    let repaired = store
        .get_session(session.id)
        .await
        .expect("load repaired session")
        .expect("repaired session");
    assert_eq!(
        repaired.execution_environment,
        ExecutionEnvironment::Sandbox,
        "bound sandbox worktree sessions should be repaired to sandbox metadata on cold open"
    );
}

#[tokio::test]
async fn backfill_fails_explicitly_for_mixed_legacy_session_modes() {
    let temp = tempfile::tempdir().expect("tempdir");
    save_test_execution_settings(temp.path()).await;

    let repo_root = temp.path().join("repo");
    std::fs::create_dir_all(&repo_root).expect("create repo root");
    let base_commit_sha = init_git_workspace(&repo_root);

    let state = test_state(temp.path()).await;
    let workspace = state
        .global_store()
        .create_workspace(
            "ws".to_string(),
            repo_root.to_string_lossy().to_string(),
            VcsKind::Git,
        )
        .await
        .expect("create workspace");
    let workspace_db_path = temp
        .path()
        .join("db")
        .join("workspaces")
        .join(workspace.id.0.to_string())
        .join("db.sqlite");
    if let Some(parent) = workspace_db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create workspace db dir");
    }
    let store = Store::open_sqlite(&workspace_db_path, Some(1))
        .await
        .expect("open workspace store");
    store
        .upsert_workspace(&workspace)
        .await
        .expect("seed workspace into workspace store");
    let task = store
        .create_task(workspace.id, "task".to_string(), None)
        .await
        .expect("create task");
    let worktree_id = ctx_core::ids::WorktreeId::new();
    let worktree = Worktree {
        id: worktree_id,
        workspace_id: workspace.id,
        root_path: legacy_guest_worktree_root(&Worktree {
            id: worktree_id,
            workspace_id: workspace.id,
            root_path: String::new(),
            base_commit_sha: base_commit_sha.clone(),
            git_branch: Some("ctx/test/mixed".to_string()),
            vcs_kind: Some(VcsKind::Git),
            base_revision: Some(base_commit_sha.clone()),
            vcs_ref: Some("ctx/test/mixed".to_string()),
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
        })
        .to_string_lossy()
        .to_string(),
        base_commit_sha: base_commit_sha.clone(),
        git_branch: Some("ctx/test/mixed".to_string()),
        vcs_kind: Some(VcsKind::Git),
        base_revision: Some(base_commit_sha.clone()),
        vcs_ref: Some("ctx/test/mixed".to_string()),
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
    };
    let worktree = store
        .insert_worktree(worktree)
        .await
        .expect("create worktree");
    store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Host,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create host session");
    store
        .create_session(
            task.id,
            workspace.id,
            worktree.id,
            ExecutionEnvironment::Sandbox,
            "fake".to_string(),
            "model".to_string(),
            "implementer".to_string(),
            None,
            None,
            None,
        )
        .await
        .expect("create sandbox session");

    let err = ensure_workspace_sandbox_bindings_backfilled(&state, &workspace, &store)
        .await
        .expect_err("mixed legacy worktree should fail explicitly");
    let message = format!("{err:#}");
    assert!(message.contains("mixed host and sandbox sessions"));
}
