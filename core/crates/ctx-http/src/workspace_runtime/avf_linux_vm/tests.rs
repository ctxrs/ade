use super::*;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

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

fn helper_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

fn write_probe_helper(dir: &Path) -> PathBuf {
    let helper = dir.join("ctx-avf-linux-helper");
    std::fs::write(
        &helper,
        r#"#!/bin/sh
set -eu
case "${1:-}" in
  probe)
    cat <<'JSON'
{"protocol_version":1,"protocol_schema":"ctx.avf_linux_helper.v1","helper_version":"test-helper","host_os":"macos","host_arch":"aarch64","supported":true,"save_restore_supported":true,"rosetta_supported":true,"notes":["ready"]}
JSON
    ;;
  *)
    echo "unsupported" >&2
    exit 1
    ;;
esac
"#,
    )
    .expect("write probe helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&helper)
            .expect("probe helper metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&helper, perms).expect("chmod probe helper");
    }
    helper
}

fn write_lifecycle_helper(dir: &Path) -> PathBuf {
    let helper = dir.join("ctx-avf-linux-helper");
    std::fs::write(
        &helper,
        r#"#!/usr/bin/env python3
import json
import os
import pathlib
import sys

PROTOCOL_VERSION = 1
PROTOCOL_SCHEMA = "ctx.avf_linux_helper.v1"
STATE_FILE = "shared-vm-state.json"
STATE_LOG = "shared-vm.log"

def vm_root(data_root: str) -> pathlib.Path:
    return pathlib.Path(data_root) / "avf-linux-vm" / "shared-vm"

def state_path(data_root: str) -> pathlib.Path:
    return vm_root(data_root) / STATE_FILE

def write_state(data_root: str, state: str, transition: str | None = None):
    root = vm_root(data_root)
    root.mkdir(parents=True, exist_ok=True)
    payload = {
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "state": state,
        "vm_root": str(root),
        "logs_root": str(root / "logs"),
        "state_path": str(root / STATE_FILE),
        "saved_state_path": str(root / "saved-machine-state.vzvmsave"),
        "saved_state_exists": state != "missing",
        "runtime_root": str(root / "runtime"),
        "rootfs_image": str(root / "runtime" / "rootfs.raw"),
        "kernel_path": str(root / "runtime" / "helpers" / "kernel"),
        "initrd_path": str(root / "runtime" / "helpers" / "initrd"),
        "runtime_version": "test-runtime",
        "log_path": str(root / STATE_LOG),
        "simulated": True,
        "notes": ["test helper"],
    }
    if transition is not None:
        payload["transition_status"] = transition
    state_path(data_root).write_text(json.dumps(payload), encoding="utf-8")

cmd = sys.argv[1]
if cmd == "probe":
    print(json.dumps({
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "helper_version": "test-helper",
        "host_os": "macos",
        "host_arch": "aarch64",
        "supported": True,
        "save_restore_supported": True,
        "rosetta_supported": True,
        "notes": ["ready"],
    }))
elif cmd == "prepare-runtime-layout":
    data_root = sys.argv[2]
    root = vm_root(data_root)
    root.mkdir(parents=True, exist_ok=True)
    (root / "logs").mkdir(exist_ok=True)
    print(json.dumps({
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "vm_root": str(root),
        "logs_root": str(root / "logs"),
        "state_path": str(root / STATE_FILE),
        "layout_status": "prepared",
        "notes": [],
    }))
elif cmd == "shared-vm-state" or cmd == "workspace-vm-state":
    data_root = sys.argv[2]
    path = state_path(data_root)
    if path.exists():
        print(path.read_text(encoding="utf-8"))
    else:
        root = vm_root(data_root)
        print(json.dumps({
            "protocol_version": PROTOCOL_VERSION,
            "protocol_schema": PROTOCOL_SCHEMA,
            "state": "missing",
            "vm_root": str(root),
            "logs_root": str(root / "logs"),
            "state_path": str(root / STATE_FILE),
            "saved_state_exists": False,
            "simulated": True,
            "notes": [],
        }))
elif cmd == "start-shared-vm" or cmd == "start-workspace-vm":
    data_root = sys.argv[2]
    write_state(data_root, "running")
    print(state_path(data_root).read_text(encoding="utf-8"))
elif cmd == "stop-shared-vm" or cmd == "stop-workspace-vm":
    data_root = sys.argv[2]
    write_state(data_root, "stopped", "stopped")
    print(state_path(data_root).read_text(encoding="utf-8"))
else:
    print(f"unsupported command: {cmd}", file=sys.stderr)
    sys.exit(1)
"#,
    )
    .expect("write lifecycle helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&helper)
            .expect("lifecycle helper metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&helper, perms).expect("chmod lifecycle helper");
    }
    helper
}

fn write_guest_exec_helper(dir: &Path) -> (PathBuf, PathBuf) {
    let helper = dir.join("ctx-avf-linux-helper");
    let capture_file = dir.join("guest-exec-capture.json");
    let capture_path = capture_file.display().to_string().replace('\\', "\\\\");
    std::fs::write(
        &helper,
        format!(
            r#"#!/usr/bin/env python3
import json
import pathlib
import sys

PROTOCOL_VERSION = 1
PROTOCOL_SCHEMA = "ctx.avf_linux_helper.v1"
CAPTURE_PATH = pathlib.Path("{capture_path}")

def workspace_vm_root(data_root: str, workspace_id: str) -> pathlib.Path:
    return pathlib.Path(data_root) / "avf-linux-vm" / "workspaces" / workspace_id

def worktree_root(data_root: str, workspace_id: str, worktree_id: str) -> pathlib.Path:
    return workspace_vm_root(data_root, workspace_id) / "worktrees" / worktree_id

cmd = sys.argv[1]
if cmd == "probe":
    print(json.dumps({{
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "helper_version": "guest-exec-helper",
        "host_os": "macos",
        "host_arch": "aarch64",
        "supported": True,
        "save_restore_supported": True,
        "rosetta_supported": True,
        "notes": ["ready"],
    }}))
elif cmd == "prepare-runtime-layout":
    data_root = sys.argv[2]
    root = pathlib.Path(data_root) / "avf-linux-vm" / "shared-vm"
    root.mkdir(parents=True, exist_ok=True)
    (root / "logs").mkdir(exist_ok=True)
    print(json.dumps({{
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "vm_root": str(root),
        "logs_root": str(root / "logs"),
        "state_path": str(root / "shared-vm-state.json"),
        "layout_status": "prepared",
        "notes": [],
    }}))
elif cmd == "start-shared-vm" or cmd == "start-workspace-vm":
    data_root = sys.argv[2]
    root = pathlib.Path(data_root) / "avf-linux-vm" / "shared-vm"
    root.mkdir(parents=True, exist_ok=True)
    (root / "logs").mkdir(exist_ok=True)
    print(json.dumps({{
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "state": "running",
        "vm_root": str(root),
        "logs_root": str(root / "logs"),
        "state_path": str(root / "shared-vm-state.json"),
        "saved_state_exists": True,
        "saved_state_path": str(root / "saved-machine-state.vzvmsave"),
        "runtime_root": str(root / "runtime"),
        "rootfs_image": str(root / "runtime" / "rootfs.raw"),
        "kernel_path": str(root / "runtime" / "helpers" / "kernel"),
        "initrd_path": str(root / "runtime" / "helpers" / "initrd"),
        "runtime_version": "test-runtime",
        "log_path": str(root / "shared-vm.log"),
        "simulated": True,
        "notes": ["guest exec ready"],
    }}))
elif cmd == "prepare-guest-worktree":
    data_root = sys.argv[2]
    workspace_id = sys.argv[3]
    worktree_id = sys.argv[4]
    branch_name = sys.argv[7]
    root = worktree_root(data_root, workspace_id, worktree_id)
    shadow = root / "shadow-root"
    shadow.mkdir(parents=True, exist_ok=True)
    (shadow / ".git").mkdir(exist_ok=True)
    metadata_path = root / "worktree.json"
    payload = {{
        "protocol_version": PROTOCOL_VERSION,
        "protocol_schema": PROTOCOL_SCHEMA,
        "workspace_id": workspace_id,
        "worktree_id": worktree_id,
        "guest_root": str(pathlib.Path("/ctx/ws/worktrees") / worktree_id),
        "host_shadow_root": str(shadow),
        "metadata_path": str(metadata_path),
        "status": "prepared",
        "simulated": True,
        "notes": [branch_name],
    }}
    metadata_path.parent.mkdir(parents=True, exist_ok=True)
    metadata_path.write_text(json.dumps(payload), encoding="utf-8")
    print(json.dumps(payload))
elif cmd == "guest-exec":
    args = sys.argv[2:]
    data_root = ""
    workspace_id = ""
    worktree_id = ""
    cwd = ""
    command = ""
    env = {{}}
    passthrough = []
    idx = 0
    while idx < len(args):
        arg = args[idx]
        if arg == "--data-root":
            data_root = args[idx + 1]
            idx += 2
        elif arg == "--workspace-id":
            workspace_id = args[idx + 1]
            idx += 2
        elif arg == "--worktree-id":
            worktree_id = args[idx + 1]
            idx += 2
        elif arg == "--cwd":
            cwd = args[idx + 1]
            idx += 2
        elif arg == "--command":
            command = args[idx + 1]
            idx += 2
        elif arg == "--env":
            key, value = args[idx + 1].split("=", 1)
            env[key] = value
            idx += 2
        elif arg == "--user":
            idx += 2
        elif arg == "--pty":
            idx += 1
        elif arg == "--":
            passthrough = args[idx + 1:]
            break
        else:
            raise SystemExit(f"unexpected guest-exec arg: {{arg}}")
    payload = {{
        "data_root": data_root,
        "workspace_id": workspace_id,
        "worktree_id": worktree_id,
        "cwd": cwd,
        "command": command,
        "args": passthrough,
        "env": env,
    }}
    CAPTURE_PATH.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print("guest-exec-ok")
else:
    print(f"unsupported command: {{cmd}}", file=sys.stderr)
    sys.exit(1)
"#,
        ),
    )
    .expect("write guest exec helper");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&helper)
            .expect("guest exec helper metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&helper, perms).expect("chmod guest exec helper");
    }
    (helper, capture_file)
}

