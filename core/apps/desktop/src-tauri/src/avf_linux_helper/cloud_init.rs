use super::*;

pub(super) fn wrap_cloud_init_base64(bytes: &[u8]) -> String {
    let encoded = BASE64_STANDARD.encode(bytes);
    let mut wrapped = String::new();
    for chunk in encoded.as_bytes().chunks(76) {
        if !wrapped.is_empty() {
            wrapped.push('\n');
        }
        wrapped.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    wrapped
}

pub(super) fn indent_cloud_init_block(content: &str, spaces: usize) -> String {
    let prefix = " ".repeat(spaces);
    content
        .lines()
        .map(|line| format!("{prefix}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn render_shared_vm_guest_agent_launcher_script(
    ready_marker_path: &Path,
    failure_marker_path: &Path,
    guest_agent_log_path: &Path,
) -> String {
    let ready_marker = shell_escape_single_quotes(&ready_marker_path.display().to_string());
    let failure_marker = shell_escape_single_quotes(&failure_marker_path.display().to_string());
    let guest_agent_log = shell_escape_single_quotes(&guest_agent_log_path.display().to_string());
    format!(
        "#!/bin/sh\nset -eu\nready_marker='{ready_marker}'\nfailure_marker='{failure_marker}'\nlog_path='{guest_agent_log}'\nagent_bin='/usr/local/bin/ctx-avf-linux-guest-agent'\nready_timeout_sec={ready_timeout_sec}\nlog() {{\n  message=\"$1\"\n  printf '%s\\n' \"$message\" >> \"$log_path\"\n  printf '%s\\n' \"$message\" >/dev/hvc0\n}}\nfail() {{\n  message=\"$1\"\n  rm -f \"$ready_marker\"\n  printf '%s\\n' \"$message\" > \"$failure_marker\"\n  log \"$message\"\n  exit 1\n}}\nmkdir -p \"$(dirname \"$ready_marker\")\" \"$(dirname \"$failure_marker\")\" \"$(dirname \"$log_path\")\"\n: > \"$log_path\"\nrm -f \"$ready_marker\" \"$failure_marker\"\nlog \"[ctx-avf-linux] guest-agent launcher starting\"\nif [ ! -x \"$agent_bin\" ]; then\n  fail \"[ctx-avf-linux] guest-agent binary missing or not executable: $agent_bin\"\nfi\nprobe_path=\"${{ready_marker}}.probe\"\nif ! touch \"$probe_path\" >/dev/null 2>&1; then\n  fail \"[ctx-avf-linux] guest-agent ready-marker parent is not writable: $(dirname \"$ready_marker\")\"\nfi\nrm -f \"$probe_path\"\nif [ ! -e /dev/vsock ]; then\n  log \"[ctx-avf-linux] /dev/vsock is not present before guest-agent exec\"\nfi\nCTX_AVF_GUEST_CONTROL_READY_MARKER=\"$ready_marker\" \"$agent_bin\" >> \"$log_path\" 2>&1 &\nagent_pid=$!\nlog \"[ctx-avf-linux] guest-agent started as pid $agent_pid; waiting for ready marker\"\nremaining=\"$ready_timeout_sec\"\nwhile [ \"$remaining\" -gt 0 ]; do\n  if [ -f \"$ready_marker\" ]; then\n    log \"[ctx-avf-linux] guest-agent published ready marker\"\n    wait \"$agent_pid\"\n    status=$?\n    fail \"[ctx-avf-linux] guest-agent exited after ready with status $status\"\n  fi\n  if ! kill -0 \"$agent_pid\" 2>/dev/null; then\n    status=1\n    wait \"$agent_pid\" || status=$?\n    fail \"[ctx-avf-linux] guest-agent exited before ready with status $status\"\n  fi\n  sleep 1\n  remaining=$((remaining - 1))\ndone\nkill \"$agent_pid\" >/dev/null 2>&1 || true\nwait \"$agent_pid\" >/dev/null 2>&1 || true\nfail \"[ctx-avf-linux] guest-agent did not publish ready marker within {ready_timeout_sec}s\"\n",
        ready_timeout_sec = SHARED_VM_GUEST_AGENT_READY_TIMEOUT_SECONDS,
    )
}

pub(super) fn render_shared_vm_guest_agent_service(
    ready_marker_path: &Path,
    failure_marker_path: &Path,
    guest_agent_log_path: &Path,
) -> String {
    let prepare_script = shell_escape_single_quotes(&format!(
        "rm -f '{ready_marker}' && echo \"[ctx-avf-linux] starting guest-agent\" >/dev/hvc0 && echo \"[ctx-avf-linux] ensuring vsock kernel modules are loaded\" >/dev/hvc0 && /usr/sbin/modprobe vsock >/dev/hvc0 2>&1 && /usr/sbin/modprobe vmw_vsock_virtio_transport_common >/dev/hvc0 2>&1 && /usr/sbin/modprobe vmw_vsock_virtio_transport >/dev/hvc0 2>&1",
        ready_marker = ready_marker_path.display(),
    ));
    let launcher_script = render_shared_vm_guest_agent_launcher_script(
        ready_marker_path,
        failure_marker_path,
        guest_agent_log_path,
    );
    format!(
        "[Unit]\nDescription=ctx AVF Linux Guest Agent\nAfter={data_disk_service} {host_data_service}\nRequires={data_disk_service} {host_data_service}\n\n[Service]\nType=simple\nEnvironment=RUST_BACKTRACE=1\nExecStartPre=/bin/sh -lc '{prepare_script}'\nExecStart={launcher_path}\nStandardOutput=journal+console\nStandardError=journal+console\nRestart=no\n\n[Install]\nWantedBy=multi-user.target\n# {guest_agent_service}\n# guest-agent-launcher\n{launcher_script_comment}",
        data_disk_service = SHARED_VM_DATA_DISK_SERVICE_NAME,
        host_data_service = SHARED_VM_HOST_DATA_SERVICE_NAME,
        prepare_script = prepare_script,
        launcher_path = SHARED_VM_GUEST_AGENT_LAUNCHER_PATH,
        guest_agent_service = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        launcher_script_comment = launcher_script
            .lines()
            .map(|line| format!("# {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub(super) fn render_shared_vm_host_data_mount_service(host_data_root: &Path) -> String {
    let mount_root = host_data_root.display().to_string();
    let escaped_mount_root = shell_escape_single_quotes(&mount_root);
    let escaped_tag = shell_escape_single_quotes(SHARED_VM_DATA_ROOT_SHARE_TAG);
    format!(
        "[Unit]\nDescription=ctx AVF Host Data Mount\nDefaultDependencies=no\nAfter=local-fs.target\nBefore={guest_agent_service}\n\n[Service]\nType=oneshot\nRemainAfterExit=yes\nExecStart=/bin/sh -lc 'mkdir -p '\\''{mount_root}'\\'' && mountpoint -q '\\''{mount_root}'\\'' || mount -t virtiofs '\\''{tag}'\\'' '\\''{mount_root}'\\''' \nExecStop=/bin/sh -lc 'mountpoint -q '\\''{mount_root}'\\'' && umount '\\''{mount_root}'\\'' || true'\n\n[Install]\nWantedBy=multi-user.target\n# {service_name}\n",
        guest_agent_service = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        mount_root = escaped_mount_root,
        tag = escaped_tag,
        service_name = SHARED_VM_HOST_DATA_SERVICE_NAME,
    )
}

pub(super) fn render_shared_vm_data_disk_script() -> String {
    format!(
        "#!/bin/sh\nset -eu\nmount_root='/ctx'\ndata_label='{data_label}'\nmarker_name='.ctx-avf-data-disk-ready'\nroot_device=\"$(findmnt -n -o SOURCE /)\"\nif [ -z \"$root_device\" ]; then\n  echo \"[ctx-avf-linux] could not determine root device\" >/dev/hvc0\n  exit 1\nfi\nroot_device=\"$(readlink -f \"$root_device\" 2>/dev/null || printf '%s' \"$root_device\")\"\nroot_disk=\"$(lsblk -nro PKNAME \"$root_device\" | head -n1)\"\nif [ -z \"$root_disk\" ]; then\n  echo \"[ctx-avf-linux] could not resolve parent disk for $root_device\" >/dev/hvc0\n  exit 1\nfi\ndata_device=\"$(lsblk -dnbo NAME,SIZE,RO,TYPE | awk -v root_disk=\"$root_disk\" '$4 == \"disk\" && $1 != root_disk && $3 == 0 && $2 >= 1073741824 {{ print \"/dev/\" $1; exit }}')\"\nif [ -z \"$data_device\" ]; then\n  echo \"[ctx-avf-linux] could not locate writable data disk\" >/dev/hvc0\n  exit 1\nfi\nmkdir -p \"$mount_root\"\nif ! blkid -s TYPE -o value \"$data_device\" >/dev/null 2>&1; then\n  mkfs.ext4 -F -L \"$data_label\" \"$data_device\" >/dev/hvc0 2>&1\nfi\ncurrent_mount_source=\"$(findmnt -n -o SOURCE \"$mount_root\" 2>/dev/null || true)\"\nif [ -n \"$current_mount_source\" ] && [ \"$current_mount_source\" != \"$data_device\" ]; then\n  umount \"$mount_root\" >/dev/null 2>&1 || true\n  current_mount_source=\"\"\nfi\nif [ \"$current_mount_source\" != \"$data_device\" ]; then\n  mount \"$data_device\" \"$mount_root\" >/dev/hvc0 2>&1\nfi\nmkdir -p \"$mount_root/ws/worktrees\" \"$mount_root/home\" \"$mount_root/cache\" \"$mount_root/tmp\" \"$mount_root/system/containerd\" \"$mount_root/system/buildkit\" /var/lib/containerd /var/lib/buildkit /tmp /var/tmp\nchmod 1777 \"$mount_root/tmp\"\ncurrent_tmp_source=\"$(findmnt -n -o SOURCE /tmp 2>/dev/null || true)\"\nif [ \"$current_tmp_source\" != \"$mount_root/tmp\" ]; then\n  mountpoint -q /tmp && umount /tmp >/dev/null 2>&1 || true\n  mount --bind \"$mount_root/tmp\" /tmp >/dev/hvc0 2>&1\nfi\nchmod 1777 /tmp\ncurrent_var_tmp_source=\"$(findmnt -n -o SOURCE /var/tmp 2>/dev/null || true)\"\nif [ \"$current_var_tmp_source\" != \"$mount_root/tmp\" ]; then\n  mountpoint -q /var/tmp && umount /var/tmp >/dev/null 2>&1 || true\n  mount --bind \"$mount_root/tmp\" /var/tmp >/dev/hvc0 2>&1\nfi\nchmod 1777 /var/tmp\nif [ ! -f \"$mount_root/$marker_name\" ]; then\n  printf 'ready\\n' > \"$mount_root/$marker_name\"\nfi\necho \"[ctx-avf-linux] mounted data disk $data_device at $mount_root\" >/dev/hvc0\nmountpoint -q /var/lib/containerd || mount --bind \"$mount_root/system/containerd\" /var/lib/containerd >/dev/hvc0 2>&1\nmountpoint -q /var/lib/buildkit || mount --bind \"$mount_root/system/buildkit\" /var/lib/buildkit >/dev/hvc0 2>&1\n",
        data_label = SHARED_VM_DATA_DISK_LABEL,
    )
}

pub(super) fn shared_vm_writable_surface_contract_digest(data_root: &Path) -> String {
    let mut hasher = Sha256::new();
    for rendered in [
        render_shared_vm_host_data_mount_service(data_root),
        render_shared_vm_data_disk_script(),
        render_shared_vm_data_disk_service(),
        render_shared_vm_containerd_service(),
        render_shared_vm_buildkit_service(),
        render_shared_vm_guest_agent_service(
            &shared_vm_guest_control_ready_path(data_root),
            &shared_vm_guest_control_failed_path(data_root),
            &shared_vm_guest_agent_log_path(data_root),
        ),
    ] {
        hasher.update(rendered.as_bytes());
        hasher.update(b"\0");
    }
    hex::encode(hasher.finalize())
}

pub(super) fn render_shared_vm_data_disk_service() -> String {
    format!(
        "[Unit]\nDescription=ctx AVF Data Disk Setup\nAfter=local-fs.target\nBefore={containerd_service} {buildkit_service} {guest_agent_service}\n\n[Service]\nType=oneshot\nExecStart=/bin/sh -lc 'exec {script_path}'\nRemainAfterExit=yes\n\n[Install]\nWantedBy=multi-user.target\n# {service_name}\n",
        containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
        buildkit_service = SHARED_VM_BUILDKIT_SERVICE_NAME,
        guest_agent_service = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        script_path = SHARED_VM_DATA_DISK_INSTALL_PATH,
        service_name = SHARED_VM_DATA_DISK_SERVICE_NAME,
    )
}

pub(super) fn render_shared_vm_containerd_service() -> String {
    format!(
        "[Unit]\nDescription=containerd Container Runtime\nAfter=network-online.target local-fs.target {data_disk_service}\nWants=network-online.target\nRequires={data_disk_service}\n\n[Service]\nType=simple\nExecStartPre=/bin/sh -lc 'mkdir -p /run/containerd /var/lib/containerd'\nExecStart=/usr/local/bin/containerd\nRestart=always\nRestartSec=1\nKillMode=process\nDelegate=yes\n\n[Install]\nWantedBy=multi-user.target\n# {containerd_service}\n",
        data_disk_service = SHARED_VM_DATA_DISK_SERVICE_NAME,
        containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
    )
}

pub(super) fn render_shared_vm_buildkit_service() -> String {
    format!(
        "[Unit]\nDescription=BuildKit\nAfter={containerd_service} network-online.target local-fs.target\nWants=network-online.target\nRequires={containerd_service}\n\n[Service]\nType=simple\nExecStartPre=/bin/sh -lc 'mkdir -p /run/buildkit /var/lib/buildkit /etc/buildkit'\nExecStart=/usr/local/bin/buildkitd --config /etc/buildkit/buildkitd.toml --addr {buildkit_socket}\nRestart=always\nRestartSec=1\n\n[Install]\nWantedBy=multi-user.target\n# {buildkit_service}\n",
        containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
        buildkit_service = SHARED_VM_BUILDKIT_SERVICE_NAME,
        buildkit_socket = SHARED_VM_GUEST_BUILDKIT_SOCKET,
    )
}

pub(super) fn render_shared_vm_container_stack_install_script(
    container_stack_host_path: &Path,
    container_stack_sha256: &str,
) -> String {
    let escaped_payload_path =
        shell_escape_single_quotes(&container_stack_host_path.display().to_string());
    let escaped_expected_sha = shell_escape_single_quotes(container_stack_sha256);
    let escaped_marker_path =
        shell_escape_single_quotes(SHARED_VM_GUEST_CONTAINER_STACK_MARKER_PATH);
    format!(
        "#!/bin/sh\nset -eu\npayload='{payload_path}'\nexpected_sha='{expected_sha}'\nmarker='{marker_path}'\nif [ ! -f \"$payload\" ]; then\n  echo \"[ctx-avf-linux] missing guest container-stack payload at $payload\" >/dev/hvc0\n  exit 1\nfi\nactual_sha=\"$(sha256sum \"$payload\" | awk '{{print $1}}')\"\nif [ \"$actual_sha\" != \"$expected_sha\" ]; then\n  echo \"[ctx-avf-linux] guest container-stack sha mismatch: expected $expected_sha got $actual_sha\" >/dev/hvc0\n  exit 1\nfi\nif [ -f \"$marker\" ] && [ \"$(cat \"$marker\" 2>/dev/null || true)\" = \"$expected_sha\" ]; then\n  exit 0\nfi\nmkdir -p /usr/local /usr/local/lib/ctx /etc/containerd /etc/buildkit /var/lib/containerd /var/lib/buildkit /run/containerd /run/buildkit\ntar -xzf \"$payload\" -C /usr/local\ncat > /etc/containerd/config.toml <<'EOF'\nversion = 2\nroot = \"/var/lib/containerd\"\nstate = \"/run/containerd\"\n[grpc]\n  address = \"/run/containerd/containerd.sock\"\nEOF\ncat > /etc/buildkit/buildkitd.toml <<'EOF'\nroot = \"/var/lib/buildkit\"\n[worker.oci]\n  enabled = false\n[worker.containerd]\n  enabled = true\n  namespace = \"default\"\nEOF\nprintf '%s\\n' \"$expected_sha\" > \"$marker\"\nchmod 0644 \"$marker\"\n",
        payload_path = escaped_payload_path,
        expected_sha = escaped_expected_sha,
        marker_path = escaped_marker_path,
    )
}

pub(super) fn shell_escape_single_quotes(value: &str) -> String {
    value.replace('\'', "'\"'\"'")
}

pub(super) fn hash_shared_vm_seed_component(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub(super) fn shared_vm_cloud_init_seed_digest(
    meta_data: &str,
    user_data: &str,
    network_config: &str,
) -> String {
    let mut seed_material =
        Vec::with_capacity(meta_data.len() + user_data.len() + network_config.len());
    seed_material.extend_from_slice(meta_data.as_bytes());
    seed_material.extend_from_slice(user_data.as_bytes());
    seed_material.extend_from_slice(network_config.as_bytes());
    hash_shared_vm_seed_component(&seed_material)
}

fn shared_vm_cloud_init_seed_digest_path(data_root: &Path) -> PathBuf {
    shared_vm_cloud_init_root(data_root).join(".seed-digest")
}

pub(super) fn render_shared_vm_cloud_init_meta_data(
    data_root: &Path,
    guest_agent_bytes: &[u8],
    egress_proxy_bytes: Option<&[u8]>,
    container_stack_sha256: &str,
) -> String {
    let mut seed_material = Vec::with_capacity(guest_agent_bytes.len() + 256);
    seed_material.extend_from_slice(guest_agent_bytes);
    if let Some(egress_proxy_bytes) = egress_proxy_bytes {
        seed_material.extend_from_slice(egress_proxy_bytes);
    }
    seed_material.extend_from_slice(container_stack_sha256.as_bytes());
    seed_material.extend_from_slice(render_shared_vm_data_disk_script().as_bytes());
    seed_material.extend_from_slice(render_shared_vm_data_disk_service().as_bytes());
    seed_material.extend_from_slice(
        render_shared_vm_guest_agent_service(
            &shared_vm_guest_control_ready_path(data_root),
            &shared_vm_guest_control_failed_path(data_root),
            &shared_vm_guest_agent_log_path(data_root),
        )
        .as_bytes(),
    );
    seed_material.extend_from_slice(render_shared_vm_containerd_service().as_bytes());
    seed_material.extend_from_slice(render_shared_vm_buildkit_service().as_bytes());
    let seed_hash = hash_shared_vm_seed_component(&seed_material);
    format!("instance-id: ctx-avf-linux-{seed_hash}\nlocal-hostname: ctx-avf-linux\n")
}

pub(super) fn render_shared_vm_cloud_init_user_data(
    data_root: &Path,
    guest_agent_bytes: &[u8],
    egress_proxy_bytes: Option<&[u8]>,
    container_stack_host_path: &Path,
    container_stack_sha256: &str,
) -> String {
    let guest_agent_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(guest_agent_bytes), 6);
    let guest_agent_service = indent_cloud_init_block(
        &render_shared_vm_guest_agent_service(
            &shared_vm_guest_control_ready_path(data_root),
            &shared_vm_guest_control_failed_path(data_root),
            &shared_vm_guest_agent_log_path(data_root),
        ),
        6,
    );
    let guest_agent_launcher = indent_cloud_init_block(
        &render_shared_vm_guest_agent_launcher_script(
            &shared_vm_guest_control_ready_path(data_root),
            &shared_vm_guest_control_failed_path(data_root),
            &shared_vm_guest_agent_log_path(data_root),
        ),
        6,
    );
    let host_data_service =
        indent_cloud_init_block(&render_shared_vm_host_data_mount_service(data_root), 6);
    let data_disk_script = indent_cloud_init_block(&render_shared_vm_data_disk_script(), 6);
    let data_disk_service = indent_cloud_init_block(&render_shared_vm_data_disk_service(), 6);
    let containerd_service = indent_cloud_init_block(&render_shared_vm_containerd_service(), 6);
    let buildkit_service = indent_cloud_init_block(&render_shared_vm_buildkit_service(), 6);
    let install_script = indent_cloud_init_block(
        &render_shared_vm_container_stack_install_script(
            container_stack_host_path,
            container_stack_sha256,
        ),
        6,
    );
    let egress_proxy_block = egress_proxy_bytes.map(|bytes| {
        let egress_proxy_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(bytes), 6);
        format!(
            "  - path: /usr/local/bin/ctx-egress-proxy\n    permissions: '0755'\n    encoding: b64\n    content: |\n{egress_proxy_b64}\n"
        )
    });
    let escaped_container_stack_host_path =
        shell_escape_single_quotes(&container_stack_host_path.display().to_string());
    let mut write_files = String::new();
    write_files.push_str(&format!(
        "  - path: /usr/local/bin/ctx-avf-linux-guest-agent\n    permissions: '0755'\n    encoding: b64\n    content: |\n{guest_agent_b64}\n"
    ));
    write_files.push_str(&format!(
        "  - path: {launcher_path}\n    permissions: '0755'\n    content: |\n{guest_agent_launcher}\n",
        launcher_path = SHARED_VM_GUEST_AGENT_LAUNCHER_PATH,
    ));
    write_files.push_str(&egress_proxy_block.unwrap_or_default());
    write_files.push_str(&format!(
        "  - path: {data_disk_install_path}\n    permissions: '0755'\n    content: |\n{data_disk_script}\n",
        data_disk_install_path = SHARED_VM_DATA_DISK_INSTALL_PATH,
        data_disk_script = data_disk_script,
    ));
    write_files.push_str(&format!(
        "  - path: {install_path}\n    permissions: '0755'\n    content: |\n{install_script}\n",
        install_path = SHARED_VM_GUEST_CONTAINER_STACK_INSTALL_PATH,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{data_disk_service_name}\n    permissions: '0644'\n    content: |\n{data_disk_service}\n",
        data_disk_service_name = SHARED_VM_DATA_DISK_SERVICE_NAME,
        data_disk_service = data_disk_service,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{host_data_service_name}\n    permissions: '0644'\n    content: |\n{host_data_service}\n",
        host_data_service_name = SHARED_VM_HOST_DATA_SERVICE_NAME,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{containerd_service_name}\n    permissions: '0644'\n    content: |\n{containerd_service}\n",
        containerd_service_name = SHARED_VM_CONTAINERD_SERVICE_NAME,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{buildkit_service_name}\n    permissions: '0644'\n    content: |\n{buildkit_service}\n",
        buildkit_service_name = SHARED_VM_BUILDKIT_SERVICE_NAME,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{guest_agent_service_name}\n    permissions: '0644'\n    content: |\n{guest_agent_service}\n",
        guest_agent_service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
    ));
    let prepare_guest_agent_cmd = indent_cloud_init_block(
        &format!(
            "echo \"[ctx-avf-linux] preparing {guest_agent_service_name}\" >/dev/hvc0\nls -l /usr/local/bin/ctx-avf-linux-guest-agent >/dev/hvc0 2>&1\nls -l {launcher_path} >/dev/hvc0 2>&1\nls -l /etc/systemd/system/{guest_agent_service_name} >/dev/hvc0 2>&1\nls -l '{container_stack_host_path}' >/dev/hvc0 2>&1",
            guest_agent_service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
            launcher_path = SHARED_VM_GUEST_AGENT_LAUNCHER_PATH,
            container_stack_host_path = escaped_container_stack_host_path,
        ),
        4,
    );
    let enable_data_disk_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {data_disk_service_name} >/dev/hvc0 2>&1 || (systemctl status {data_disk_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            data_disk_service_name = SHARED_VM_DATA_DISK_SERVICE_NAME,
        ),
        4,
    );
    let enable_host_data_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {host_data_service_name} >/dev/hvc0 2>&1 || (systemctl status {host_data_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            host_data_service_name = SHARED_VM_HOST_DATA_SERVICE_NAME,
        ),
        4,
    );
    let install_container_stack_cmd = indent_cloud_init_block(
        &format!(
            "{install_path} >/dev/hvc0 2>&1",
            install_path = SHARED_VM_GUEST_CONTAINER_STACK_INSTALL_PATH,
        ),
        4,
    );
    let enable_containerd_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {containerd_service_name} >/dev/hvc0 2>&1 || (systemctl status {containerd_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            containerd_service_name = SHARED_VM_CONTAINERD_SERVICE_NAME,
        ),
        4,
    );
    let enable_buildkit_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {buildkit_service_name} >/dev/hvc0 2>&1 || (systemctl status {buildkit_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            buildkit_service_name = SHARED_VM_BUILDKIT_SERVICE_NAME,
        ),
        4,
    );
    let enable_guest_agent_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {guest_agent_service_name} >/dev/hvc0 2>&1 || (systemctl status {guest_agent_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            guest_agent_service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        ),
        4,
    );
    format!(
        "#cloud-config\nwrite_files:\n{write_files}runcmd:\n  - [ systemctl, daemon-reload ]\n  - |\n{enable_host_data_cmd}\n  - |\n{enable_data_disk_cmd}\n  - |\n{prepare_guest_agent_cmd}\n  - |\n{install_container_stack_cmd}\n  - |\n{enable_containerd_cmd}\n  - |\n{enable_buildkit_cmd}\n  - |\n{enable_guest_agent_cmd}\n",
        prepare_guest_agent_cmd = prepare_guest_agent_cmd,
        enable_host_data_cmd = enable_host_data_cmd,
        enable_data_disk_cmd = enable_data_disk_cmd,
        install_container_stack_cmd = install_container_stack_cmd,
        enable_containerd_cmd = enable_containerd_cmd,
        enable_buildkit_cmd = enable_buildkit_cmd,
        enable_guest_agent_cmd = enable_guest_agent_cmd,
    )
}

pub(super) fn render_shared_vm_cloud_init_network_config() -> &'static str {
    "version: 2\nethernets:\n  default:\n    match:\n      name: \"en*\"\n    dhcp4: true\n    optional: true\n"
}

fn sha256_hex_file(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let read = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn stage_shared_vm_runtime_payload(source_path: &Path, destination_path: &Path) -> Result<()> {
    if let Some(parent) = destination_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp_path = destination_path.with_extension("tmp");
    fs::copy(source_path, &tmp_path).with_context(|| {
        format!(
            "staging shared VM runtime payload {} -> {}",
            source_path.display(),
            tmp_path.display()
        )
    })?;
    fs::rename(&tmp_path, destination_path).with_context(|| {
        format!(
            "finalizing shared VM runtime payload {} -> {}",
            tmp_path.display(),
            destination_path.display()
        )
    })?;
    Ok(())
}

pub(super) fn stage_shared_vm_cloud_init_seed(
    data_root: &Path,
    runtime_root: &Path,
    preserve_existing_image: bool,
) -> Result<Option<PathBuf>> {
    let guest_agent_path = shared_vm_guest_agent_helper_path(runtime_root);
    if !guest_agent_path.is_file() {
        bail!(
            "AVF Linux runtime is missing guest-agent payload at {}",
            guest_agent_path.display()
        );
    }
    let egress_proxy_path = shared_vm_egress_proxy_helper_path(runtime_root);
    let container_stack_runtime_path = shared_vm_container_stack_helper_path(runtime_root);
    if !container_stack_runtime_path.is_file() {
        bail!(
            "AVF Linux runtime is missing guest container-stack payload at {}",
            container_stack_runtime_path.display()
        );
    }
    let container_stack_payload_path = shared_vm_container_stack_payload_path(data_root);
    stage_shared_vm_runtime_payload(&container_stack_runtime_path, &container_stack_payload_path)?;
    let container_stack_sha256 = sha256_hex_file(&container_stack_payload_path)?;
    let image_path = shared_vm_cloud_init_image_path(data_root);
    let seed_root = shared_vm_cloud_init_root(data_root);
    let guest_agent_bytes = fs::read(&guest_agent_path)
        .with_context(|| format!("reading {}", guest_agent_path.display()))?;
    let egress_proxy_bytes = if egress_proxy_path.is_file() {
        Some(
            fs::read(&egress_proxy_path)
                .with_context(|| format!("reading {}", egress_proxy_path.display()))?,
        )
    } else {
        None
    };
    let meta_data = render_shared_vm_cloud_init_meta_data(
        data_root,
        &guest_agent_bytes,
        egress_proxy_bytes.as_deref(),
        &container_stack_sha256,
    );
    let user_data = render_shared_vm_cloud_init_user_data(
        data_root,
        &guest_agent_bytes,
        egress_proxy_bytes.as_deref(),
        &container_stack_payload_path,
        &container_stack_sha256,
    );
    let network_config = render_shared_vm_cloud_init_network_config();
    let seed_digest = shared_vm_cloud_init_seed_digest(&meta_data, &user_data, &network_config);
    let seed_digest_path = shared_vm_cloud_init_seed_digest_path(data_root);
    if preserve_existing_image
        && image_path.is_file()
        && fs::read_to_string(&seed_digest_path)
            .ok()
            .map(|value| value.trim() == seed_digest)
            .unwrap_or(false)
    {
        return Ok(Some(image_path));
    }

    fs::remove_dir_all(&seed_root).ok();
    fs::create_dir_all(&seed_root).with_context(|| format!("creating {}", seed_root.display()))?;
    fs::write(shared_vm_cloud_init_meta_data_path(data_root), &meta_data).with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_meta_data_path(data_root).display()
        )
    })?;
    fs::write(shared_vm_cloud_init_user_data_path(data_root), &user_data).with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_user_data_path(data_root).display()
        )
    })?;
    fs::write(
        shared_vm_cloud_init_network_config_path(data_root),
        &network_config,
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_network_config_path(data_root).display()
        )
    })?;
    fs::write(&seed_digest_path, format!("{seed_digest}\n"))
        .with_context(|| format!("writing {}", seed_digest_path.display()))?;

    fs::remove_file(&image_path).ok();
    let image = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&image_path)
        .with_context(|| format!("creating {}", image_path.display()))?;
    image
        .set_len(16 * 1024 * 1024)
        .with_context(|| format!("sizing {}", image_path.display()))?;
    drop(image);

    let raw_device = attach_raw_disk_image_nomount(&image_path)?;
    let mut raw_device_attached = true;
    let mut mounted_device: Option<String> = None;
    let result = (|| -> Result<()> {
        run_command(
            "diskutil",
            &[
                "partitionDisk",
                &raw_device,
                "MBR",
                "MS-DOS",
                "CIDATA",
                "100%",
            ],
            "partitioning raw cloud-init image",
        )?;
        detach_disk_image_device(&raw_device)?;
        raw_device_attached = false;
        let (device, volume_path) = attach_raw_disk_image_with_mount(&image_path)?;
        mounted_device = Some(device);
        fs::copy(
            shared_vm_cloud_init_meta_data_path(data_root),
            volume_path.join("meta-data"),
        )
        .context("copying cloud-init meta-data into mounted seed volume")?;
        fs::copy(
            shared_vm_cloud_init_user_data_path(data_root),
            volume_path.join("user-data"),
        )
        .context("copying cloud-init user-data into mounted seed volume")?;
        fs::copy(
            shared_vm_cloud_init_network_config_path(data_root),
            volume_path.join("network-config"),
        )
        .context("copying cloud-init network-config into mounted seed volume")?;
        Ok(())
    })();

    if let Some(device) = mounted_device.as_deref() {
        let _ = detach_disk_image_device(device);
    }
    if raw_device_attached {
        let _ = detach_disk_image_device(&raw_device);
    }
    result?;

    Ok(Some(image_path))
}

