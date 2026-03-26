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

pub(super) fn render_shared_vm_guest_agent_service() -> String {
    format!(
        "[Unit]\nDescription=ctx AVF Linux Guest Agent\n\n[Service]\nType=simple\nEnvironment=RUST_BACKTRACE=1\nExecStartPre=/bin/sh -lc 'echo \"[ctx-avf-linux] starting guest-agent\" >/dev/hvc0'\nExecStart=/bin/sh -lc 'exec /usr/local/bin/ctx-avf-linux-guest-agent'\nStandardOutput=journal+console\nStandardError=journal+console\nRestart=always\nRestartSec=1\n\n[Install]\nWantedBy=multi-user.target\n# {}\n",
        SHARED_VM_GUEST_AGENT_SERVICE_NAME
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

pub(super) fn render_shared_vm_grow_rootfs_script() -> String {
    "#!/bin/sh\nset -eu\nroot_device=\"$(findmnt -n -o SOURCE /)\"\nif [ -z \"$root_device\" ]; then\n  echo \"[ctx-avf-linux] could not determine root device\" >/dev/hvc0\n  exit 1\nfi\nroot_device=\"$(readlink -f \"$root_device\" 2>/dev/null || printf '%s' \"$root_device\")\"\ncase \"$root_device\" in\n  /dev/*) ;;\n  *)\n    echo \"[ctx-avf-linux] unsupported root device $root_device\" >/dev/hvc0\n    exit 1\n    ;;\nesac\nif ! command -v growpart >/dev/null 2>&1; then\n  echo \"[ctx-avf-linux] missing growpart\" >/dev/hvc0\n  exit 1\nfi\nif ! command -v resize2fs >/dev/null 2>&1; then\n  echo \"[ctx-avf-linux] missing resize2fs\" >/dev/hvc0\n  exit 1\nfi\ndisk_name=\"$(lsblk -nro PKNAME \"$root_device\" | head -n1)\"\npart_number=\"$(lsblk -nro PARTN \"$root_device\" | head -n1)\"\nif [ -z \"$disk_name\" ] || [ -z \"$part_number\" ]; then\n  echo \"[ctx-avf-linux] could not resolve parent disk for $root_device\" >/dev/hvc0\n  exit 1\nfi\ngrow_output=\"\"\ngrow_status=0\nif ! grow_output=\"$(growpart \"/dev/$disk_name\" \"$part_number\" 2>&1)\"; then\n  grow_status=$?\nfi\nif [ \"$grow_status\" -ne 0 ]; then\n  case \"$grow_output\" in\n    *NOCHANGE:*)\n      printf '%s\\n' \"$grow_output\" >/dev/hvc0\n      ;;\n    *)\n      printf '%s\\n' \"$grow_output\" >/dev/hvc0\n      exit \"$grow_status\"\n      ;;\n  esac\nelif [ -n \"$grow_output\" ]; then\n  printf '%s\\n' \"$grow_output\" >/dev/hvc0\nfi\nresize2fs \"$root_device\" >/dev/hvc0 2>&1\n".to_string()
}

pub(super) fn render_shared_vm_grow_rootfs_service() -> String {
    format!(
        "[Unit]\nDescription=ctx AVF Root Filesystem Growth\nAfter=local-fs.target\nBefore={containerd_service} {buildkit_service} {guest_agent_service}\n\n[Service]\nType=oneshot\nExecStart=/bin/sh -lc 'exec {script_path}'\nRemainAfterExit=yes\n\n[Install]\nWantedBy=multi-user.target\n# {service_name}\n",
        containerd_service = SHARED_VM_CONTAINERD_SERVICE_NAME,
        buildkit_service = SHARED_VM_BUILDKIT_SERVICE_NAME,
        guest_agent_service = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        script_path = SHARED_VM_GROW_ROOTFS_INSTALL_PATH,
        service_name = SHARED_VM_GROW_ROOTFS_SERVICE_NAME,
    )
}

pub(super) fn render_shared_vm_containerd_service() -> String {
    format!(
        "[Unit]\nDescription=containerd Container Runtime\nAfter=network-online.target local-fs.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStartPre=/bin/sh -lc 'mkdir -p /run/containerd /var/lib/containerd'\nExecStart=/usr/local/bin/containerd\nRestart=always\nRestartSec=1\nKillMode=process\nDelegate=yes\n\n[Install]\nWantedBy=multi-user.target\n# {}\n",
        SHARED_VM_CONTAINERD_SERVICE_NAME
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

pub(super) fn render_shared_vm_cloud_init_meta_data(
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
    seed_material.extend_from_slice(render_shared_vm_guest_agent_service().as_bytes());
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
    let guest_agent_service = indent_cloud_init_block(&render_shared_vm_guest_agent_service(), 6);
    let host_data_service =
        indent_cloud_init_block(&render_shared_vm_host_data_mount_service(data_root), 6);
    let grow_rootfs_script = indent_cloud_init_block(&render_shared_vm_grow_rootfs_script(), 6);
    let grow_rootfs_service = indent_cloud_init_block(&render_shared_vm_grow_rootfs_service(), 6);
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
    write_files.push_str(&egress_proxy_block.unwrap_or_default());
    write_files.push_str(&format!(
        "  - path: {grow_rootfs_install_path}\n    permissions: '0755'\n    content: |\n{grow_rootfs_script}\n",
        grow_rootfs_install_path = SHARED_VM_GROW_ROOTFS_INSTALL_PATH,
    ));
    write_files.push_str(&format!(
        "  - path: {install_path}\n    permissions: '0755'\n    content: |\n{install_script}\n",
        install_path = SHARED_VM_GUEST_CONTAINER_STACK_INSTALL_PATH,
    ));
    write_files.push_str(&format!(
        "  - path: /etc/systemd/system/{grow_rootfs_service_name}\n    permissions: '0644'\n    content: |\n{grow_rootfs_service}\n",
        grow_rootfs_service_name = SHARED_VM_GROW_ROOTFS_SERVICE_NAME,
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
            "echo \"[ctx-avf-linux] preparing {guest_agent_service_name}\" >/dev/hvc0\nls -l /usr/local/bin/ctx-avf-linux-guest-agent >/dev/hvc0 2>&1\nls -l /etc/systemd/system/{guest_agent_service_name} >/dev/hvc0 2>&1\nls -l '{container_stack_host_path}' >/dev/hvc0 2>&1",
            guest_agent_service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
            container_stack_host_path = escaped_container_stack_host_path,
        ),
        4,
    );
    let enable_grow_rootfs_cmd = indent_cloud_init_block(
        &format!(
            "systemctl enable --now {grow_rootfs_service_name} >/dev/hvc0 2>&1 || (systemctl status {grow_rootfs_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)",
            grow_rootfs_service_name = SHARED_VM_GROW_ROOTFS_SERVICE_NAME,
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
        "#cloud-config\nwrite_files:\n{write_files}runcmd:\n  - [ systemctl, daemon-reload ]\n  - |\n{enable_grow_rootfs_cmd}\n  - |\n{enable_host_data_cmd}\n  - |\n{prepare_guest_agent_cmd}\n  - |\n{install_container_stack_cmd}\n  - |\n{enable_containerd_cmd}\n  - |\n{enable_buildkit_cmd}\n  - |\n{enable_guest_agent_cmd}\n",
        prepare_guest_agent_cmd = prepare_guest_agent_cmd,
        enable_grow_rootfs_cmd = enable_grow_rootfs_cmd,
        enable_host_data_cmd = enable_host_data_cmd,
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
    if preserve_existing_image && image_path.is_file() {
        return Ok(Some(image_path));
    }

    let seed_root = shared_vm_cloud_init_root(data_root);
    fs::remove_dir_all(&seed_root).ok();
    fs::create_dir_all(&seed_root).with_context(|| format!("creating {}", seed_root.display()))?;
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
    fs::write(
        shared_vm_cloud_init_meta_data_path(data_root),
        render_shared_vm_cloud_init_meta_data(
            &guest_agent_bytes,
            egress_proxy_bytes.as_deref(),
            &container_stack_sha256,
        ),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_meta_data_path(data_root).display()
        )
    })?;
    fs::write(
        shared_vm_cloud_init_user_data_path(data_root),
        render_shared_vm_cloud_init_user_data(
            data_root,
            &guest_agent_bytes,
            egress_proxy_bytes.as_deref(),
            &container_stack_payload_path,
            &container_stack_sha256,
        ),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_user_data_path(data_root).display()
        )
    })?;
    fs::write(
        shared_vm_cloud_init_network_config_path(data_root),
        render_shared_vm_cloud_init_network_config(),
    )
    .with_context(|| {
        format!(
            "writing {}",
            shared_vm_cloud_init_network_config_path(data_root).display()
        )
    })?;

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