fn runtime_archive_bytes() -> Vec<u8> {
    let temp = tempfile::tempdir().expect("temp runtime archive dir");
    let root = temp.path().join("avf-linux-runtime");
    std::fs::create_dir_all(root.join("helpers")).expect("create helpers");
    std::fs::write(root.join("rootfs.raw"), b"rootfs\n").expect("write rootfs");
    std::fs::write(root.join("helpers").join("kernel"), b"kernel\n").expect("write kernel");
    std::fs::write(root.join("helpers").join("initrd"), b"initrd\n").expect("write initrd");
    std::fs::write(root.join("helpers").join("guest-agent"), b"guest-agent\n")
        .expect("write guest agent");
    std::fs::write(root.join("helpers").join("egress-proxy"), b"egress-proxy\n")
        .expect("write egress proxy");
    std::fs::write(
        root.join("version.txt"),
        "version=managed-runtime\nubuntu-release=noble\nubuntu-arch=arm64\n",
    )
    .expect("write version metadata");

    let archive_path = temp.path().join("runtime.tar.gz");
    let status = std::process::Command::new("tar")
        .arg("-czf")
        .arg(&archive_path)
        .arg("-C")
        .arg(temp.path())
        .arg("avf-linux-runtime")
        .status()
        .expect("spawn tar");
    assert!(status.success(), "tar should succeed");
    std::fs::read(&archive_path).expect("read archive")
}

