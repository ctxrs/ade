use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(windows)]
use ctx_bundled_assets as bundled_assets;
use ctx_bundled_assets::test_support::{
    override_managed_ctx_harness_image_source_for_test,
    override_managed_sandbox_machine_cache_source_for_test, ManagedArtifactSource,
    TestManagedCtxHarnessImageSourceGuard, TestManagedSandboxMachineCacheSourceGuard,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Barrier, Notify, Semaphore};
use tokio::task::JoinHandle;

use crate::execution_setup::warmup_coordination::SharedWarmupOperations;
use crate::ops_events::OpsEvents;
use crate::perf_telemetry::PerfTelemetry;
use crate::settings::{ContainerRuntimeKind, ExecutionMode, ExecutionSettings, Settings};
use crate::test_support::{
    install_test_managed_avf_linux_runtime_source, wait_for_execution_launch_terminal,
    write_avf_linux_lifecycle_helper, write_running_container_sandbox_cli_shim,
    TrackedExecutionLaunch,
};
use ctx_store::Store;
use ctx_workspace_runtime::HarnessRuntimeManager;

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

    fn unset(key: &'static str) -> Self {
        let prev = std::env::var(key).ok();
        std::env::remove_var(key);
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

fn env_var_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::sandbox_cli_env_test_lock()
}

fn process_env_test_lock() -> &'static tokio::sync::Mutex<()> {
    crate::test_support::process_env_test_lock()
}

const BACKGROUND_TEST_TIMEOUT: Duration = Duration::from_secs(10);
const QUICK_ASYNC_TEST_TIMEOUT: Duration = Duration::from_secs(5);

fn test_workspace(id: WorkspaceId) -> Workspace {
    Workspace {
        id,
        name: "ws".to_string(),
        root_path: "/tmp/ws".to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    }
}

fn sandbox_container_settings() -> crate::settings::ContainerExecutionSettings {
    crate::settings::ContainerExecutionSettings {
        runtime: ContainerRuntimeKind::NativeContainer,
        mount_mode: crate::settings::ContainerMountMode::DiskIsolated,
        ..Default::default()
    }
}

fn sandbox_execution_settings() -> ExecutionSettings {
    ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: sandbox_container_settings(),
    }
}

fn test_coordinator(data_root: PathBuf) -> Arc<ExecutionSetupCoordinator> {
    Arc::new(ExecutionSetupCoordinator::new(
        data_root.clone(),
        Arc::new(HarnessRuntimeManager::new(data_root.clone())),
        PerfTelemetry::new(data_root.clone()),
        OpsEvents::new(data_root),
    ))
}

fn test_coordinator_with_operations(
    data_root: PathBuf,
    operations: Arc<dyn SharedWarmupOperations>,
) -> Arc<ExecutionSetupCoordinator> {
    Arc::new(ExecutionSetupCoordinator::new_with_operations(
        data_root.clone(),
        Arc::new(HarnessRuntimeManager::new(data_root.clone())),
        PerfTelemetry::new(data_root.clone()),
        OpsEvents::new(data_root),
        operations,
    ))
}

async fn init_settings_store(data_root: &Path) {
    let db_path = data_root.join("db").join("db.sqlite");
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("create db directory");
    }
    let store = Store::open_sqlite(&db_path, Some(1))
        .await
        .expect("open sqlite store");
    crate::settings::save_settings(&store, &crate::settings::Settings::default())
        .await
        .expect("save default settings");
    store.close().await;
}

async fn load_execution_settings_from_store(data_root: &Path) -> ExecutionSettings {
    let db_path = data_root.join("db").join("db.sqlite");
    let store = Store::open_sqlite(&db_path, Some(1))
        .await
        .expect("open sqlite store");
    let settings = crate::settings::load_settings(&store)
        .await
        .expect("load settings");
    store.close().await;
    settings.execution.unwrap_or_default()
}

async fn run_startup_prewarm_from_store(coordinator: &Arc<ExecutionSetupCoordinator>) {
    let execution = load_execution_settings_from_store(&coordinator.data_root).await;
    coordinator.run_startup_prewarm(execution).await;
}

async fn spawn_startup_prewarm_from_store(coordinator: &Arc<ExecutionSetupCoordinator>) {
    let execution = load_execution_settings_from_store(&coordinator.data_root).await;
    coordinator.spawn_startup_prewarm(execution);
}

async fn wait_for_startup_prewarm_terminal(
    coordinator: &Arc<ExecutionSetupCoordinator>,
    timeout: Duration,
) -> StartupPrewarmSnapshot {
    tokio::time::timeout(timeout, async {
        loop {
            let latest = coordinator.startup_status().await;
            if latest.last_attempt_at.is_some() && latest.state != StartupPrewarmState::Running {
                break latest;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for startup prewarm terminal state")
}

fn bundle_tar_fingerprint(tar_path: &Path) -> String {
    let metadata = std::fs::metadata(tar_path).expect("stat bundled image tar");
    let len = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_secs())
        .unwrap_or(0);
    format!("{len}:{modified}")
}

fn write_startup_prewarm_sandbox_cli_shim(dir: &Path) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        "sandbox-cli-startup-test.cmd"
    } else {
        "sandbox-cli-startup-test.sh"
    });
    let script = if cfg!(windows) {
        "@echo off\r\nif \"%1\"==\"info\" (\r\n  >&2 echo engine unavailable\r\n  exit /b 125\r\n)\r\n>&2 echo unexpected sandbox CLI invocation: %*\r\nexit /b 1\r\n"
    } else {
        "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  echo 'engine unavailable' >&2\n  exit 125\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n"
    };
    std::fs::write(&path, script).expect("write startup prewarm sandbox CLI shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod startup prewarm sandbox CLI shim");
    }
    path
}

fn write_ready_runtime_sandbox_cli_shim(dir: &Path) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        "sandbox-cli-ready-runtime-test.cmd"
    } else {
        "sandbox-cli-ready-runtime-test.sh"
    });
    let script = if cfg!(windows) {
        "@echo off\r\nif \"%1\"==\"info\" (\r\n  echo {}\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"image\" if \"%2\"==\"inspect\" exit /b 0\r\n>&2 echo unexpected sandbox CLI invocation: %*\r\nexit /b 1\r\n"
    } else {
        "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n"
    };
    std::fs::write(&path, script).expect("write ready runtime sandbox CLI shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod ready runtime sandbox CLI shim");
    }
    path
}

fn write_logging_ready_runtime_sandbox_cli_shim(dir: &Path) -> (PathBuf, PathBuf) {
    let path = dir.join(if cfg!(windows) {
        "sandbox-cli-ready-runtime-logging-test.cmd"
    } else {
        "sandbox-cli-ready-runtime-logging-test.sh"
    });
    let log_path = dir.join("sandbox-cli-ready-runtime.log");
    let script = if cfg!(windows) {
        format!(
            "@echo off\r\nset \"LOG_PATH={log_path}\"\r\necho %*>>\"%LOG_PATH%\"\r\nif \"%1\"==\"info\" (\r\n  echo {{}}\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"image\" if \"%2\"==\"inspect\" exit /b 0\r\n>&2 echo unexpected sandbox CLI invocation: %*\r\nexit /b 1\r\n",
            log_path = log_path.display()
        )
    } else {
        format!(
            "#!/bin/sh\nlog_path=\"{log_path}\"\nprintf '%s\\n' \"$*\" >> \"$log_path\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log_path = log_path.display()
        )
    };
    std::fs::write(&path, script).expect("write logging ready runtime sandbox CLI shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod logging ready runtime sandbox CLI shim");
    }
    (path, log_path)
}

fn with_workspace_volume_support(script: String) -> String {
    script.replace(
        "if [ \"$1\" = \"container\" ]",
        "if [ \"$1\" = \"volume\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"volume\" ] && [ \"$2\" = \"create\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"inspect\" ]; then\n  suffix=${2#ctx-harness-}\n  printf '[{\"Mounts\":[{\"Type\":\"volume\",\"Name\":\"ctx-ws-%s\",\"Destination\":\"/ctx/ws\"}]}]\\n' \"$suffix\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ]",
    )
}

fn with_native_runtime_ready(script: String) -> String {
    script.replace(
        "if [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\n",
        "if [ \"$1\" = \"info\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\n",
    )
}

async fn save_test_execution_settings(data_root: &Path, execution: ExecutionSettings) {
    let db_path = data_root.join("db").join("db.sqlite");
    if !db_path.exists() {
        init_settings_store(data_root).await;
    }
    let store = Store::open_sqlite(&db_path, Some(1))
        .await
        .expect("open settings store");
    let settings = Settings {
        execution: Some(execution),
        ..Settings::default()
    };
    crate::settings::save_settings(&store, &settings)
        .await
        .expect("save settings");
    store.close().await;
}

fn count_matching_lines(contents: &str, needle: &str) -> usize {
    contents
        .lines()
        .filter(|line| line.contains(needle))
        .count()
}

async fn run_startup_prewarm_with_timeout(coordinator: &Arc<ExecutionSetupCoordinator>) {
    tokio::time::timeout(
        BACKGROUND_TEST_TIMEOUT,
        run_startup_prewarm_from_store(coordinator),
    )
    .await
    .expect("timed out running startup prewarm");
}

async fn spawn_static_http_server_with_suffix(
    body: Vec<u8>,
    suffix: &'static str,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind static http server");
    let addr = listener.local_addr().expect("static http local addr");
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
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

async fn spawn_static_http_server(body: Vec<u8>) -> (String, JoinHandle<()>) {
    spawn_static_http_server_with_suffix(body, "machine.raw").await
}

async fn install_test_managed_machine_cache_source(
    body: Vec<u8>,
) -> (TestManagedSandboxMachineCacheSourceGuard, JoinHandle<()>) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server(body).await;
    let guard = override_managed_sandbox_machine_cache_source_for_test(ManagedArtifactSource {
        uri: url,
        sha256: digest,
    });
    (guard, server)
}

async fn install_test_managed_harness_image_source(
    body: Vec<u8>,
) -> (TestManagedCtxHarnessImageSourceGuard, JoinHandle<()>) {
    let digest = {
        let mut hasher = Sha256::new();
        hasher.update(&body);
        hex::encode(hasher.finalize())
    };
    let (url, server) = spawn_static_http_server(body).await;
    let guard = override_managed_ctx_harness_image_source_for_test(ManagedArtifactSource {
        uri: url,
        sha256: digest,
    });
    (guard, server)
}