pub(super) fn run_command(program: &str, args: &[&str], context_label: &str) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("spawning {program} for {context_label}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let details = [stdout, stderr]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if details.is_empty() {
        bail!("{context_label} failed with status {}", output.status);
    }
    bail!(
        "{context_label} failed with status {}:\n{details}",
        output.status
    );
}

pub(super) fn attach_raw_disk_image_nomount(image_path: &Path) -> Result<String> {
    let output = run_command(
        "hdiutil",
        &[
            "attach",
            "-nomount",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
            &image_path.display().to_string(),
        ],
        "attaching raw cloud-init image without mounting",
    )?;
    output
        .lines()
        .find_map(|line| line.split_whitespace().next())
        .filter(|value| value.starts_with("/dev/"))
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow::anyhow!("could not determine raw device from hdiutil output"))
}

pub(super) fn attach_raw_disk_image_with_mount(image_path: &Path) -> Result<(String, PathBuf)> {
    let output = run_command(
        "hdiutil",
        &[
            "attach",
            "-imagekey",
            "diskimage-class=CRawDiskImage",
            &image_path.display().to_string(),
        ],
        "attaching raw cloud-init image with mount",
    )?;
    for line in output.lines() {
        let parts = line
            .split('\t')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }
        let device = parts[0];
        let mount = parts[2];
        if device.starts_with("/dev/") && mount.starts_with("/Volumes/") {
            return Ok((device.to_string(), PathBuf::from(mount)));
        }
    }
    bail!("could not determine mounted cloud-init volume from hdiutil output");
}

pub(super) fn detach_disk_image_device(device: &str) -> Result<()> {
    let _ = run_command(
        "hdiutil",
        &["detach", device],
        "detaching raw cloud-init image",
    )?;
    Ok(())
}
