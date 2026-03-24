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
) -> String {
    let mut seed_material = Vec::with_capacity(guest_agent_bytes.len() + 256);
    seed_material.extend_from_slice(guest_agent_bytes);
    if let Some(egress_proxy_bytes) = egress_proxy_bytes {
        seed_material.extend_from_slice(egress_proxy_bytes);
    }
    seed_material.extend_from_slice(render_shared_vm_guest_agent_service().as_bytes());
    let seed_hash = hash_shared_vm_seed_component(&seed_material);
    format!("instance-id: ctx-avf-linux-{seed_hash}\nlocal-hostname: ctx-avf-linux\n")
}

pub(super) fn render_shared_vm_cloud_init_user_data(
    data_root: &Path,
    guest_agent_bytes: &[u8],
    egress_proxy_bytes: Option<&[u8]>,
) -> String {
    let guest_agent_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(guest_agent_bytes), 6);
    let service = indent_cloud_init_block(&render_shared_vm_guest_agent_service(), 6);
    let host_data_service = indent_cloud_init_block(
        &render_shared_vm_host_data_mount_service(data_root),
        6,
    );
    let egress_proxy_block = egress_proxy_bytes.map(|bytes| {
        let egress_proxy_b64 = indent_cloud_init_block(&wrap_cloud_init_base64(bytes), 6);
        format!(
            "  - path: /usr/local/bin/ctx-egress-proxy\n    permissions: '0755'\n    encoding: b64\n    content: |\n{egress_proxy_b64}\n"
        )
    });
    format!(
        "#cloud-config\npackages:\n  - podman\n  - uidmap\n  - slirp4netns\n  - fuse-overlayfs\nwrite_files:\n  - path: /usr/local/bin/ctx-avf-linux-guest-agent\n    permissions: '0755'\n    encoding: b64\n    content: |\n{guest_agent_b64}\n{egress_proxy_block}  - path: /etc/systemd/system/{host_data_service_name}\n    permissions: '0644'\n    content: |\n{host_data_service}\n  - path: /etc/systemd/system/{service_name}\n    permissions: '0644'\n    content: |\n{service}\nruncmd:\n  - [ sh, -lc, 'echo \"[ctx-avf-linux] preparing {service_name}\" >/dev/hvc0; ls -l /usr/local/bin/ctx-avf-linux-guest-agent >/dev/hvc0 2>&1; ls -l /etc/systemd/system/{service_name} >/dev/hvc0 2>&1' ]\n  - [ systemctl, daemon-reload ]\n  - [ sh, -lc, 'systemctl enable --now {host_data_service_name} >/dev/hvc0 2>&1 || (systemctl status {host_data_service_name} --no-pager >/dev/hvc0 2>&1; exit 1)' ]\n  - [ sh, -lc, 'systemctl enable --now {service_name} >/dev/hvc0 2>&1 || (systemctl status {service_name} --no-pager >/dev/hvc0 2>&1; exit 1)' ]\n",
        service_name = SHARED_VM_GUEST_AGENT_SERVICE_NAME,
        host_data_service_name = SHARED_VM_HOST_DATA_SERVICE_NAME,
        host_data_service = host_data_service,
        egress_proxy_block = egress_proxy_block.unwrap_or_default(),
    )
}

pub(super) fn render_shared_vm_cloud_init_network_config() -> &'static str {
    "version: 2\nethernets:\n  default:\n    match:\n      name: \"en*\"\n    dhcp4: true\n    optional: true\n"
}

pub(super) fn stage_shared_vm_cloud_init_seed(
    data_root: &Path,
    runtime_root: &Path,
    preserve_existing_image: bool,
) -> Result<Option<PathBuf>> {
    let guest_agent_path = shared_vm_guest_agent_helper_path(runtime_root);
    if !guest_agent_path.is_file() {
        return Ok(None);
    }
    let egress_proxy_path = shared_vm_egress_proxy_helper_path(runtime_root);
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
        render_shared_vm_cloud_init_meta_data(&guest_agent_bytes, egress_proxy_bytes.as_deref()),
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