#[cfg(windows)]
#[expect(
    dead_code,
    reason = "legacy Windows-only AVF helper fixture kept until the shared lifecycle helper is ported"
)]
fn write_avf_linux_helper_shim(dir: &Path) -> PathBuf {
    let path = dir.join(if cfg!(windows) {
        "ctx-avf-linux-helper-test.cmd"
    } else {
        "ctx-avf-linux-helper-test.sh"
    });
    let sandbox_cli_path = dir.join(if cfg!(windows) {
        "ctx-avf-sandbox-cli-test.cmd"
    } else {
        "ctx-avf-sandbox-cli-test.sh"
    });
    let sandbox_cli_script = if cfg!(windows) {
        "@echo off\r\nset \"DATA_ROOT=%1\"\r\nshift\r\nset \"VM_ROOT=%DATA_ROOT%/managed/vms/avf-linux/macos/aarch64/shared\"\r\nset \"IMAGES_ROOT=%VM_ROOT%/test-images\"\r\nset \"LOG_PATH=%VM_ROOT%/sandbox-cli-invocations.log\"\r\nif not exist \"%IMAGES_ROOT%\" mkdir \"%IMAGES_ROOT%\"\r\nfor %%I in (\"%LOG_PATH%\") do if not exist \"%%~dpI\" mkdir \"%%~dpI\"\r\necho %*>>\"%LOG_PATH%\"\r\nif \"%1\"==\"info\" (\r\n  echo {}\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"image\" if \"%2\"==\"inspect\" (\r\n  if exist \"%IMAGES_ROOT%\\default-image\" (\r\n    echo []\r\n    exit /b 0\r\n  )\r\n  exit /b 1\r\n)\r\nif \"%1\"==\"load\" (\r\n  if \"%2\"==\"-i\" shift & shift\r\n  >\"%IMAGES_ROOT%\\default-image\" echo loaded\r\n  echo Loaded image: ctx-harness\r\n  exit /b 0\r\n)\r\n>&2 echo unexpected sandbox CLI invocation: %*\r\nexit /b 1\r\n".to_string()
    } else {
        "#!/bin/sh\ndata_root=\"$1\"\nshift\nvm_root=\"$data_root/managed/vms/avf-linux/macos/aarch64/shared\"\nimages_root=\"$vm_root/test-images\"\nlog_path=\"$vm_root/sandbox-cli-invocations.log\"\nmkdir -p \"$images_root\" \"$(dirname \"$log_path\")\"\nprintf '%s\\n' \"$*\" >> \"$log_path\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  find \"$images_root\" -mindepth 1 -maxdepth 1 | grep -q .\n  exit $?\nfi\nif [ \"$1\" = \"load\" ]; then\n  if [ \"$2\" = \"-i\" ]; then\n    shift 2\n  fi\n  : > \"$images_root/default-image\"\n  printf 'Loaded image: ctx-harness\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n"
            .to_string()
    };
    std::fs::write(&sandbox_cli_path, sandbox_cli_script).expect("write AVF sandbox CLI shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod AVF sandbox CLI shim");
    }
    let script = if cfg!(windows) {
        format!(
            "@echo off\r\nif \"%1\"==\"probe\" (\r\n  echo {{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}}\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"prepare-runtime-layout\" (\r\n  set \"DATA_ROOT=%2\"\r\n  set \"VM_ROOT=%DATA_ROOT%/managed/vms/avf-linux/macos/aarch64/shared\"\r\n  set \"LOGS_ROOT=%VM_ROOT%/logs\"\r\n  if not exist \"%LOGS_ROOT%\" mkdir \"%LOGS_ROOT%\"\r\n  echo {{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"vm_root\":\"%VM_ROOT%\",\"logs_root\":\"%LOGS_ROOT%\",\"state_path\":\"%VM_ROOT%/shared-vm-state.json\",\"layout_status\":\"prepared\",\"notes\":[\"layout ready\"]}}\r\n  exit /b 0\r\n)\r\nif \"%1\"==\"shared-vm-state\" goto shared_vm_state\r\nif \"%1\"==\"workspace-vm-state\" goto shared_vm_state\r\n:shared_vm_state\r\n  set \"DATA_ROOT=%2\"\r\n  set \"VM_ROOT=%DATA_ROOT%/managed/vms/avf-linux/macos/aarch64/shared\"\r\n  set \"LOGS_ROOT=%VM_ROOT%/logs\"\r\n  set \"STATUS=stopped\"\r\n  if exist \"%VM_ROOT%/helper-status.txt\" set /p STATUS=<\"%VM_ROOT%/helper-status.txt\"\r\n  if \"%STATUS%\"==\"running\" (\r\n    set \"RUNTIME_VERSION=\"\r\n    if exist \"%VM_ROOT%/runtime-version.txt\" set /p RUNTIME_VERSION=<\"%VM_ROOT%/runtime-version.txt\"\r\n    echo {{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%VM_ROOT%\",\"logs_root\":\"%LOGS_ROOT%\",\"state_path\":\"%VM_ROOT%/shared-vm-state.json\",\"log_path\":\"%LOGS_ROOT%/shared-vm.log\",\"runtime_version\":\"%RUNTIME_VERSION%\",\"transition_status\":\"ready\",\"last_start_outcome\":\"already_running\",\"simulated\":true,\"notes\":[\"state ready\"]}}\r\n    exit /b 0\r\n  )\r\n  echo {{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"%STATUS%\",\"vm_root\":\"%VM_ROOT%\",\"logs_root\":\"%LOGS_ROOT%\",\"state_path\":\"%VM_ROOT%/shared-vm-state.json\",\"log_path\":\"%LOGS_ROOT%/shared-vm.log\",\"simulated\":true,\"notes\":[\"state ready\"]}}\r\n  exit /b 0\r\nif \"%1\"==\"start-shared-vm\" goto start_shared_vm\r\nif \"%1\"==\"start-workspace-vm\" goto start_shared_vm\r\n:start_shared_vm\r\n  set \"DATA_ROOT=%2\"\r\n  set \"VM_ROOT=%DATA_ROOT%/managed/vms/avf-linux/macos/aarch64/shared\"\r\n  set \"LOGS_ROOT=%VM_ROOT%/logs\"\r\n  if not exist \"%LOGS_ROOT%\" mkdir \"%LOGS_ROOT%\"\r\n  >\"%VM_ROOT%/helper-status.txt\" echo running\r\n  >\"%VM_ROOT%/runtime-version.txt\" echo %7\r\n  echo {{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%VM_ROOT%\",\"logs_root\":\"%LOGS_ROOT%\",\"state_path\":\"%VM_ROOT%/shared-vm-state.json\",\"log_path\":\"%LOGS_ROOT%/shared-vm.log\",\"runtime_root\":\"%3\",\"rootfs_image\":\"%4\",\"kernel_path\":\"%5\",\"initrd_path\":\"%6\",\"runtime_version\":\"%7\",\"transition_status\":\"ready\",\"last_start_outcome\":\"cold_boot\",\"simulated\":true,\"notes\":[\"launch ready\"]}}\r\n  exit /b 0\r\nif \"%1\"==\"shared-vm-exec\" (\r\n  set \"DATA_ROOT=\"\r\n  set \"SHARED_COMMAND=\"\r\n  :shared_exec_parse\r\n  if \"%2\"==\"\" goto shared_exec_run\r\n  if \"%2\"==\"--data-root\" (\r\n    set \"DATA_ROOT=%3\"\r\n    shift\r\n    shift\r\n    goto shared_exec_parse\r\n  )\r\n  if \"%2\"==\"--command\" (\r\n    set \"SHARED_COMMAND=%3\"\r\n    shift\r\n    shift\r\n    goto shared_exec_parse\r\n  )\r\n  if \"%2\"==\"--cwd\" (\r\n    shift\r\n    shift\r\n    goto shared_exec_parse\r\n  )\r\n  if \"%2\"==\"--user\" (\r\n    shift\r\n    shift\r\n    goto shared_exec_parse\r\n  )\r\n  if \"%2\"==\"--env\" (\r\n    shift\r\n    shift\r\n    goto shared_exec_parse\r\n  )\r\n  if \"%2\"==\"--\" (\r\n    shift\r\n    goto shared_exec_run\r\n  )\r\n  >&2 echo unexpected shared-vm-exec arg: %2\r\n  exit /b 1\r\n  :shared_exec_run\r\n  if \"%SHARED_COMMAND%\"==\"sandbox-cli\" goto run_sandbox_cli\r\n  if \"%SHARED_COMMAND%\"==\"/usr/local/bin/nerdctl\" goto run_sandbox_cli\r\n  \"%SHARED_COMMAND%\" %*\r\n  exit /b %errorlevel%\r\n  :run_sandbox_cli\r\n  \"{sandbox_cli_path}\" \"%DATA_ROOT%\" %*\r\n  exit /b %errorlevel%\r\n)\r\n>&2 echo unexpected helper invocation: %*\r\nexit /b 1\r\n",
            sandbox_cli_path = sandbox_cli_path.display()
        )
    } else {
        format!(
            "#!/bin/sh\ncmd=\"$1\"\nshift\ncase \"$cmd\" in\nprobe)\n  printf '%s\\n' '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"helper_version\":\"0.0.0-test\",\"host_os\":\"macos\",\"host_arch\":\"aarch64\",\"supported\":true,\"save_restore_supported\":true,\"rosetta_supported\":true,\"notes\":[\"test helper\"]}}'\n  exit 0\n  ;;\nprepare-runtime-layout)\n  data_root=\"$1\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  mkdir -p \"$logs_root\"\n  printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"layout_status\":\"prepared\",\"notes\":[\"layout ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\"\n  exit 0\n  ;;\nshared-vm-state|workspace-vm-state)\n  data_root=\"$1\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  log_path=\"$logs_root/shared-vm.log\"\n  status_file=\"$vm_root/helper-status.txt\"\n  version_file=\"$vm_root/runtime-version.txt\"\n  state=$(cat \"$status_file\" 2>/dev/null || printf 'stopped')\n  runtime_version=$(cat \"$version_file\" 2>/dev/null || true)\n  if [ \"$state\" = \"running\" ]; then\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"runtime_version\":\"%s\",\"transition_status\":\"ready\",\"last_start_outcome\":\"already_running\",\"simulated\":true,\"notes\":[\"state ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\" \"$runtime_version\"\n  else\n    printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"%s\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"simulated\":true,\"notes\":[\"state ready\"]}}\\n' \"$state\" \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\"\n  fi\n  exit 0\n  ;;\nstart-shared-vm|start-workspace-vm)\n  data_root=\"$1\"\n  runtime_root=\"$2\"\n  rootfs_image=\"$3\"\n  kernel_path=\"$4\"\n  initrd_path=\"$5\"\n  runtime_version=\"$6\"\n  vm_root=\"$data_root/managed/vms/avf-linux/{host_os}/{host_arch}/shared\"\n  logs_root=\"$vm_root/logs\"\n  state_path=\"$vm_root/shared-vm-state.json\"\n  log_path=\"$logs_root/shared-vm.log\"\n  mkdir -p \"$logs_root\"\n  printf 'running' > \"$vm_root/helper-status.txt\"\n  printf '%s' \"$runtime_version\" > \"$vm_root/runtime-version.txt\"\n  printf '{{\"protocol_version\":1,\"protocol_schema\":\"ctx.avf_linux_helper.v1\",\"state\":\"running\",\"vm_root\":\"%s\",\"logs_root\":\"%s\",\"state_path\":\"%s\",\"log_path\":\"%s\",\"runtime_root\":\"%s\",\"rootfs_image\":\"%s\",\"kernel_path\":\"%s\",\"initrd_path\":\"%s\",\"runtime_version\":\"%s\",\"transition_status\":\"ready\",\"last_start_outcome\":\"cold_boot\",\"simulated\":true,\"notes\":[\"launch ready\"]}}\\n' \"$vm_root\" \"$logs_root\" \"$state_path\" \"$log_path\" \"$runtime_root\" \"$rootfs_image\" \"$kernel_path\" \"$initrd_path\" \"$runtime_version\"\n  exit 0\n  ;;\nshared-vm-exec)\n  data_root=\"\"\n  shared_command=\"\"\n  while [ $# -gt 0 ]; do\n    case \"$1\" in\n      --data-root) data_root=\"$2\"; shift 2 ;;\n      --command) shared_command=\"$2\"; shift 2 ;;\n      --cwd) shift 2 ;;\n      --user) shift 2 ;;\n      --env)\n        kv=\"$2\"\n        key=$(printf '%s' \"$kv\" | sed 's/=.*//')\n        value=$(printf '%s' \"$kv\" | sed 's/^[^=]*=//')\n        export \"$key=$value\"\n        shift 2\n        ;;\n      --) shift; break ;;\n      *) echo \"unexpected shared-vm-exec arg: $1\" >&2; exit 1 ;;\n    esac\n  done\n  if [ \"$shared_command\" = \"sandbox-cli\" ] || [ \"$shared_command\" = \"/usr/local/bin/nerdctl\" ]; then\n    exec \"{sandbox_cli_path}\" \"$data_root\" \"$@\"\n  fi\n  exec \"$shared_command\" \"$@\"\n  ;;\nesac\necho \"unexpected helper invocation: $cmd $*\" >&2\nexit 1\n",
            host_os = std::env::consts::OS,
            host_arch = std::env::consts::ARCH,
            sandbox_cli_path = sandbox_cli_path.display(),
        )
    };
    std::fs::write(&path, script).expect("write AVF Linux helper shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod AVF Linux helper shim");
    }
    path
}