async fn spawn_static_http_server(
    body: Vec<u8>,
    requests: usize,
) -> Result<(url::Url, JoinHandle<Result<()>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("bind static http server")?;
    let addr = listener
        .local_addr()
        .context("static http server local addr")?;
    let url = url::Url::parse(&format!("http://127.0.0.1:{}/runtime.tar.gz", addr.port()))
        .context("static http server url")?;
    let handle = tokio::spawn(async move {
        for _ in 0..requests {
            let (mut stream, _) = listener.accept().await.context("accept static http")?;
            let mut request_buf = vec![0_u8; 4096];
            let _ = stream.read(&mut request_buf).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .context("write static http headers")?;
            stream
                .write_all(&body)
                .await
                .context("write static http body")?;
        }
        Ok(())
    });
    Ok((url, handle))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[test]
fn helper_probe_uses_configured_helper_binary() {
    let temp = tempfile::tempdir().unwrap();
    let helper = write_probe_helper(temp.path());
    let _guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, helper.to_str().unwrap());

    let probe = probe_helper().unwrap();
    assert!(probe.supported);
    assert_eq!(probe.helper_version, "test-helper");
    assert_eq!(probe.host_os, "macos");
    assert_eq!(probe.host_arch, "aarch64");
}

#[tokio::test]
async fn managed_avf_linux_runtime_downloads_archive_and_helpers() {
    let _helper_lock = helper_env_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let archive = runtime_archive_bytes();
    let archive_sha = sha256_hex(&archive);
    let (archive_url, server_handle) = spawn_static_http_server(archive.clone(), 1).await.unwrap();

    let helper_bytes = b"#!/bin/sh\nexit 0\n".to_vec();
    let helper_sha = sha256_hex(&helper_bytes);
    let (helper_url, helper_server_handle) = spawn_static_http_server(helper_bytes.clone(), 4)
        .await
        .unwrap();

    let source = bundled_assets::ManagedRuntimeSource {
        version: "managed-runtime".to_string(),
        uri: archive_url.to_string(),
        sha256: archive_sha.clone(),
        bin: "rootfs.raw".to_string(),
        helpers: HashMap::from([
            (
                AVF_LINUX_KERNEL_HELPER.to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: helper_url.to_string(),
                    sha256: helper_sha.clone(),
                },
            ),
            (
                AVF_LINUX_INITRD_HELPER.to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: helper_url.to_string(),
                    sha256: helper_sha.clone(),
                },
            ),
            (
                AVF_LINUX_GUEST_AGENT_HELPER.to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: helper_url.to_string(),
                    sha256: helper_sha.clone(),
                },
            ),
            (
                AVF_LINUX_EGRESS_PROXY_HELPER.to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: helper_url.to_string(),
                    sha256: helper_sha.clone(),
                },
            ),
        ]),
    };

    let runtime = runtime_assets::ensure_managed_avf_linux_guest_runtime_with_override(
        temp.path(),
        Some(&source),
        None,
        None,
    )
    .await
    .unwrap();

    server_handle.await.unwrap().unwrap();
    helper_server_handle.await.unwrap().unwrap();

    assert_eq!(runtime.version, "managed-runtime");
    assert!(runtime.managed);
    assert!(runtime.runtime_root.exists());
    assert!(runtime.rootfs_image.exists());
    assert!(runtime.kernel_path.exists());
    assert!(runtime.initrd_path.exists());
    let guest_agent_path = runtime
        .guest_agent_path
        .as_ref()
        .expect("guest agent helper should exist");
    assert!(guest_agent_path.exists());
    let egress_proxy_path = runtime
        .egress_proxy_path
        .as_ref()
        .expect("egress proxy helper should exist");
    assert!(egress_proxy_path.exists());

    let archive_path = runtime_assets::managed_avf_linux_archive_path(temp.path(), &source);
    assert!(archive_path.exists());
    assert_eq!(
        tokio::fs::read_to_string(runtime_assets::managed_avf_linux_runtime_ready_marker_path(
            &runtime.runtime_root
        ))
        .await
        .unwrap(),
        "ready"
    );
    assert!(runtime_assets::avf_linux_runtime_is_ready(&runtime));
    assert_eq!(
        tokio::fs::read(&runtime.kernel_path).await.unwrap(),
        helper_bytes
    );
    assert_eq!(
        tokio::fs::read(&runtime.initrd_path).await.unwrap(),
        helper_bytes
    );
    assert_eq!(
        tokio::fs::read(guest_agent_path).await.unwrap(),
        helper_bytes
    );
    assert_eq!(
        tokio::fs::read(egress_proxy_path).await.unwrap(),
        helper_bytes
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let kernel_mode = tokio::fs::metadata(&runtime.kernel_path)
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(kernel_mode, 0o644);

        let initrd_mode = tokio::fs::metadata(&runtime.initrd_path)
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(initrd_mode, 0o644);

        let guest_agent_mode = tokio::fs::metadata(guest_agent_path)
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(guest_agent_mode, 0o755);

        let egress_proxy_mode = tokio::fs::metadata(egress_proxy_path)
            .await
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(egress_proxy_mode, 0o755);
    }
}

