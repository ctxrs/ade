use super::*;

#[cfg(unix)]
pub(super) fn write_avf_linux_lifecycle_helper(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("ctx-avf-linux-helper-runtime-manager-test.sh");
    let sandbox_cli_path = dir.join("ctx-avf-linux-sandbox-cli-runtime-manager-test.sh");
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
    mapped="$arg"
    case "$arg" in
      /*) mapped="$(map_container_path "$container_name" "$arg")" ;;
    esac
    printf '%s\n' "$mapped"
  done
}}

should_short_circuit_network_policy_script() {{
  script="$1"
  case "$script" in
    *ctx-egress-proxy*|*iptables*|*pid_file=*|*CTX_CONTAINER_TERMINAL_USER*|*CTX_CONTAINER_TERMINAL_HOME*|*sudoers.d/*)
      return 0
      ;;
  esac
  return 1
}}

run_exec() {{
  pty=0
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
      *)
        break
        ;;
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
      if ( [ "$1" = "-c" ] || [ "$1" = "-lc" ] ) && should_short_circuit_network_policy_script "$2"; then
        exit 0
      fi
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
  case "$src" in
    */.)
      src_dir=$(dirname "$src")
      cp -R "$src_dir"/. "$host_dest"
      ;;
    *)
      cp -R "$src" "$host_dest"
      ;;
  esac
}}

subcmd="$1"
shift
case "$subcmd" in
  info)
    printf '{{}}\n'
    ;;
  image)
    image_cmd="$1"
    shift
    case "$image_cmd" in
      inspect)
        find "$images_root" -mindepth 1 -maxdepth 1 | grep -q .
        if [ $? -ne 0 ]; then
          exit 1
        fi
        printf '[]\n'
        ;;
      *)
        echo "unexpected sandbox CLI image command: $image_cmd $*" >&2
        exit 1
        ;;
    esac
    ;;
  load)
    if [ "$1" = "-i" ]; then
      shift 2
    fi
    image_key="default-image"
    : > "$images_root/$image_key"
    printf 'Loaded image: ctx-harness\n'
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
      rm)
        [ "$1" = "-f" ] && shift
        volume_name="$1"
        rm -rf "$volumes_root/$volume_name"
        ;;
      *)
        echo "unexpected sandbox CLI volume command: $volume_cmd $*" >&2
        exit 1
        ;;
    esac
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
        if [ "$1" = "--format" ] && [ "$2" = "{{{{.State.Running}}}}" ]; then
          container_name="$3"
          [ -d "$(container_dir "$container_name")" ] || exit 1
          state=$(cat "$(container_state_file "$container_name")" 2>/dev/null || printf 'false')
          printf '%s\n' "$state"
        elif [ $# -eq 1 ]; then
          container_name="$1"
          [ -d "$(container_dir "$container_name")" ] || exit 1
          printf '[{{}}]\n'
        else
          echo "unexpected sandbox CLI container inspect command: $*" >&2
          exit 1
        fi
        ;;
      *)
        echo "unexpected sandbox CLI container command: $container_cmd $*" >&2
        exit 1
        ;;
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
        printf '{{"Type":"bind","Source":"%s","Destination":"%s"}}' "$mount_src" "$mount_dst"
      fi
      first=0
    done < "$mounts_file"
    printf ']}}]\n'
    ;;
  start)
    container_name="$1"
    ensure_container_dir "$container_name"
    printf 'true' > "$(container_state_file "$container_name")"
    ;;
  rm)
    [ "$1" = "-f" ] && shift
    container_name="$1"
    rm -rf "$(container_dir "$container_name")"
    ;;
  run)
    container_name=""
    mounts_file_tmp="$vm_root/run-mounts.$$"
    : > "$mounts_file_tmp"
    while [ $# -gt 0 ]; do
      case "$1" in
        -d) shift ;;
        --name) container_name="$2"; shift 2 ;;
        --hostname) shift 2 ;;
        --userns=*) shift ;;
        --user) shift 2 ;;
        --network) shift 2 ;;
        --cap-add) shift 2 ;;
        --add-host) shift 2 ;;
        --mount)
          printf '%s\n' "$2" >> "$mounts_file_tmp"
          shift 2
          ;;
        *)
          break
          ;;
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
      printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"running","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","runtime_version":"%s","transition_status":"ready","last_start_outcome":"already_running","simulated":true,"notes":["state ready"]}}\n' "$vm_root" "$logs_root" "$state_path" "$log_path" "$runtime_version"
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
    printf '{{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","state":"running","vm_root":"%s","logs_root":"%s","state_path":"%s","log_path":"%s","runtime_root":"%s","rootfs_image":"%s","kernel_path":"%s","initrd_path":"%s","runtime_version":"%s","transition_status":"ready","last_start_outcome":"cold_boot","simulated":true,"notes":["launch ready"]}}\n' "$vm_root" "$logs_root" "$state_path" "$log_path" "$runtime_root" "$rootfs_image" "$kernel_path" "$initrd_path" "$runtime_version"
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
    if [ "$shared_command" = "sandbox-cli" ] || [ "$shared_command" = "/usr/local/bin/nerdctl" ]; then
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
        sandbox_cli_shim = sandbox_cli_path.display()
    );
    std::fs::write(&path, script).expect("write AVF lifecycle helper shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod AVF lifecycle helper shim");
    path
}

pub(super) fn write_ready_runtime_sandbox_cli_shim(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("sandbox-cli-ready-runtime-test.sh");
    let inner_path = crate::test_support::avf_linux_runtime_manager_test_sandbox_cli_path(dir);
    let script = format!(
        "#!/bin/sh\nexec \"{inner}\" \"{data_root}\" \"$@\"\n",
        inner = inner_path.display(),
        data_root = dir.display()
    );
    std::fs::write(&path, script).expect("write ready runtime sandbox CLI shim");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod ready runtime sandbox CLI shim");
    path
}