#[cfg(windows)]
#[expect(
    dead_code,
    reason = "legacy Windows-only AVF helper fixture kept until the shared lifecycle helper is ported"
)]
fn avf_runtime_archive_bytes() -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tar = tar::Builder::new(&mut encoder);
        let payload = b"rootfs";
        let mut header = tar::Header::new_gnu();
        header.set_path("runtime/rootfs.img").expect("set tar path");
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, &payload[..])
            .expect("append rootfs image");
        tar.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip encoder")
}

#[cfg(windows)]
#[expect(
    dead_code,
    reason = "legacy Windows-only AVF helper fixture kept until the shared lifecycle helper is ported"
)]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(windows)]
#[expect(
    dead_code,
    reason = "legacy Windows-only AVF helper fixture kept until the shared lifecycle helper is ported"
)]
async fn make_test_managed_avf_linux_runtime_source(
) -> (bundled_assets::ManagedRuntimeSource, Vec<JoinHandle<()>>) {
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

    let source = bundled_assets::ManagedRuntimeSource {
        uri: archive_url,
        sha256: sha256_hex(&archive_bytes),
        version: "ubuntu-minimal-test".to_string(),
        bin: "rootfs.img".to_string(),
        helpers: [
            (
                "kernel".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: kernel_url,
                    sha256: sha256_hex(&kernel_bytes),
                },
            ),
            (
                "initrd".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: initrd_url,
                    sha256: sha256_hex(&initrd_bytes),
                },
            ),
            (
                "guest-agent".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: guest_agent_url,
                    sha256: sha256_hex(&guest_agent_bytes),
                },
            ),
            (
                "egress-proxy".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: egress_proxy_url,
                    sha256: sha256_hex(&egress_proxy_bytes),
                },
            ),
            (
                "container-stack".to_string(),
                bundled_assets::ManagedArtifactSource {
                    uri: container_stack_url,
                    sha256: sha256_hex(&container_stack_bytes),
                },
            ),
        ]
        .into_iter()
        .collect(),
    };
    (
        source,
        vec![
            archive_server,
            kernel_server,
            initrd_server,
            guest_agent_server,
            egress_proxy_server,
            container_stack_server,
        ],
    )
}

struct BlockingWarmupOperations {
    runtime_runs: AtomicUsize,
    launch_ready_runs: AtomicUsize,
    builder_runs: AtomicUsize,
    runtime_release: Semaphore,
    builder_release: Semaphore,
    runtime_notify: Notify,
    launch_ready_notify: Notify,
    builder_notify: Notify,
}

impl Default for BlockingWarmupOperations {
    fn default() -> Self {
        Self {
            runtime_runs: AtomicUsize::new(0),
            launch_ready_runs: AtomicUsize::new(0),
            builder_runs: AtomicUsize::new(0),
            runtime_release: Semaphore::new(0),
            builder_release: Semaphore::new(0),
            runtime_notify: Notify::new(),
            launch_ready_notify: Notify::new(),
            builder_notify: Notify::new(),
        }
    }
}

impl BlockingWarmupOperations {
    async fn wait_for_runtime_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.runtime_runs.load(Ordering::SeqCst) >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for runtime runs");
    }

    async fn wait_for_builder_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.builder_runs.load(Ordering::SeqCst) >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for builder runs");
    }

    async fn wait_for_launch_ready_runs(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if self.launch_ready_runs.load(Ordering::SeqCst) >= expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("timed out waiting for launch-ready runs");
    }

    fn release_runtime(&self) {
        self.runtime_release.add_permits(1);
    }

    fn release_builder(&self) {
        self.builder_release.add_permits(1);
    }
}

#[derive(Default)]
struct UnexpectedRuntimeWarmupOperations {
    runtime_runs: AtomicUsize,
}

#[async_trait]
impl SharedWarmupOperations for UnexpectedRuntimeWarmupOperations {
    async fn warm_runtime(
        &self,
        _settings: ExecutionSettings,
        _observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("unexpected runtime warmup")
    }

    async fn warm_runtime_launch_ready(
        &self,
        _settings: ExecutionSettings,
        _observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("unexpected launch-ready runtime warmup")
    }

    async fn warm_builder(&self, _observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl SharedWarmupOperations for BlockingWarmupOperations {
    async fn warm_runtime(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        self.runtime_notify.notify_waiters();
        observer.on_phase(HarnessSetupPhase::MachineCheck, "warming runtime");
        self.runtime_release
            .acquire()
            .await
            .expect("runtime release semaphore closed")
            .forget();
        Ok(())
    }

    async fn warm_runtime_launch_ready(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        self.launch_ready_runs.fetch_add(1, Ordering::SeqCst);
        self.runtime_notify.notify_waiters();
        self.launch_ready_notify.notify_waiters();
        observer.on_phase(
            HarnessSetupPhase::MachineStartOrInit,
            "warming launch-ready runtime",
        );
        self.runtime_release
            .acquire()
            .await
            .expect("runtime release semaphore closed")
            .forget();
        Ok(())
    }

    async fn warm_builder(&self, observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        self.builder_runs.fetch_add(1, Ordering::SeqCst);
        self.builder_notify.notify_waiters();
        observer.on_phase(HarnessSetupPhase::ImageLoad, "warming builder");
        self.builder_release
            .acquire()
            .await
            .expect("builder release semaphore closed")
            .forget();
        Ok(())
    }
}

#[derive(Default)]
struct RecordingStartupWarmupOperations {
    runtime_runs: AtomicUsize,
    steps: StdMutex<Vec<&'static str>>,
}

#[async_trait]
impl SharedWarmupOperations for RecordingStartupWarmupOperations {
    async fn warm_runtime(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        self.steps
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push("runtime");
        observer.on_phase(HarnessSetupPhase::MachineCheck, "warming runtime");
        Ok(())
    }

    async fn warm_runtime_launch_ready(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.runtime_runs.fetch_add(1, Ordering::SeqCst);
        self.steps
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push("launch_ready");
        observer.on_phase(
            HarnessSetupPhase::MachineStartOrInit,
            "warming launch-ready runtime",
        );
        Ok(())
    }

    async fn warm_builder(&self, _observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        Ok(())
    }
}

struct BlockingSandboxCliLoadWarmupOperations {
    data_root: PathBuf,
    image_tar: PathBuf,
}

#[async_trait]
impl SharedWarmupOperations for BlockingSandboxCliLoadWarmupOperations {
    async fn warm_runtime(
        &self,
        _settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        observer.on_phase(
            HarnessSetupPhase::ImageLoad,
            "loading harness image into local sandbox runtime",
        );
        let mut cmd = ctx_harness_runtime::sandbox_container_command(&self.data_root)?;
        cmd.arg("load").arg("-i").arg(&self.image_tar);
        let output =
            crate::workspace_runtime::command_output_with_timeout(cmd, Duration::from_secs(60))
                .await?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let combined = format!("{stderr}\n{stdout}").trim().to_string();
        anyhow::bail!(
            "sandbox CLI load failed: {}",
            if combined.is_empty() {
                format!("status {}", output.status)
            } else {
                combined
            }
        );
    }

    async fn warm_runtime_launch_ready(
        &self,
        settings: ExecutionSettings,
        observer: Arc<dyn HarnessSetupObserver>,
    ) -> Result<()> {
        self.warm_runtime(settings, observer).await
    }

    async fn warm_builder(&self, _observer: Arc<dyn HarnessSetupObserver>) -> Result<()> {
        Ok(())
    }
}

#[test]
fn needs_prewarm_gate_matches_truth_table() {
    assert!(needs_prewarm(false, false, false, false));
    assert!(needs_prewarm(true, false, false, false));
    assert!(needs_prewarm(true, true, true, false));
    assert!(needs_prewarm(true, true, false, true));
    assert!(!needs_prewarm(true, true, false, false));
}

#[test]
fn normalize_container_engine_ready_for_gate_treats_missing_cli_as_not_ready() {
    let value = normalize_container_engine_ready_for_gate(Err(anyhow::anyhow!(
        "native sandbox container runtime is unavailable"
    )))
    .expect("missing binary should map to not-ready");
    assert!(!value);
}

#[test]
fn normalize_container_engine_ready_for_gate_preserves_other_errors() {
    let err = normalize_container_engine_ready_for_gate(Err(anyhow::anyhow!("boom")));
    assert!(err.is_err());
}

#[test]
fn launch_job_logs_are_bounded_and_ordered() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    for i in 0..(JOB_LOG_CAP + 32) {
        let msg = format!("line {i}");
        let _ = job.push_log(
            HarnessSetupPhase::MachineCheck,
            HarnessSetupLogLevel::Info,
            &msg,
        );
    }
    let snapshot = job.snapshot();
    assert_eq!(snapshot.logs.len(), JOB_LOG_CAP);
    let first = snapshot.logs.first().expect("missing first line");
    let last = snapshot.logs.last().expect("missing last line");
    assert!(first.seq < last.seq);
    let mut prev = first.seq;
    for line in snapshot.logs.iter().skip(1) {
        assert!(line.seq > prev);
        prev = line.seq;
    }
}

#[test]
fn phase_transition_closes_previous_phase() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    let _ = job.transition_phase(HarnessSetupPhase::MachineCheck, "check");
    let _ = job.transition_phase(HarnessSetupPhase::ImageCheck, "image");
    let snapshot = job.snapshot();
    assert_eq!(snapshot.current_phase, Some(HarnessSetupPhase::ImageCheck));
    assert_eq!(snapshot.phases.len(), 2);
    assert!(snapshot.phases[0].finished_at.is_some());
    assert!(snapshot.phases[0].elapsed_ms.is_some());
    assert!(snapshot.phases[1].finished_at.is_none());
}