#[tokio::test]
async fn ensure_avf_linux_runtime_prefers_bundled_guest_runtime_over_managed_source() {
    let _helper_lock = helper_env_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let bundle_root = temp.path().join("bundle");
    let runtime_root = bundle_root
        .join("runtimes")
        .join("avf-linux-guest")
        .join(format!(
            "{}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    let helpers_root = runtime_root.join("helpers");
    std::fs::create_dir_all(&helpers_root).unwrap();
    let rootfs_path = runtime_root.join("rootfs.raw");
    let kernel_path = helpers_root.join("kernel");
    let initrd_path = helpers_root.join("initrd");
    let guest_agent_path = helpers_root.join("guest-agent");
    let egress_proxy_path = helpers_root.join("egress-proxy");
    std::fs::write(&rootfs_path, b"rootfs").unwrap();
    std::fs::write(&kernel_path, b"kernel").unwrap();
    std::fs::write(&initrd_path, b"initrd").unwrap();
    std::fs::write(&guest_agent_path, b"guest-agent").unwrap();
    std::fs::write(&egress_proxy_path, b"egress-proxy").unwrap();
    let manifest_path = bundle_root.join("manifest.json");
    std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    std::fs::write(
        &manifest_path,
        serde_json::json!({
            "runtimes": [{
                "id": AVF_LINUX_GUEST_RUNTIME_ID,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "version": "bundled-runtime",
                "root": format!(
                    "runtimes/{}/{}/{}",
                    AVF_LINUX_GUEST_RUNTIME_ID,
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
                "bin": "rootfs.raw"
            }],
            "providers": [],
            "images": [],
            "daemons": []
        })
        .to_string(),
    )
    .unwrap();
    let _bundle_dir = EnvGuard::set("CTX_BUNDLE_DIR", bundle_root.to_str().unwrap());
    let _bundle_manifest = EnvGuard::set("CTX_BUNDLE_MANIFEST", manifest_path.to_str().unwrap());
    assert!(
        bundled_assets::bundled_avf_linux_guest_runtime().is_some(),
        "bundled AVF Linux runtime should resolve from the test bundle manifest"
    );
    let source = bundled_assets::ManagedRuntimeSource {
        version: "managed-runtime".to_string(),
        uri: "https://example.invalid/runtime.tar.gz".to_string(),
        sha256: "deadbeef".to_string(),
        bin: "rootfs.raw".to_string(),
        helpers: HashMap::new(),
    };
    let _source_override = override_managed_avf_linux_runtime_source_for_test(source.clone());

    let runtime = runtime_assets::ensure_managed_avf_linux_guest_runtime_with_override(
        temp.path(),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(runtime.version, "bundled-runtime");
    assert!(!runtime.managed);
    assert_eq!(runtime.runtime_root, runtime_root);
    assert_eq!(runtime.rootfs_image, rootfs_path);
    assert_eq!(runtime.kernel_path, kernel_path);
    assert_eq!(runtime.initrd_path, initrd_path);
    assert_eq!(
        runtime.guest_agent_path.as_deref(),
        Some(guest_agent_path.as_path())
    );
    assert_eq!(
        runtime.egress_proxy_path.as_deref(),
        Some(egress_proxy_path.as_path())
    );
}

#[test]
fn helper_lifecycle_commands_round_trip_structured_state() {
    let temp = tempfile::tempdir().unwrap();
    let helper = write_lifecycle_helper(temp.path());
    let _guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, helper.to_str().unwrap());

    let layout = prepare_runtime_layout(temp.path()).unwrap();
    assert_eq!(layout.layout_status, AvfLinuxRuntimeLayoutStatus::Prepared);
    assert!(layout.vm_root.ends_with("shared-vm"));

    let initial = shared_vm_state(temp.path()).unwrap();
    assert_eq!(initial.state, AvfLinuxSharedVmLifecycleState::Missing);
    assert!(initial.simulated);

    let runtime = AvfLinuxGuestRuntime {
        runtime_root: temp.path().join("runtime"),
        rootfs_image: temp.path().join("runtime/rootfs.raw"),
        kernel_path: temp.path().join("runtime/helpers/kernel"),
        initrd_path: temp.path().join("runtime/helpers/initrd"),
        guest_agent_path: Some(temp.path().join("runtime/helpers/guest-agent")),
        egress_proxy_path: Some(temp.path().join("runtime/helpers/egress-proxy")),
        version: "test-runtime".to_string(),
        managed: false,
    };
    let started = start_shared_vm(temp.path(), &runtime).unwrap();
    assert_eq!(started.state, AvfLinuxSharedVmLifecycleState::Running);
    assert_eq!(started.runtime_version.as_deref(), Some("test-runtime"));

    let state_after_start = shared_vm_state(temp.path()).unwrap();
    assert_eq!(
        state_after_start.state,
        AvfLinuxSharedVmLifecycleState::Running
    );

    let stopped = stop_shared_vm(temp.path()).unwrap();
    assert_eq!(stopped.state, AvfLinuxSharedVmLifecycleState::Stopped);
    assert_eq!(
        stopped.transition_status,
        Some(AvfLinuxSharedVmTransitionStatus::Stopped)
    );

    let state_after_stop = shared_vm_state(temp.path()).unwrap();
    assert_eq!(
        state_after_stop.state,
        AvfLinuxSharedVmLifecycleState::Stopped
    );
}

#[test]
fn helper_prepare_guest_worktree_round_trips_structured_state() {
    let temp = tempfile::tempdir().unwrap();
    let (helper, capture_path) = write_guest_exec_helper(temp.path());
    let _guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, helper.to_str().unwrap());

    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();
    let host_root = temp.path().join("workspace");
    std::fs::create_dir_all(host_root.join(".git")).unwrap();
    std::fs::write(host_root.join("README.md"), "hello\n").unwrap();

    let prepared = prepare_guest_worktree(
        temp.path(),
        workspace_id,
        worktree_id,
        &host_root,
        "abc123def456",
        "ctx/test-branch",
    )
    .unwrap();
    assert_eq!(prepared.status, AvfLinuxGuestWorktreeStatus::Prepared);
    assert!(prepared.simulated);
    assert_eq!(prepared.notes, vec!["ctx/test-branch"]);

    let expected_guest_root = PathBuf::from("/ctx/ws/worktrees").join(worktree_id.0.to_string());
    assert_eq!(prepared.guest_root, expected_guest_root);
    assert!(prepared.host_shadow_root.exists());
    assert!(prepared.host_shadow_root.join("README.md").exists());
    assert!(prepared.host_shadow_root.join(".git").exists());

    let state = workspace_vm_state(temp.path(), workspace_id).unwrap();
    assert_eq!(state.state, AvfLinuxSharedVmLifecycleState::Missing);

    let output = futures::executor::block_on(run_guest_exec_capture(
        temp.path(),
        workspace_id,
        worktree_id,
        Path::new(&host_root),
        "python3",
        &["-c".to_string(), "print('ok')".to_string()],
        &HashMap::from([
            ("CTX_CUSTOM".to_string(), "value".to_string()),
            ("PATH".to_string(), "/custom/bin".to_string()),
        ]),
        None,
        false,
    ))
    .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "guest-exec-ok"
    );

    let captured: serde_json::Value =
        serde_json::from_slice(&std::fs::read(capture_path).unwrap()).unwrap();
    assert_eq!(captured["workspace_id"], workspace_id.0.to_string());
    assert_eq!(captured["worktree_id"], worktree_id.0.to_string());
    assert_eq!(captured["cwd"], host_root.display().to_string());
    assert_eq!(captured["command"], "python3");
    assert_eq!(captured["args"], serde_json::json!(["-c", "print('ok')"]));
    assert_eq!(captured["env"]["CTX_CUSTOM"], "value");
    assert_eq!(captured["env"]["PATH"], "/custom/bin");
}

#[tokio::test]
async fn run_guest_exec_capture_invokes_helper_with_expected_args() {
    let _helper_lock = helper_env_test_lock().lock().await;
    let temp = tempfile::tempdir().unwrap();
    let (helper, capture_path) = write_guest_exec_helper(temp.path());
    let _guard = EnvGuard::set(AVF_LINUX_HELPER_PATH_ENV, helper.to_str().unwrap());

    let workspace_id = WorkspaceId::new();
    let worktree_id = WorktreeId::new();
    let host_root = temp.path().join("workspace");
    tokio::fs::create_dir_all(host_root.join(".git"))
        .await
        .unwrap();
    tokio::fs::write(host_root.join("README.md"), b"hello\n")
        .await
        .unwrap();

    let output = run_guest_exec_capture(
        temp.path(),
        workspace_id,
        worktree_id,
        Path::new(&host_root),
        "python3",
        &["-c".to_string(), "print('ok')".to_string()],
        &HashMap::from([("CTX_SAMPLE".to_string(), "value".to_string())]),
        None,
        false,
    )
    .await
    .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "guest-exec-ok"
    );

    let captured: serde_json::Value =
        serde_json::from_slice(&tokio::fs::read(capture_path).await.unwrap()).unwrap();
    assert_eq!(captured["workspace_id"], workspace_id.0.to_string());
    assert_eq!(captured["worktree_id"], worktree_id.0.to_string());
    assert_eq!(captured["guest_root"], host_root.display().to_string());
    assert_eq!(captured["cwd"], host_root.display().to_string());
    assert_eq!(captured["command"], "python3");
    assert_eq!(captured["args"], serde_json::json!(["-c", "print('ok')"]));
    assert_eq!(captured["env"]["CTX_SAMPLE"], "value");
}