#[test]
fn duplicate_phase_updates_do_not_append_duplicate_logs() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    let _ = job.transition_phase(
        HarnessSetupPhase::ArtifactDownload,
        "downloading required artifacts",
    );
    let _ = job.transition_phase(
        HarnessSetupPhase::ArtifactDownload,
        "downloading required artifacts",
    );
    let snapshot = job.snapshot();
    assert_eq!(snapshot.logs.len(), 1);
    assert_eq!(
        snapshot.current_step_label.as_deref(),
        Some("downloading required artifacts")
    );
}

#[test]
fn launch_snapshot_projects_download_eta_and_progress() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    let _ = job.transition_phase(
        HarnessSetupPhase::ArtifactDownload,
        "downloading required artifacts",
    );
    let _ = job.set_progress(HarnessSetupProgressUpdate {
        phase: HarnessSetupPhase::ArtifactDownload,
        active_download: Some(HarnessSetupDownloadStatus {
            artifact: "Required artifacts".to_string(),
            downloaded_bytes: 400,
            total_bytes: Some(1000),
            bytes_per_sec: Some(100),
        }),
    });
    let snapshot = job.snapshot();
    assert_eq!(
        snapshot.current_phase,
        Some(HarnessSetupPhase::ArtifactDownload)
    );
    assert_eq!(
        snapshot.current_step_label.as_deref(),
        Some("downloading required artifacts")
    );
    assert!(snapshot.eta_ms.unwrap_or(0) >= 39_000);
    assert!(snapshot.progress_pct.unwrap_or(0) < 100);
    assert_eq!(
        snapshot
            .active_download
            .as_ref()
            .and_then(|value| value.total_bytes),
        Some(1000)
    );
}

#[test]
fn launch_snapshot_drops_expired_non_download_eta() {
    let job = LaunchJob::new(uuid::Uuid::new_v4().to_string(), WorkspaceId::new());
    let _ = job.transition_phase(
        HarnessSetupPhase::ImageLoad,
        "loading harness image into local sandbox runtime",
    );
    {
        let mut inner = lock_or_recover(&job.inner, "launch job");
        if let Some(phase) = inner.phases.last_mut() {
            phase.started_at -= chrono::TimeDelta::milliseconds(6_000);
        }
    }
    let snapshot = job.snapshot();
    assert_eq!(snapshot.current_phase, Some(HarnessSetupPhase::ImageLoad));
    assert_eq!(snapshot.eta_ms, None);
}

#[test]
fn format_error_chain_includes_context_and_cause() {
    let err = anyhow::anyhow!("inner").context("outer");
    assert_eq!(format_error_chain(&err), "outer: inner");
}

#[test]
fn format_error_chain_marks_empty_cause() {
    #[derive(Debug)]
    struct EmptyCause;
    impl std::fmt::Display for EmptyCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "")
        }
    }
    impl std::error::Error for EmptyCause {}

    let err = anyhow::Error::new(EmptyCause).context("container runtime failed");
    assert_eq!(
        format_error_chain(&err),
        "container runtime failed: <empty error cause>"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn concurrent_launch_start_is_deduplicated() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let sandbox_cli_path =
        write_running_container_sandbox_cli_shim(data_dir.path(), &log_path, &container_name);
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(UnexpectedRuntimeWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    let barrier = Arc::new(Barrier::new(3));

    let coordinator_a = Arc::clone(&coordinator);
    let workspace_a = workspace.clone();
    let settings_a = settings.clone();
    let barrier_a = Arc::clone(&barrier);
    let start_a = tokio::spawn(async move {
        barrier_a.wait().await;
        coordinator_a
            .start_workspace_launch(workspace_a, settings_a, "http://127.0.0.1:4399".to_string())
            .await
    });

    let coordinator_b = Arc::clone(&coordinator);
    let workspace_b = workspace.clone();
    let settings_b = settings.clone();
    let barrier_b = Arc::clone(&barrier);
    let start_b = tokio::spawn(async move {
        barrier_b.wait().await;
        coordinator_b
            .start_workspace_launch(workspace_b, settings_b, "http://127.0.0.1:4399".to_string())
            .await
    });

    barrier.wait().await;
    let first = start_a.await.expect("first launch task failed");
    let second = start_b.await.expect("second launch task failed");
    assert_eq!(first.job_id, second.job_id);
    let workspace_id = workspace.id.0.to_string();
    assert_eq!(first.workspace_id, workspace_id);
    assert_eq!(second.workspace_id, workspace_id);

    TrackedExecutionLaunch::new(&coordinator, first.clone())
        .wait_ready(Duration::from_secs(10))
        .await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    let exists_line = format!("container inspect {container_name}");
    let inspect_line =
        format!("container inspect --format {{{{.State.Running}}}} {container_name}");
    assert_eq!(
        log.matches(&exists_line).count(),
        1,
        "expected exactly one existing-container check in log:\n{log}"
    );
    assert_eq!(
        log.matches(&inspect_line).count(),
        1,
        "expected exactly one running-container inspect in log:\n{log}"
    );
    assert!(
        !log.contains("image inspect"),
        "deduplicated running-container launch should not front-load image checks:\n{log}"
    );
    assert!(
            !log.contains(&format!("start {container_name}")),
            "deduplicated running-container launch should not restart an already running container:\n{log}"
        );
    assert!(
        !log.contains("run -d --name"),
        "deduplicated running-container launch should not create a new container:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn startup_prewarm_runs_runtime_warmup_for_cold_container_settings() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
            &sandbox_cli_path,
            "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nexit 0\n",
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: sandbox_container_settings(),
    };
    save_test_execution_settings(data_dir.path(), settings).await;

    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let coordinator_task = Arc::clone(&coordinator);
    let startup = tokio::spawn(async move {
        run_startup_prewarm_from_store(&coordinator_task).await;
    });

    ops.wait_for_runtime_runs(1).await;

    let running = coordinator.startup_status().await;
    assert_eq!(running.state, StartupPrewarmState::Running);
    assert!(!running.machine_ready);

    ops.release_runtime();
    startup.await.expect("startup prewarm task");

    let ready = coordinator.startup_status().await;
    assert_eq!(ready.state, StartupPrewarmState::Ready);
    assert!(ready.needs_prewarm);
    assert!(!ready.machine_ready);
    assert!(!ready.image_present);
}

#[cfg(unix)]
#[tokio::test]
async fn spawned_startup_prewarm_respects_sandbox_cli_env_test_lock() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let serial = env_var_test_lock().lock().await;
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    save_test_execution_settings(data_dir.path(), sandbox_execution_settings()).await;
    let coordinator = test_coordinator(data_dir.path().to_path_buf());

    spawn_startup_prewarm_from_store(&coordinator).await;
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    let startup = coordinator.startup_status().await;
    assert!(
        startup.last_attempt_at.is_none(),
        "startup prewarm should not begin while the sandbox CLI env test lock is held: {startup:?}"
    );

    drop(serial);

    let terminal = wait_for_startup_prewarm_terminal(&coordinator, Duration::from_secs(30)).await;
    assert!(
        terminal.last_attempt_at.is_some(),
        "startup prewarm should begin once the env lock is released: {terminal:?}"
    );
}

#[tokio::test]
async fn startup_prewarm_backfills_metadata_when_runtime_is_already_ready() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    save_test_execution_settings(data_dir.path(), sandbox_execution_settings()).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    run_startup_prewarm_with_timeout(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(!snapshot.needs_prewarm);
    assert!(snapshot.machine_ready);
    assert!(snapshot.image_present);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read prewarm metadata")
        .expect("expected prewarm metadata");
    assert_eq!(
        metadata.image_ref,
        crate::workspace_runtime::default_container_image()
    );
    assert_eq!(metadata.bundled_image_fingerprint, None);
    assert_eq!(
        snapshot.last_success_at.as_deref(),
        Some(metadata.ready_at.as_str())
    );
}

#[tokio::test]
async fn startup_prewarm_preserves_existing_ready_timestamp_when_reusing_ready_runtime() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    save_test_execution_settings(data_dir.path(), sandbox_execution_settings()).await;
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: crate::workspace_runtime::default_container_image().to_string(),
            bundled_image_fingerprint: None,
            ready_at: "2026-03-20T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write existing prewarm metadata");

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    run_startup_prewarm_with_timeout(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(!snapshot.needs_prewarm);
    assert_eq!(
        snapshot.last_success_at.as_deref(),
        Some("2026-03-20T00:00:00Z")
    );
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read prewarm metadata")
        .expect("expected prewarm metadata");
    assert_eq!(metadata.ready_at, "2026-03-20T00:00:00Z");
}

#[tokio::test]
async fn startup_prewarm_reuses_initial_runtime_probe_for_gate_checks() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let (sandbox_cli_path, log_path) =
        write_logging_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    save_test_execution_settings(data_dir.path(), sandbox_execution_settings()).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    run_startup_prewarm_with_timeout(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(!snapshot.needs_prewarm);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert_eq!(
        count_matching_lines(&log, "info"),
        1,
        "expected one engine readiness probe:\n{log}"
    );
    assert_eq!(
        count_matching_lines(&log, "image inspect"),
        1,
        "expected one image presence probe:\n{log}"
    );
}

#[tokio::test]
async fn startup_prewarm_keeps_existing_metadata_when_machine_stays_down() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_startup_prewarm_sandbox_cli_shim(data_dir.path());
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let image = crate::workspace_runtime::default_container_image();
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: image.to_string(),
            bundled_image_fingerprint: Some("existing-fingerprint".to_string()),
            ready_at: "2026-03-18T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write existing prewarm metadata");

    let settings = sandbox_execution_settings();
    save_test_execution_settings(data_dir.path(), settings).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    run_startup_prewarm_from_store(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(snapshot.needs_prewarm);
    assert!(!snapshot.machine_ready);
    assert!(!snapshot.image_present);
    assert!(snapshot.last_success_at.is_none());
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read prewarm metadata")
        .expect("expected prewarm metadata to remain");
    assert_eq!(metadata.image_ref, image);
    assert_eq!(
        metadata.bundled_image_fingerprint.as_deref(),
        Some("existing-fingerprint")
    );
    assert_eq!(metadata.ready_at, "2026-03-18T00:00:00Z");
}

#[cfg(unix)]
#[tokio::test]
async fn startup_prewarm_does_not_record_success_for_stale_loaded_default_image() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let bundle_dir = data_dir.path().join("bundle");
    let bundle_images_dir = bundle_dir.join("images");
    std::fs::create_dir_all(&bundle_images_dir).expect("create bundle images dir");
    let tar_path = bundle_images_dir.join("ctx-harness.tar");
    std::fs::write(&tar_path, b"bundle image tar").expect("write bundled image tar");
    let default_image = crate::workspace_runtime::default_container_image();
    let manifest = serde_json::json!({
        "version": 1,
        "providers": [],
        "runtimes": [],
        "images": [{
            "id": "ctx-harness",
            "version": "test",
            "os": "linux",
            "arch": std::env::consts::ARCH,
            "sha256": "test-sha",
            "tar": "images/ctx-harness.tar",
            "image": default_image,
        }],
    });
    std::fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write bundle manifest");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        "#!/bin/sh\nif [ \"$1\" = \"info\" ]; then\n  printf '{}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let (_image_guard, _image_server) =
        install_test_managed_harness_image_source(vec![4, 5, 6]).await;

    let current_fingerprint = bundle_tar_fingerprint(&tar_path);
    let stale_fingerprint = format!("{current_fingerprint}-stale");
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: default_image.to_string(),
            bundled_image_fingerprint: Some(stale_fingerprint.clone()),
            ready_at: "2026-03-19T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write stale prewarm metadata");

    let settings = sandbox_execution_settings();
    save_test_execution_settings(data_dir.path(), settings).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    run_startup_prewarm_from_store(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Skipped);
    assert!(snapshot.needs_prewarm);
    assert!(snapshot.machine_ready);
    assert!(snapshot.image_present);
    assert!(snapshot.bundled_image_digest_changed);
    assert!(snapshot.last_success_at.is_none());
    assert!(snapshot
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("loaded harness image is still stale"));
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
        vec!["runtime"]
    );

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read prewarm metadata")
        .expect("expected stale metadata to remain");
    assert_eq!(metadata.bundled_image_fingerprint, Some(stale_fingerprint));
    assert_eq!(metadata.ready_at, "2026-03-19T00:00:00Z");
}

#[cfg(unix)]
#[tokio::test]
async fn successful_workspace_launch_writes_missing_prewarm_metadata() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let bundle_dir = data_dir.path().join("bundle");
    let bundle_images_dir = bundle_dir.join("images");
    std::fs::create_dir_all(&bundle_images_dir).expect("create bundle images dir");
    let tar_path = bundle_images_dir.join("ctx-harness.tar");
    std::fs::write(&tar_path, b"bundle image tar").expect("write bundled image tar");
    let default_image = crate::workspace_runtime::default_container_image();
    let manifest = serde_json::json!({
        "version": 1,
        "providers": [],
        "runtimes": [],
        "images": [{
            "id": "ctx-harness",
            "version": "test",
            "os": "linux",
            "arch": std::env::consts::ARCH,
            "sha256": "test-sha",
            "tar": "images/ctx-harness.tar",
            "image": default_image,
        }],
    });
    std::fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write bundle manifest");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

    let _current_fingerprint = bundle_tar_fingerprint(&tar_path);
    let expected_runtime_fingerprint = bundled_image_fingerprint(default_image)
        .await
        .expect("compute expected runtime fingerprint");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let machine_started = data_dir.path().join("machine-started");
    let image_present = data_dir.path().join("image-present");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nSTARTED=\"{started}\"\nIMAGE_PRESENT=\"{image_present}\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  : > \"$IMAGE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            started = machine_started.display(),
            image_present = image_present.display(),
            container = container_name,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let (_cache_guard, _cache_server) =
        install_test_managed_machine_cache_source(vec![1, 2, 3]).await;
    let (_image_guard, _image_server) =
        install_test_managed_harness_image_source(vec![4, 5, 6]).await;

    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let launch = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;
    assert_eq!(launch_terminal.state, ExecutionLaunchState::Ready);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read refreshed prewarm metadata")
        .expect("expected refreshed metadata");
    assert_eq!(metadata.image_ref, default_image);
    assert_eq!(
        metadata.bundled_image_fingerprint,
        expected_runtime_fingerprint
    );

    let startup = coordinator.startup_status().await;
    assert_eq!(startup.state, StartupPrewarmState::Ready);
    assert_eq!(startup.target_image, default_image);
    assert!(!startup.needs_prewarm);
    assert!(startup.machine_ready);
    assert!(startup.image_present);
    assert!(!startup.bundled_image_digest_changed);
}

#[cfg(unix)]
#[tokio::test]
async fn successful_workspace_launch_refresh_clears_stale_prewarm_metadata() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let bundle_dir = data_dir.path().join("bundle");
    let bundle_images_dir = bundle_dir.join("images");
    std::fs::create_dir_all(&bundle_images_dir).expect("create bundle images dir");
    let tar_path = bundle_images_dir.join("ctx-harness.tar");
    std::fs::write(&tar_path, b"bundle image tar").expect("write bundled image tar");
    let default_image = crate::workspace_runtime::default_container_image();
    let manifest = serde_json::json!({
        "version": 1,
        "providers": [],
        "runtimes": [],
        "images": [{
            "id": "ctx-harness",
            "version": "test",
            "os": "linux",
            "arch": std::env::consts::ARCH,
            "sha256": "test-sha",
            "tar": "images/ctx-harness.tar",
            "image": default_image,
        }],
    });
    std::fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write bundle manifest");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

    let current_fingerprint = bundle_tar_fingerprint(&tar_path);
    let stale_fingerprint = format!("{current_fingerprint}-stale");
    let expected_runtime_fingerprint = bundled_image_fingerprint(default_image)
        .await
        .expect("compute expected runtime fingerprint");
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: default_image.to_string(),
            bundled_image_fingerprint: Some(stale_fingerprint.clone()),
            ready_at: "2026-03-19T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write stale prewarm metadata");

    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let machine_started = data_dir.path().join("machine-started");
    let image_present = data_dir.path().join("image-present");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nLOG=\"{log}\"\nSTARTED=\"{started}\"\nIMAGE_PRESENT=\"{image_present}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  : > \"$IMAGE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            started = machine_started.display(),
            image_present = image_present.display(),
            container = container_name,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let (_cache_guard, _cache_server) =
        install_test_managed_machine_cache_source(vec![1, 2, 3]).await;
    let (_image_guard, _image_server) =
        install_test_managed_harness_image_source(vec![4, 5, 6]).await;

    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

    run_startup_prewarm_from_store(&coordinator).await;
    let startup = coordinator.startup_status().await;
    assert!(startup.bundled_image_digest_changed);

    let launch = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;
    assert_eq!(launch_terminal.state, ExecutionLaunchState::Ready);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read refreshed prewarm metadata")
        .expect("expected refreshed metadata");
    assert_eq!(metadata.image_ref, default_image);
    assert_eq!(
        metadata.bundled_image_fingerprint,
        expected_runtime_fingerprint
    );
    assert_ne!(metadata.ready_at, "2026-03-19T00:00:00Z");

    run_startup_prewarm_from_store(&coordinator).await;
    let refreshed = coordinator.startup_status().await;
    assert_eq!(refreshed.state, StartupPrewarmState::Ready);
    assert!(!refreshed.needs_prewarm);
    assert!(refreshed.machine_ready);
    assert!(refreshed.image_present);
    assert!(!refreshed.bundled_image_digest_changed);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(log.contains("load -i"));
}

#[cfg(unix)]
#[tokio::test]
async fn successful_workspace_launch_refreshes_prewarm_metadata_when_image_ref_changes() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let bundle_dir = data_dir.path().join("bundle");
    let bundle_images_dir = bundle_dir.join("images");
    std::fs::create_dir_all(&bundle_images_dir).expect("create bundle images dir");
    let tar_path = bundle_images_dir.join("ctx-harness.tar");
    std::fs::write(&tar_path, b"bundle image tar").expect("write bundled image tar");
    let default_image = crate::workspace_runtime::default_container_image();
    let manifest = serde_json::json!({
        "version": 1,
        "providers": [],
        "runtimes": [],
        "images": [{
            "id": "ctx-harness",
            "version": "test",
            "os": "linux",
            "arch": std::env::consts::ARCH,
            "sha256": "test-sha",
            "tar": "images/ctx-harness.tar",
            "image": default_image,
        }],
    });
    std::fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write bundle manifest");
    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());

    let expected_runtime_fingerprint = bundled_image_fingerprint(default_image)
        .await
        .expect("compute expected runtime fingerprint");
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: "ghcr.io/ctxrs/ctx-harness:old".to_string(),
            bundled_image_fingerprint: expected_runtime_fingerprint.clone(),
            ready_at: "2026-03-19T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write stale image-ref metadata");

    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let machine_started = data_dir.path().join("machine-started");
    let image_present = data_dir.path().join("image-present");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nSTARTED=\"{started}\"\nIMAGE_PRESENT=\"{image_present}\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  : > \"$IMAGE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            started = machine_started.display(),
            image_present = image_present.display(),
            container = container_name,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let (_cache_guard, _cache_server) =
        install_test_managed_machine_cache_source(vec![1, 2, 3]).await;
    let (_image_guard, _image_server) =
        install_test_managed_harness_image_source(vec![4, 5, 6]).await;

    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let launch = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;
    assert_eq!(launch_terminal.state, ExecutionLaunchState::Ready);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read refreshed prewarm metadata")
        .expect("expected refreshed metadata");
    assert_eq!(metadata.image_ref, default_image);
    assert_eq!(
        metadata.bundled_image_fingerprint,
        expected_runtime_fingerprint
    );
    assert_ne!(metadata.ready_at, "2026-03-19T00:00:00Z");

    let startup = coordinator.startup_status().await;
    assert_eq!(startup.state, StartupPrewarmState::Ready);
    assert_eq!(startup.target_image, default_image);
    assert!(!startup.image_ref_changed);
    assert!(!startup.needs_prewarm);
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_override_image_does_not_clobber_startup_prewarm_metadata() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");

    let default_image = crate::workspace_runtime::default_container_image();
    write_prewarm_metadata(
        data_dir.path(),
        &StartupPrewarmMetadata {
            image_ref: default_image.to_string(),
            bundled_image_fingerprint: Some("existing-fingerprint".to_string()),
            ready_at: "2026-03-19T00:00:00Z".to_string(),
        },
    )
    .await
    .expect("write baseline prewarm metadata");

    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let machine_started = data_dir.path().join("machine-started");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    let override_image = "ghcr.io/ctxrs/custom-harness:test";
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nSTARTED=\"{started}\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{override_image}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            started = machine_started.display(),
            container = container_name,
            override_image = override_image,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let (_cache_guard, _cache_server) =
        install_test_managed_machine_cache_source(vec![1, 2, 3]).await;

    save_test_execution_settings(
        data_dir.path(),
        ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                network_mode: crate::settings::ContainerNetworkMode::All,
                ..sandbox_container_settings()
            },
        },
    )
    .await;

    let launch_settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            image: Some(override_image.to_string()),
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };

    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let launch = coordinator
        .start_workspace_launch(
            workspace,
            launch_settings,
            "http://127.0.0.1:4399".to_string(),
        )
        .await;
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;
    assert_eq!(launch_terminal.state, ExecutionLaunchState::Ready);

    let metadata = read_prewarm_metadata(data_dir.path())
        .await
        .expect("read prewarm metadata")
        .expect("expected prewarm metadata");
    assert_eq!(metadata.image_ref, default_image);
    assert_eq!(
        metadata.bundled_image_fingerprint.as_deref(),
        Some("existing-fingerprint")
    );
    assert_eq!(metadata.ready_at, "2026-03-19T00:00:00Z");
}

#[tokio::test]
async fn runtime_prewarm_reuses_background_all_job_and_waits_for_builder_tail_when_runtime_joins() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let background = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::All)
        .await;
    ops.wait_for_runtime_runs(1).await;

    let foreground = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::Runtime)
        .await;

    assert_eq!(foreground.job_id, background.job_id);
    {
        let inner = coordinator.inner.lock().await;
        assert_eq!(inner.launch_jobs.len(), 1);
        assert_eq!(inner.launch_history.len(), 1);
    }

    ops.release_runtime();
    ops.wait_for_builder_runs(1).await;

    let still_running = coordinator
        .launch_status(&background.job_id)
        .await
        .expect("missing shared prewarm job");
    assert_eq!(still_running.state, ExecutionLaunchState::Running);

    ops.release_builder();

    let ready = tokio::time::timeout(BACKGROUND_TEST_TIMEOUT, async {
        loop {
            let latest = coordinator
                .launch_status(&background.job_id)
                .await
                .expect("missing shared prewarm job");
            if latest.state == ExecutionLaunchState::Ready {
                break latest;
            }
            if latest.state == ExecutionLaunchState::Error {
                panic!(
                    "shared all/builder prewarm failed unexpectedly: error={:?}, logs={:?}",
                    latest.error, latest.logs
                );
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shared all prewarm wait timed out");

    assert_eq!(
        ready.state,
        ExecutionLaunchState::Ready,
        "expected shared all prewarm to finish ready, got terminal snapshot: {ready:#?}"
    );
    assert_eq!(ready.job_id, background.job_id);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn runtime_prewarm_errors_when_only_startup_artifacts_were_warmed() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_startup_prewarm_sandbox_cli_shim(data_dir.path());
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let snapshot = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::Runtime)
        .await;
    let terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, Duration::from_secs(5))
            .await;

    assert_eq!(terminal.state, ExecutionLaunchState::Error);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert!(terminal
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("still needs first-launch startup"));
}

#[tokio::test]
async fn runtime_prewarm_launch_ready_request_reuses_running_runtime_job_and_promotes_scope() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let background = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::Runtime)
        .await;
    ops.wait_for_runtime_runs(1).await;

    let promoted = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::LaunchReady)
        .await;

    assert_eq!(promoted.job_id, background.job_id);
    {
        let inner = coordinator.inner.lock().await;
        assert_eq!(inner.launch_jobs.len(), 1);
        assert_eq!(inner.launch_history.len(), 1);
    }

    ops.release_runtime();
    ops.wait_for_launch_ready_runs(1).await;

    let still_running = coordinator
        .launch_status(&background.job_id)
        .await
        .expect("missing promoted prewarm job");
    assert_eq!(still_running.state, ExecutionLaunchState::Running);

    ops.release_runtime();

    let ready = wait_for_execution_launch_terminal(
        &coordinator,
        &background.job_id,
        BACKGROUND_TEST_TIMEOUT,
    )
    .await;

    assert_eq!(ready.state, ExecutionLaunchState::Ready);
    assert!(ready.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::Ready
            && line.message == "local sandbox runtime and launch image are ready"
    }));
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 2);
    assert_eq!(ops.launch_ready_runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn compute_prewarm_gate_keeps_prefetched_avf_runtime_unready_without_vm_boot() {
    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let helper_path = write_avf_linux_lifecycle_helper(data_dir.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _helper = EnvVarGuard::set(
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        crate::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_fixture, servers) = install_test_managed_avf_linux_runtime_source().await;
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            runtime: ContainerRuntimeKind::SharedVmContainer,
            ..Default::default()
        },
    };
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    ctx_harness_runtime::prewarm_selected_runtime_with_observer(
        data_dir.path(),
        &settings.container,
        None,
    )
    .await
    .expect("prewarm AVF runtime");

    let gate = coordinator
        .compute_prewarm_gate(&settings.container)
        .await
        .expect("compute AVF prewarm gate");
    assert!(!gate.machine_ready);
    assert!(!gate.image_present);
    assert!(!gate.image_ref_changed);
    assert!(!gate.bundled_image_digest_changed);
    assert!(gate.needs_prewarm);
    assert_eq!(gate.bundled_image_fingerprint, None);

    let runtime_state = coordinator
        .startup_runtime_state(&settings.container)
        .await
        .expect("read AVF runtime state");
    assert_eq!(runtime_state, (false, false));

    for server in servers {
        server.abort();
    }
}

#[tokio::test]
async fn runtime_prewarm_runtime_scope_stays_substrate_only_for_avf_linux_runtime() {
    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let helper_path = write_avf_linux_lifecycle_helper(data_dir.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _helper = EnvVarGuard::set(
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        crate::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_fixture, servers) = install_test_managed_avf_linux_runtime_source().await;
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            runtime: ContainerRuntimeKind::SharedVmContainer,
            ..Default::default()
        },
    };

    let launch = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::Runtime)
        .await;
    let terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;

    assert_eq!(
        terminal.state,
        ExecutionLaunchState::Ready,
        "launch-ready prewarm failed: error={:?}, logs={:?}",
        terminal.error,
        terminal.logs
    );
    assert!(terminal.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::Ready
            && line.message
                == "shared VM runtime artifacts are ready; launch image loads when the shared VM starts"
    }));
    let runtime_state = coordinator
        .startup_runtime_state(&settings.container)
        .await
        .expect("read AVF startup state");
    assert_eq!(runtime_state, (false, false));
    let artifact_state =
        ctx_harness_runtime::selected_runtime_state(data_dir.path(), &settings.container)
            .await
            .expect("read AVF runtime artifact state");
    assert_eq!(artifact_state, (true, true));
    let launch_ready =
        ctx_harness_runtime::selected_runtime_launch_ready(data_dir.path(), &settings.container)
            .await
            .expect("read AVF launch-ready state");
    assert!(
        !launch_ready,
        "runtime scope should not boot the shared AVF VM"
    );

    for server in servers {
        server.abort();
    }
}

#[tokio::test]
async fn runtime_prewarm_launch_ready_scope_starts_shared_vm_and_reports_launch_ready() {
    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let helper_path = write_avf_linux_lifecycle_helper(data_dir.path());
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _helper = EnvVarGuard::set(
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        crate::workspace_runtime::CTX_HARNESS_SANDBOX_CLI_PATH_ENV,
        &sandbox_cli_path.to_string_lossy(),
    );
    let (_runtime_fixture, servers) = install_test_managed_avf_linux_runtime_source().await;
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            runtime: ContainerRuntimeKind::SharedVmContainer,
            ..Default::default()
        },
    };

    let launch = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::LaunchReady)
        .await;
    let terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;

    assert_eq!(
        terminal.state,
        ExecutionLaunchState::Ready,
        "AVF launch-ready prewarm failed: error={:?}, logs={:?}",
        terminal.error,
        terminal.logs
    );
    assert!(terminal.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::Ready
            && line.message == "shared VM substrate and launch image are ready"
    }));
    let launch_ready =
        ctx_harness_runtime::selected_runtime_launch_ready(data_dir.path(), &settings.container)
            .await
            .expect("read AVF launch-ready state");
    assert!(
        launch_ready,
        "AVF launch-ready prewarm should boot the shared VM"
    );
    let runtime_state = coordinator
        .startup_runtime_state(&settings.container)
        .await
        .expect("read AVF startup runtime state");
    assert_eq!(runtime_state, (true, true));
    let gate = coordinator
        .compute_prewarm_gate(&settings.container)
        .await
        .expect("compute AVF prewarm gate");
    assert!(gate.machine_ready);
    assert!(gate.image_present);
    assert!(!gate.needs_prewarm);

    for server in servers {
        server.abort();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_waits_for_running_startup_prewarm_without_duplicate_runtime_warmup() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let machine_started = data_dir.path().join("machine-started");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let (_cache_guard, _cache_server) =
        install_test_managed_machine_cache_source(vec![1, 2, 3]).await;
    std::fs::write(
            &sandbox_cli_path,
            with_native_runtime_ready(with_workspace_volume_support(format!(
                "#!/bin/sh\nLOG=\"{log}\"\nSTARTED=\"{started}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
                log = log_path.display(),
                started = machine_started.display(),
                container = container_name,
            ))),
        )
        .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let coordinator_task = Arc::clone(&coordinator);
    let startup = tokio::spawn(async move {
        run_startup_prewarm_from_store(&coordinator_task).await;
    });

    ops.wait_for_runtime_runs(1).await;

    let snapshot = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);

    ops.release_runtime();
    startup.await.expect("startup prewarm task");

    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let latest = coordinator
                .launch_status(&snapshot.job_id)
                .await
                .expect("missing workspace launch job");
            if latest.state == ExecutionLaunchState::Ready {
                break latest;
            }
            if latest.state == ExecutionLaunchState::Error {
                panic!("workspace launch failed unexpectedly: {:?}", latest.error);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for joined launch readiness");

    assert_eq!(ready.state, ExecutionLaunchState::Ready);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected reusable container check in log:\n{log}"
    );
    assert!(
        !log.contains("load -i"),
        "joined launch should not trigger a second image load for reusable containers:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_reuses_active_runtime_prewarm_without_second_image_load() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let image_tar = data_dir.path().join("ctx-harness.tar");
    std::fs::write(&image_tar, b"bundle image tar").expect("write image tar");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let load_release = data_dir.path().join("release-load");
    let image_present = data_dir.path().join("image-present");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nLOG=\"{log}\"\nLOAD_RELEASE=\"{load_release}\"\nIMAGE_PRESENT=\"{image_present}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  while [ ! -f \"$LOAD_RELEASE\" ]; do\n    sleep 0.05\n  done\n  : > \"$IMAGE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            load_release = load_release.display(),
            image_present = image_present.display(),
            container = container_name,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let coordinator = test_coordinator_with_operations(
        data_dir.path().to_path_buf(),
        Arc::new(BlockingSandboxCliLoadWarmupOperations {
            data_root: data_dir.path().to_path_buf(),
            image_tar,
        }),
    );
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };

    let prewarm = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::Runtime)
        .await;

    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            if log.contains("load -i") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for runtime prewarm sandbox CLI load");

    let launch = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;

    let running = coordinator
        .launch_status(&launch.job_id)
        .await
        .expect("missing workspace launch");
    assert_eq!(running.state, ExecutionLaunchState::Running);

    tokio::time::sleep(Duration::from_millis(250)).await;
    std::fs::write(&load_release, b"ok").expect("release sandbox CLI load");

    let prewarm_terminal =
        wait_for_execution_launch_terminal(&coordinator, &prewarm.job_id, Duration::from_secs(5))
            .await;
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(5))
            .await;

    assert_eq!(
        prewarm_terminal.state,
        ExecutionLaunchState::Ready,
        "runtime prewarm failed: error={:?}, logs={:?}",
        prewarm_terminal.error,
        prewarm_terminal.logs
    );
    assert_eq!(
        launch_terminal.state,
        ExecutionLaunchState::Ready,
        "workspace launch failed: error={:?}, logs={:?}",
        launch_terminal.error,
        launch_terminal.logs
    );

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert_eq!(
        log.matches("load -i").count(),
        1,
        "workspace launch should join the active runtime prewarm instead of racing a second image load:\n{log}"
    );
    assert!(
        log.contains("run -d --name"),
        "workspace launch should still create the workspace container after the shared image warmup completes:\n{log}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_reuses_startup_prewarm_without_second_image_load_when_machine_is_ready() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let image_tar = data_dir.path().join("ctx-harness.tar");
    std::fs::write(&image_tar, b"bundle image tar").expect("write image tar");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let load_release = data_dir.path().join("release-load");
    let image_present = data_dir.path().join("image-present");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        with_native_runtime_ready(with_workspace_volume_support(format!(
            "#!/bin/sh\nLOG=\"{log}\"\nLOAD_RELEASE=\"{load_release}\"\nIMAGE_PRESENT=\"{image_present}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    exit 0\n  fi\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  while [ ! -f \"$LOAD_RELEASE\" ]; do\n    sleep 0.05\n  done\n  : > \"$IMAGE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  printf 'fake-container-id\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            load_release = load_release.display(),
            image_present = image_present.display(),
            container = container_name,
        ))),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let coordinator = test_coordinator_with_operations(
        data_dir.path().to_path_buf(),
        Arc::new(BlockingSandboxCliLoadWarmupOperations {
            data_root: data_dir.path().to_path_buf(),
            image_tar,
        }),
    );
    let coordinator_task = Arc::clone(&coordinator);
    let startup = tokio::spawn(async move {
        run_startup_prewarm_from_store(&coordinator_task).await;
    });

    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            if log.contains("load -i") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for startup prewarm sandbox CLI load");

    let running = coordinator.startup_status().await;
    assert_eq!(running.state, StartupPrewarmState::Running);
    assert!(
        running.machine_ready,
        "startup prewarm should expose the current machine-ready state while the shared image warmup is running"
    );

    let launch = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;

    let launch_running = coordinator
        .launch_status(&launch.job_id)
        .await
        .expect("missing workspace launch");
    assert_eq!(launch_running.state, ExecutionLaunchState::Running);

    tokio::time::sleep(Duration::from_millis(250)).await;
    std::fs::write(&load_release, b"ok").expect("release sandbox CLI load");

    startup.await.expect("startup prewarm task");
    let launch_terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(5))
            .await;

    assert_eq!(
        launch_terminal.state,
        ExecutionLaunchState::Ready,
        "workspace launch failed: error={:?}, logs={:?}",
        launch_terminal.error,
        launch_terminal.logs
    );

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert_eq!(
        log.matches("load -i").count(),
        1,
        "workspace launch should join the running startup prewarm instead of triggering a second image load:\n{log}"
    );
    assert!(
        log.contains("run -d --name"),
        "workspace launch should still create the workspace container after the startup prewarm completes:\n{log}"
    );
}

#[tokio::test]
async fn builder_prewarm_reuses_background_all_job() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let background = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::All)
        .await;
    ops.wait_for_runtime_runs(1).await;

    let builder_only = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::Builder)
        .await;

    assert_eq!(builder_only.job_id, background.job_id);
    {
        let inner = coordinator.inner.lock().await;
        assert_eq!(inner.launch_jobs.len(), 1);
        assert_eq!(inner.launch_history.len(), 1);
    }
    assert_eq!(
            ops.builder_runs.load(Ordering::SeqCst),
            0,
            "builder-only join should not start a second builder warmup before the shared all job reaches builder work"
        );

    ops.release_runtime();
    ops.wait_for_builder_runs(1).await;
    ops.release_builder();

    let ready = wait_for_execution_launch_terminal(
        &coordinator,
        &background.job_id,
        BACKGROUND_TEST_TIMEOUT,
    )
    .await;

    assert_eq!(ready.state, ExecutionLaunchState::Ready);
    assert_eq!(ready.job_id, background.job_id);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(ops.builder_runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn subscribe_launch_returns_terminal_snapshot_when_job_is_done() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let workspace_id = WorkspaceId::new();
    let job_id = uuid::Uuid::new_v4().to_string();
    let job = Arc::new(LaunchJob::new(job_id.clone(), workspace_id));

    {
        let mut inner = coordinator.inner.lock().await;
        inner
            .running_launch_by_workspace
            .insert(workspace_id, job_id.clone());
        inner.launch_jobs.insert(job_id.clone(), Arc::clone(&job));
        inner.launch_history.push_back(job_id.clone());
    }

    let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
    let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
        snapshot: terminal.snapshot,
    });
    coordinator
        .clear_running_launch(workspace_id, &job_id)
        .await;

    let (snapshot, mut rx) = coordinator
        .subscribe_launch(&job_id)
        .await
        .expect("missing launch job");
    assert_eq!(snapshot.state, ExecutionLaunchState::Ready);
    assert!(matches!(
        rx.try_recv(),
        Ok(ExecutionLaunchStreamEvent::LaunchComplete { .. })
            | Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn subscribe_launch_receiver_gets_terminal_event_after_running_snapshot() {
    let data_dir = tempfile::tempdir().expect("tempdir");
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let workspace_id = WorkspaceId::new();
    let job_id = uuid::Uuid::new_v4().to_string();
    let job = Arc::new(LaunchJob::new(job_id.clone(), workspace_id));

    {
        let mut inner = coordinator.inner.lock().await;
        inner
            .running_launch_by_workspace
            .insert(workspace_id, job_id.clone());
        inner.launch_jobs.insert(job_id.clone(), Arc::clone(&job));
        inner.launch_history.push_back(job_id.clone());
    }

    let (snapshot, mut rx) = coordinator
        .subscribe_launch(&job_id)
        .await
        .expect("missing launch job");
    assert_eq!(snapshot.state, ExecutionLaunchState::Running);

    let terminal = job.mark_terminal(ExecutionLaunchState::Ready, None);
    let _ = job.tx.send(ExecutionLaunchStreamEvent::LaunchComplete {
        snapshot: terminal.snapshot,
    });
    coordinator
        .clear_running_launch(workspace_id, &job_id)
        .await;

    let event = tokio::time::timeout(QUICK_ASYNC_TEST_TIMEOUT, rx.recv())
        .await
        .expect("timed out waiting for terminal launch event")
        .expect("launch event channel closed");
    assert!(matches!(
        event,
        ExecutionLaunchStreamEvent::LaunchComplete { .. }
    ));
}

#[tokio::test]
async fn runtime_prewarm_emits_initial_log_before_runtime_work_completes() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let snapshot = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::Runtime)
        .await;
    assert_eq!(
        snapshot.current_phase,
        Some(HarnessSetupPhase::MachineCheck)
    );
    assert!(snapshot.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::MachineCheck
            && line.message == "requesting shared container readiness"
    }));

    ops.wait_for_runtime_runs(1).await;
    let observed = coordinator
        .launch_status(&snapshot.job_id)
        .await
        .expect("missing launch job");

    assert!(matches!(
        observed.current_phase,
        Some(HarnessSetupPhase::MachineCheck) | None
    ));
    assert!(observed.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::MachineCheck
            && (line.message == "requesting shared container readiness"
                || line.message == "warming runtime")
    }));

    ops.release_runtime();
    let _terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, BACKGROUND_TEST_TIMEOUT)
            .await;
}

#[cfg(unix)]
#[cfg(target_os = "macos")]
#[tokio::test]
async fn runtime_prewarm_launch_ready_scope_starts_native_runtime_before_loading_image() {
    use std::os::unix::fs::PermissionsExt;

    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let bundle_dir = data_dir.path().join("bundle");
    let bundle_images_dir = bundle_dir.join("images");
    std::fs::create_dir_all(&bundle_images_dir).expect("create bundle images dir");
    let tar_path = bundle_images_dir.join("ctx-harness.tar");
    std::fs::write(&tar_path, b"bundle image tar").expect("write bundled image tar");
    let default_image = crate::workspace_runtime::default_container_image();
    let manifest = serde_json::json!({
        "version": 1,
        "providers": [],
        "runtimes": [],
        "images": [{
            "id": "ctx-harness",
            "version": "test",
            "os": "linux",
            "arch": std::env::consts::ARCH,
            "sha256": "test-sha",
            "tar": "images/ctx-harness.tar",
            "image": default_image,
        }],
    });
    std::fs::write(
        bundle_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
    )
    .expect("write bundle manifest");

    let machine_present = data_dir.path().join("machine-present");
    let machine_started = data_dir.path().join("machine-started");
    let image_present = data_dir.path().join("image-present");
    let log_path = data_dir.path().join("sandbox-cli.log");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    std::fs::write(
        &sandbox_cli_path,
        format!(
            "#!/bin/sh\nLOG=\"{log}\"\nMACHINE_PRESENT=\"{machine_present}\"\nSTARTED=\"{machine_started}\"\nIMAGE_PRESENT=\"{image_present}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  if [ -f \"$STARTED\" ]; then\n    printf '{{}}\\n'\n    exit 0\n  fi\n  echo 'sandbox runtime unreachable' >&2\n  exit 125\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$MACHINE_PRESENT\" ]; then\n    printf '[{{\"State\":\"stopped\",\"Resources\":{{\"Memory\":4096}}}}]\\n'\n    exit 0\n  fi\n  echo 'machine not found' >&2\n  exit 1\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"init\" ]; then\n  : > \"$MACHINE_PRESENT\"\n  exit 0\nfi\nif [ \"$1\" = \"machine\" ] && [ \"$2\" = \"start\" ]; then\n  : > \"$STARTED\"\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  if [ -f \"$IMAGE_PRESENT\" ]; then\n    printf '[{{}}]\\n'\n    exit 0\n  fi\n  echo 'image missing' >&2\n  exit 1\nfi\nif [ \"$1\" = \"load\" ] && [ \"$2\" = \"-i\" ]; then\n  : > \"$IMAGE_PRESENT\"\n  printf 'Loaded image: {image}\\n'\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            machine_present = machine_present.display(),
            machine_started = machine_started.display(),
            image_present = image_present.display(),
            image = default_image,
        ),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");

    let _bundle_dir = EnvVarGuard::set("CTX_BUNDLE_DIR", &bundle_dir.to_string_lossy());
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");

    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let settings = sandbox_execution_settings();
    let launch = coordinator
        .start_runtime_prewarm(settings.clone(), RuntimePrewarmScope::LaunchReady)
        .await;
    let terminal =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, Duration::from_secs(10))
            .await;

    assert_eq!(terminal.state, ExecutionLaunchState::Ready);
    assert!(terminal.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::Ready
            && line.message == "local sandbox runtime and launch image are ready"
    }));
    let launch_ready =
        ctx_harness_runtime::selected_runtime_launch_ready(data_dir.path(), &settings.container)
            .await
            .expect("read native launch-ready state");
    assert!(
        launch_ready,
        "launch-ready scope should start the sandbox runtime and load the image"
    );

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI log");
    assert!(
        log.contains("machine start "),
        "expected launch-ready prewarm to start the sandbox machine: {log}"
    );
    assert!(
        log.contains("load -i"),
        "expected launch-ready prewarm to load the harness image after runtime startup: {log}"
    );
}

#[tokio::test]
async fn builder_only_prewarm_skips_runtime_warmup_and_runtime_availability() {
    let _serial = env_var_test_lock().lock().await;
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "0");
    let data_dir = tempfile::tempdir().expect("tempdir");
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let settings = sandbox_execution_settings();

    let snapshot = coordinator
        .start_runtime_prewarm(settings, RuntimePrewarmScope::Builder)
        .await;

    ops.wait_for_builder_runs(1).await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);

    let running = coordinator
        .launch_status(&snapshot.job_id)
        .await
        .expect("missing builder-only prewarm job");
    assert_eq!(running.current_phase, Some(HarnessSetupPhase::ImageLoad));
    assert!(running.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::ImageLoad && line.message == "warming builder"
    }));

    ops.release_builder();

    let terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, BACKGROUND_TEST_TIMEOUT)
            .await;

    assert_eq!(terminal.state, ExecutionLaunchState::Ready);
    assert!(terminal.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::Ready && line.message == "container builder is ready"
    }));
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn startup_prewarm_enters_shared_runtime_warmup_when_machine_is_not_ready() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_startup_prewarm_sandbox_cli_shim(data_dir.path());
    let _test_sandbox_cli = EnvVarGuard::unset("CTX_TEST_SANDBOX_CLI_AVAILABLE");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    save_test_execution_settings(data_dir.path(), sandbox_execution_settings()).await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

    run_startup_prewarm_from_store(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(snapshot.needs_prewarm);
    assert!(!snapshot.machine_ready);
    assert!(!snapshot.image_present);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
        vec!["runtime"]
    );
}

#[tokio::test]
async fn startup_prewarm_uses_runtime_scope_for_sandbox_mode_avf_linux_runtime() {
    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let helper_path = write_avf_linux_lifecycle_helper(data_dir.path());
    let _helper = EnvVarGuard::set(
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );
    save_test_execution_settings(
        data_dir.path(),
        ExecutionSettings {
            mode: ExecutionMode::Sandbox,
            container: crate::settings::ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::SharedVmContainer,
                ..Default::default()
            },
        },
    )
    .await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

    run_startup_prewarm_from_store(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(snapshot.needs_prewarm);
    assert!(!snapshot.machine_ready);
    assert!(!snapshot.image_present);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
        vec!["runtime"]
    );
}

#[tokio::test]
async fn startup_prewarm_uses_runtime_scope_for_host_mode_avf_linux_runtime() {
    let _process_env = process_env_test_lock().lock().await;
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let helper_path = write_avf_linux_lifecycle_helper(data_dir.path());
    let _helper = EnvVarGuard::set(
        ctx_avf_linux_runtime::AVF_LINUX_HELPER_PATH_ENV,
        &helper_path.to_string_lossy(),
    );
    save_test_execution_settings(
        data_dir.path(),
        ExecutionSettings {
            mode: ExecutionMode::Host,
            container: crate::settings::ContainerExecutionSettings {
                runtime: ContainerRuntimeKind::SharedVmContainer,
                ..Default::default()
            },
        },
    )
    .await;

    let ops = Arc::new(RecordingStartupWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());

    run_startup_prewarm_from_store(&coordinator).await;

    let snapshot = coordinator.startup_status().await;
    assert_eq!(snapshot.state, StartupPrewarmState::Ready);
    assert!(snapshot.needs_prewarm);
    assert!(!snapshot.machine_ready);
    assert!(!snapshot.image_present);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        *ops.steps.lock().unwrap_or_else(|err| err.into_inner()),
        vec!["runtime"]
    );
}

#[tokio::test]
async fn workspace_launch_is_not_blocked_by_background_runtime_prewarm_job() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let sandbox_cli_path = write_ready_runtime_sandbox_cli_shim(data_dir.path());
    let _sandbox_cli = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");
    let _sandbox_cli_path = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let ops = Arc::new(BlockingWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let prewarm_settings = sandbox_execution_settings();
    let workspace = test_workspace(WorkspaceId::new());
    let host_settings = ExecutionSettings {
        mode: ExecutionMode::Host,
        ..ExecutionSettings::default()
    };

    let background = coordinator
        .start_runtime_prewarm(prewarm_settings, RuntimePrewarmScope::Runtime)
        .await;
    ops.wait_for_runtime_runs(1).await;

    let launch = coordinator
        .start_workspace_launch(
            workspace.clone(),
            host_settings,
            "http://127.0.0.1:4399".to_string(),
        )
        .await;

    let ready =
        wait_for_execution_launch_terminal(&coordinator, &launch.job_id, BACKGROUND_TEST_TIMEOUT)
            .await;

    let background_snapshot = coordinator
        .launch_status(&background.job_id)
        .await
        .expect("missing background prewarm job");
    assert_eq!(background_snapshot.state, ExecutionLaunchState::Running);
    assert_eq!(ready.state, ExecutionLaunchState::Ready);
    assert_ne!(background.job_id, launch.job_id);

    ops.release_runtime();

    let background_terminal = wait_for_execution_launch_terminal(
        &coordinator,
        &background.job_id,
        BACKGROUND_TEST_TIMEOUT,
    )
    .await;
    assert_eq!(background_terminal.state, ExecutionLaunchState::Ready);
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_reuses_existing_container_without_waiting_for_startup_prewarm() {
    use std::os::unix::fs::PermissionsExt;

    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let sandbox_cli_path = data_dir.path().join("sandbox-cli.sh");
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    std::fs::write(
        &sandbox_cli_path,
        with_workspace_volume_support(format!(
            "#!/bin/sh\nLOG=\"{log}\"\nprintf '%s\\n' \"$*\" >> \"$LOG\"\nif [ \"$1\" = \"info\" ]; then\n  printf '{{}}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  exit 1\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$3\" = \"{container}\" ]; then\n  exit 0\nfi\nif [ \"$1\" = \"container\" ] && [ \"$2\" = \"inspect\" ] && [ \"$5\" = \"{container}\" ]; then\n  printf 'true\\n'\n  exit 0\nfi\nif [ \"$1\" = \"exec\" ]; then\n  exit 0\nfi\necho \"unexpected sandbox CLI invocation: $*\" >&2\nexit 1\n",
            log = log_path.display(),
            container = container_name,
        )),
    )
    .expect("write sandbox CLI shim");
    std::fs::set_permissions(&sandbox_cli_path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod sandbox CLI shim");
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );
    let _sandbox_cli_available = EnvVarGuard::set("CTX_TEST_SANDBOX_CLI_AVAILABLE", "1");

    let ops = Arc::new(BlockingWarmupOperations::default());
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };
    save_test_execution_settings(data_dir.path(), settings.clone()).await;

    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let coordinator_task = Arc::clone(&coordinator);
    let startup = tokio::spawn(async move {
        run_startup_prewarm_from_store(&coordinator_task).await;
    });

    ops.wait_for_runtime_runs(1).await;
    assert_eq!(
        coordinator.startup_status().await.state,
        StartupPrewarmState::Running
    );

    let launch = TrackedExecutionLaunch::new(
        &coordinator,
        coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await,
    );

    let ready = launch.wait_ready(BACKGROUND_TEST_TIMEOUT).await;
    assert_eq!(ready.state, ExecutionLaunchState::Ready);
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 1);
    assert_eq!(
        coordinator.startup_status().await.state,
        StartupPrewarmState::Running
    );

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected existing-container check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected running-container inspect in log:\n{log}"
    );
    assert!(
        !log.contains(&format!("start {container_name}")),
        "launch should not restart an already running reusable container:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "launch should not create a new workspace container while reusing an existing one:\n{log}"
    );

    ops.release_runtime();
    startup.await.expect("startup prewarm task");
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_launch_reuses_running_container_without_runtime_prewarm_or_image_checks() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let workspace_root = data_dir.path().join("ws");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");
    let log_path = data_dir.path().join("sandbox-cli-invocations.log");
    let ops = Arc::new(UnexpectedRuntimeWarmupOperations::default());
    let coordinator = test_coordinator_with_operations(data_dir.path().to_path_buf(), ops.clone());
    let workspace = Workspace {
        id: WorkspaceId::new(),
        name: "ws".to_string(),
        root_path: workspace_root.to_string_lossy().to_string(),
        created_at: Utc::now(),
        vcs_kind: None,
    };
    let container_name = format!("ctx-harness-{}", workspace.id.0);
    let settings = ExecutionSettings {
        mode: ExecutionMode::Sandbox,
        container: crate::settings::ContainerExecutionSettings {
            network_mode: crate::settings::ContainerNetworkMode::All,
            ..sandbox_container_settings()
        },
    };

    let sandbox_cli_path =
        write_running_container_sandbox_cli_shim(data_dir.path(), &log_path, &container_name);
    let _sandbox_cli = EnvVarGuard::set(
        "CTX_HARNESS_SANDBOX_CLI_PATH",
        &sandbox_cli_path.to_string_lossy(),
    );

    let launch = TrackedExecutionLaunch::new(
        &coordinator,
        coordinator
            .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
            .await,
    );

    let ready = launch.wait_ready(Duration::from_secs(5)).await;
    assert_eq!(ops.runtime_runs.load(Ordering::SeqCst), 0);
    assert!(
        ready.phases.iter().all(|phase| {
            phase.phase != HarnessSetupPhase::ImageCheck
                && phase.phase != HarnessSetupPhase::ImageLoad
        }),
        "workspace launch should not emit image phases for reusable containers: {:?}",
        ready.phases
    );
    assert!(
        ready.logs.iter().all(|line| {
            line.phase != HarnessSetupPhase::ImageCheck
                && line.phase != HarnessSetupPhase::ImageLoad
        }),
        "workspace launch should not emit image logs for reusable containers: {:?}",
        ready.logs
    );

    let log = std::fs::read_to_string(&log_path).expect("read sandbox CLI invocation log");
    assert!(
        log.contains(&format!("container inspect {container_name}")),
        "expected existing-container check in log:\n{log}"
    );
    assert!(
        log.contains(&format!(
            "container inspect --format {{{{.State.Running}}}} {container_name}"
        )),
        "expected running-container inspect in log:\n{log}"
    );
    assert!(
            !log.contains("load -i") && !log.contains("pull "),
            "workspace launch should not materialize or pull a new image for reusable containers:\n{log}"
        );
    assert!(
        !log.contains(&format!("start {container_name}")),
        "workspace launch should not restart an already running container:\n{log}"
    );
    assert!(
        !log.contains("run -d --name"),
        "workspace launch should not create a new container when reuse is possible:\n{log}"
    );
}

#[tokio::test]
async fn workspace_launch_emits_initial_log_before_runtime_work_completes() {
    let _serial = env_var_test_lock().lock().await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let coordinator = test_coordinator(data_dir.path().to_path_buf());
    let workspace = test_workspace(WorkspaceId::new());
    let settings = sandbox_execution_settings();

    let snapshot = coordinator
        .start_workspace_launch(workspace, settings, "http://127.0.0.1:4399".to_string())
        .await;
    let observed = tokio::time::timeout(QUICK_ASYNC_TEST_TIMEOUT, async {
        loop {
            let latest = coordinator
                .launch_status(&snapshot.job_id)
                .await
                .expect("missing launch job");
            if !latest.logs.is_empty() {
                break latest;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("timed out waiting for initial launch log");

    assert!(matches!(
        observed.current_phase,
        Some(HarnessSetupPhase::MachineCheck) | None
    ));
    assert!(observed.logs.iter().any(|line| {
        line.phase == HarnessSetupPhase::MachineCheck
            && line.message == "checking container runtime"
    }));

    let _terminal =
        wait_for_execution_launch_terminal(&coordinator, &snapshot.job_id, Duration::from_secs(5))
            .await;
}
