use super::*;
use super::diagnostics::ssh_log_snippet;
use super::login_relay::is_loopback_host_name;
#[cfg(test)]
use std::cell::Cell;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DaemonHealthCompatibility {
    #[serde(default)]
    pub(super) desktop_exact_version: String,
    #[serde(default)]
    pub(super) desktop_dev_instance_id: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct DaemonHealthSummary {
    #[serde(default)]
    pub(crate) pid: u32,
    #[serde(default)]
    pub(super) data_root: String,
    #[serde(default)]
    pub(super) compatibility: DaemonHealthCompatibility,
}

pub(crate) fn normalize_daemon_pid(pid: u32) -> Option<u32> {
    if pid == 0 {
        None
    } else {
        Some(pid)
    }
}

pub(crate) fn should_reclaim_incompatible_local_daemon(
    base_url: &str,
    health: &DaemonHealthSummary,
    expected_data_dir: &Path,
) -> bool {
    if health.pid == 0 {
        return false;
    }
    let Ok(parsed) = Url::parse(base_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    if !is_loopback_host_name(host) {
        return false;
    }
    let daemon_data_root = health.data_root.trim();
    if daemon_data_root.is_empty() {
        return false;
    }
    let daemon_root = normalize_path_for_compare(Path::new(daemon_data_root));
    let expected_root = normalize_path_for_compare(expected_data_dir);
    daemon_root == expected_root
}

pub(crate) fn reclaim_incompatible_local_daemon(
    base_url: &str,
    health: &DaemonHealthSummary,
) -> Result<()> {
    if health.pid == 0 {
        anyhow::bail!("incompatible local daemon missing pid");
    }
    let pid = health.pid;
    let graceful_revalidated = daemon_reports_expected_pid(base_url, pid);
    let graceful_err = if graceful_revalidated {
        terminate_pid(pid, false).err()
    } else {
        None
    };
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(3)).is_ok() {
        return Ok(());
    }
    let force_revalidated = daemon_reports_expected_pid(base_url, pid);
    let force_err = if force_revalidated {
        terminate_pid(pid, true).err()
    } else {
        None
    };
    if wait_for_daemon_reclaim(base_url, pid, Duration::from_secs(2)).is_ok() {
        return Ok(());
    }
    let mut details = Vec::new();
    if !graceful_revalidated {
        details.push(
            "skipped graceful terminate (daemon pid could not be revalidated via /api/health)"
                .to_string(),
        );
    }
    if let Some(err) = graceful_err {
        details.push(format!("graceful terminate failed: {err:#}"));
    }
    if !force_revalidated {
        details.push(
            "skipped force terminate (daemon pid could not be revalidated via /api/health)"
                .to_string(),
        );
    }
    if let Some(err) = force_err {
        details.push(format!("force terminate failed: {err:#}"));
    }
    if details.is_empty() {
        anyhow::bail!("incompatible local daemon pid {} did not exit", pid);
    }
    anyhow::bail!(
        "incompatible local daemon pid {} did not exit ({})",
        pid,
        details.join("; ")
    );
}

fn wait_until_daemon_reclaimed(base_url: &str, pid: u32, timeout: Duration) -> bool {
    const RECLAIM_HEALTH_PROBE_MAX_TIMEOUT: Duration = Duration::from_millis(250);
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let health_timeout =
            reclaim_health_probe_timeout(remaining, RECLAIM_HEALTH_PROBE_MAX_TIMEOUT);
        let health = daemon_health_with_timeout(base_url, health_timeout).ok();
        let pid_alive = is_pid_alive(pid).unwrap_or(true);
        if reclaim_complete(pid, pid_alive, health.as_ref()) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

pub(crate) fn wait_for_daemon_reclaim(base_url: &str, pid: u32, timeout: Duration) -> Result<()> {
    if pid == 0 {
        anyhow::bail!("invalid pid 0");
    }
    if wait_until_daemon_reclaimed(base_url, pid, timeout) {
        return Ok(());
    }
    anyhow::bail!("local daemon pid {pid} did not exit within {:?}", timeout);
}

fn reclaim_health_probe_timeout(remaining: Duration, max_probe_timeout: Duration) -> Duration {
    if remaining.is_zero() {
        return Duration::from_millis(1);
    }
    std::cmp::min(remaining, max_probe_timeout)
}

fn reclaim_complete(pid: u32, pid_alive: bool, health: Option<&DaemonHealthSummary>) -> bool {
    let same_pid_serving_health = health.map(|h| h.pid == pid).unwrap_or(false);
    !pid_alive && !same_pid_serving_health
}

fn daemon_reports_expected_pid(base_url: &str, pid: u32) -> bool {
    let health = daemon_health(base_url).ok();
    health_reports_expected_pid(pid, health.as_ref())
}

fn health_reports_expected_pid(pid: u32, health: Option<&DaemonHealthSummary>) -> bool {
    health.map(|h| h.pid == pid).unwrap_or(false)
}

fn is_pid_alive(pid: u32) -> Result<bool> {
    if pid == 0 {
        return Ok(false);
    }

    #[cfg(unix)]
    {
        let output = Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .output()
            .with_context(|| format!("running kill -0 {pid}"))?;
        if output.status.success() {
            return Ok(true);
        }
        if command_reports_missing_process(&output) {
            return Ok(false);
        }
        if command_reports_permission_denied(&output) {
            return Ok(true);
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("kill -0 {pid} failed: {stderr}");
    }

    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .arg("/FI")
            .arg(format!("PID eq {pid}"))
            .arg("/FO")
            .arg("CSV")
            .arg("/NH")
            .output()
            .with_context(|| format!("running tasklist for pid {pid}"))?;
        if !output.status.success() {
            if command_reports_missing_process(&output) {
                return Ok(false);
            }
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            anyhow::bail!("tasklist pid {pid} failed: {stderr}");
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let pid_token = format!(",\"{pid}\",");
        if stdout.contains(&pid_token) {
            return Ok(true);
        }
        if command_reports_missing_process(&output) {
            return Ok(false);
        }
        Ok(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        anyhow::bail!("pid liveness checks are unsupported on this platform");
    }
}

pub(crate) fn terminate_pid(pid: u32, force: bool) -> Result<()> {
    if pid == 0 {
        anyhow::bail!("invalid pid 0");
    }

    #[cfg(unix)]
    {
        let signal = if force { "-KILL" } else { "-TERM" };
        let output = Command::new("kill")
            .arg(signal)
            .arg(pid.to_string())
            .output()
            .with_context(|| format!("running kill {signal} {pid}"))?;
        if output.status.success() || command_reports_missing_process(&output) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("kill {signal} {pid} failed: {stderr}");
    }

    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill");
        cmd.arg("/PID").arg(pid.to_string()).arg("/T");
        if force {
            cmd.arg("/F");
        }
        let output = cmd
            .output()
            .with_context(|| format!("running taskkill for pid {pid}"))?;
        if output.status.success() || command_reports_missing_process(&output) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("taskkill pid {pid} failed: {stderr}");
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (pid, force);
        anyhow::bail!("process termination is unsupported on this platform");
    }
}

fn command_reports_missing_process(output: &std::process::Output) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains("no such process")
        || stderr.contains("no such process")
        || stdout.contains("not found")
        || stderr.contains("not found")
        || stdout.contains("not running")
        || stderr.contains("not running")
        || stdout.contains("no running instance")
        || stderr.contains("no running instance")
}

fn command_reports_permission_denied(output: &std::process::Output) -> bool {
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    stdout.contains("operation not permitted")
        || stderr.contains("operation not permitted")
        || stdout.contains("permission denied")
        || stderr.contains("permission denied")
}

pub(crate) fn daemon_health(base_url: &str) -> Result<DaemonHealthSummary> {
    daemon_health_with_timeout(base_url, daemon_health_timeout())
}

#[cfg(test)]
thread_local! {
    static DAEMON_HEALTH_CLIENT_BUILD_COUNT: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn reset_daemon_health_client_build_count() {
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn daemon_health_client_build_count() -> usize {
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(Cell::get)
}

fn daemon_health_client(timeout: Duration) -> Result<reqwest::blocking::Client> {
    static DAEMON_HEALTH_CLIENTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<u64, reqwest::blocking::Client>>,
    > = std::sync::OnceLock::new();
    let timeout_key = timeout.as_millis() as u64;
    let clients = DAEMON_HEALTH_CLIENTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut guard = clients
        .lock()
        .map_err(|err| anyhow!("daemon health client cache poisoned: {err}"))?;
    if let Some(existing) = guard.get(&timeout_key) {
        return Ok(existing.clone());
    }
    #[cfg(test)]
    DAEMON_HEALTH_CLIENT_BUILD_COUNT.with(|count| count.set(count.get() + 1));
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .context("building http client")?;
    guard.insert(timeout_key, client.clone());
    Ok(client)
}

fn daemon_health_with_timeout(base_url: &str, timeout: Duration) -> Result<DaemonHealthSummary> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = daemon_health_client(timeout)?;
    let res = client.get(url).send().context("requesting /api/health")?;
    let res = res.error_for_status().context("health status")?;
    res.json::<DaemonHealthSummary>()
        .context("parsing /api/health response")
}

fn daemon_health_timeout() -> Duration {
    const DEFAULT_MS: u64 = 5000;
    const MIN_MS: u64 = 100;
    const MAX_MS: u64 = 30000;
    let raw = std::env::var("CTX_DESKTOP_DAEMON_HEALTH_TIMEOUT_MS").unwrap_or_default();
    let parsed = raw.trim().parse::<u64>().ok().unwrap_or(DEFAULT_MS);
    let bounded = parsed.clamp(MIN_MS, MAX_MS);
    Duration::from_millis(bounded)
}

fn normalize_path_for_compare(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path))
}

pub(crate) fn local_daemon_health_matches_expected(
    health: &DaemonHealthSummary,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> bool {
    let daemon_data_root = health.data_root.trim();
    if daemon_data_root.is_empty() {
        return false;
    }
    let daemon_root = normalize_path_for_compare(Path::new(daemon_data_root));
    let expected_root = normalize_path_for_compare(expected_data_dir);
    if daemon_root != expected_root {
        return false;
    }
    let expected_version = expected_desktop_version.trim();
    if expected_version.is_empty() {
        return false;
    }
    if health.compatibility.desktop_exact_version.trim() != expected_version {
        return false;
    }
    if cfg!(debug_assertions) {
        let expected_dev_instance_id = expected_desktop_dev_instance_id.trim();
        if expected_dev_instance_id.is_empty() {
            return false;
        }
        if health.compatibility.desktop_dev_instance_id.trim() != expected_dev_instance_id {
            return false;
        }
    }
    true
}

fn display_nonempty(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "<empty>".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(crate) fn spawned_local_daemon_incompatibility_message(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
    health: &DaemonHealthSummary,
) -> String {
    format!(
        "spawned local daemon is incompatible (expected_version={}, daemon_version={}, expected_dev_instance_id={}, daemon_dev_instance_id={}, expected_data_dir={}, daemon_data_root={}, daemon_pid={}, url={})",
        display_nonempty(expected_desktop_version),
        display_nonempty(&health.compatibility.desktop_exact_version),
        display_nonempty(expected_desktop_dev_instance_id),
        display_nonempty(&health.compatibility.desktop_dev_instance_id),
        expected_data_dir.display(),
        display_nonempty(&health.data_root),
        health.pid,
        base_url,
    )
}

pub(crate) fn existing_local_daemon_matches(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> Result<bool> {
    let health = daemon_health(base_url)?;
    Ok(local_daemon_health_matches_expected(
        &health,
        expected_data_dir,
        expected_desktop_version,
        expected_desktop_dev_instance_id,
    ))
}

pub(crate) fn existing_local_daemon_matches_or_absent(
    base_url: &str,
    expected_data_dir: &Path,
    expected_desktop_version: &str,
    expected_desktop_dev_instance_id: &str,
) -> bool {
    existing_local_daemon_matches(
        base_url,
        expected_data_dir,
        expected_desktop_version,
        expected_desktop_dev_instance_id,
    )
    .unwrap_or(false)
}

pub(crate) fn probe_daemon_health(base_url: &str) -> Result<()> {
    let _ = daemon_health(base_url)?;
    Ok(())
}

pub(crate) fn probe_local_daemon_health_with_retry(base_url: &str) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..LOCAL_DAEMON_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
        let delay = LOCAL_DAEMON_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    Err(last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed")))
}

pub(crate) fn probe_daemon_health_with_retry(
    base_url: &str,
    local_port: u16,
    tunnel: &mut Child,
    stderr_log: &std::sync::Arc<std::sync::Mutex<String>>,
) -> Result<()> {
    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..SSH_TUNNEL_HEALTH_RETRIES {
        match probe_daemon_health(base_url) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if let Ok(Some(status)) = tunnel.try_wait() {
                    let stderr = ssh_log_snippet(stderr_log);
                    if stderr.is_empty() {
                        return Err(anyhow!("ssh tunnel exited ({status})"));
                    }
                    return Err(anyhow!("ssh tunnel exited ({status}): {stderr}"));
                }
            }
        }
        let delay = SSH_TUNNEL_HEALTH_BASE_DELAY_MS.saturating_mul((attempt + 1) as u64);
        std::thread::sleep(Duration::from_millis(delay));
    }
    let err = last_err.unwrap_or_else(|| anyhow!("requesting /api/health failed"));
    let stderr = ssh_log_snippet(stderr_log);
    if stderr.is_empty() {
        Err(anyhow!(
            "daemon did not become healthy at {base_url} (local_port={local_port}): {err:#}"
        ))
    } else {
        Err(anyhow!(
            "daemon did not become healthy at {base_url} (local_port={local_port}): {err:#}; ssh stderr: {stderr}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_daemon_health_match_requires_expected_data_root_and_mode_specific_identity() {
        let expected_dir =
            std::env::temp_dir().join(format!("ctx-daemon-health-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");
        let other_dir = expected_dir.join("other");
        std::fs::create_dir_all(&other_dir).expect("create other dir");

        let matching = DaemonHealthSummary {
            pid: 42,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "1.2.3".to_string(),
                desktop_dev_instance_id: "dev-wt-a".to_string(),
            },
        };
        assert!(local_daemon_health_matches_expected(
            &matching,
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));
        if cfg!(debug_assertions) {
            assert!(!local_daemon_health_matches_expected(
                &matching,
                &expected_dir,
                "9.9.9",
                "dev-wt-a",
            ));
            assert!(!local_daemon_health_matches_expected(
                &matching,
                &expected_dir,
                "1.2.3",
                "dev-wt-b",
            ));
        }

        let wrong_root = DaemonHealthSummary {
            pid: 42,
            data_root: other_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "1.2.3".to_string(),
                desktop_dev_instance_id: "dev-wt-a".to_string(),
            },
        };
        assert!(!local_daemon_health_matches_expected(
            &wrong_root,
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn existing_local_daemon_match_errors_are_treated_as_absent() {
        let expected_dir = std::env::temp_dir().join(format!(
            "ctx-daemon-existing-match-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");

        assert!(!existing_local_daemon_matches_or_absent(
            "not-a-url",
            &expected_dir,
            "1.2.3",
            "dev-wt-a",
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn spawned_daemon_incompatibility_message_reports_expected_and_actual_values() {
        let expected_dir = std::env::temp_dir().join(format!(
            "ctx-daemon-spawn-incompatible-{}",
            uuid::Uuid::new_v4()
        ));
        let health = DaemonHealthSummary {
            pid: 4242,
            data_root: "/tmp/ctx-daemon-other".to_string(),
            compatibility: DaemonHealthCompatibility {
                desktop_exact_version: "0.1.1".to_string(),
                desktop_dev_instance_id: "dev-other".to_string(),
            },
        };

        let msg = spawned_local_daemon_incompatibility_message(
            "http://127.0.0.1:4123",
            &expected_dir,
            "0.2.20",
            "dev-main",
            &health,
        );

        assert!(msg.contains("expected_version=0.2.20"));
        assert!(msg.contains("daemon_version=0.1.1"));
        assert!(msg.contains("expected_dev_instance_id=dev-main"));
        assert!(msg.contains("daemon_dev_instance_id=dev-other"));
        assert!(msg.contains("daemon_data_root=/tmp/ctx-daemon-other"));
        assert!(msg.contains("daemon_pid=4242"));
        assert!(msg.contains("url=http://127.0.0.1:4123"));
    }

    #[test]
    fn daemon_health_reuses_cached_client_for_same_timeout() {
        reset_daemon_health_client_build_count();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let addr = listener.local_addr().expect("listener addr");
        let server = std::thread::spawn(move || {
            let body =
                "{\"pid\":1,\"data_root\":\"/tmp/test\",\"compatibility\":{\"desktop_exact_version\":\"1.0.0\",\"desktop_dev_instance_id\":\"dev\"}}";
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buf = [0_u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                std::io::Write::write_all(&mut stream, response.as_bytes())
                    .expect("write response");
            }
        });

        let base_url = format!("http://{}", addr);
        for _ in 0..2 {
            let health = daemon_health_with_timeout(&base_url, Duration::from_secs(5))
                .expect("daemon health succeeds");
            assert_eq!(health.pid, 1);
        }

        server.join().expect("join test server");
        assert_eq!(daemon_health_client_build_count(), 1);
    }

    #[test]
    fn reclaim_predicate_requires_loopback_same_data_dir_and_pid() {
        let expected_dir =
            std::env::temp_dir().join(format!("ctx-daemon-reclaim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&expected_dir).expect("create expected dir");
        let other_dir = expected_dir.join("other");
        std::fs::create_dir_all(&other_dir).expect("create other dir");

        let compatible_root = DaemonHealthSummary {
            pid: 100,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &compatible_root,
            &expected_dir,
        ));

        let non_loopback = DaemonHealthSummary {
            pid: 100,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://192.168.1.30:4123",
            &non_loopback,
            &expected_dir,
        ));

        let wrong_root = DaemonHealthSummary {
            pid: 100,
            data_root: other_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &wrong_root,
            &expected_dir,
        ));

        let missing_pid = DaemonHealthSummary {
            pid: 0,
            data_root: expected_dir.to_string_lossy().to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        assert!(!should_reclaim_incompatible_local_daemon(
            "http://127.0.0.1:4123",
            &missing_pid,
            &expected_dir,
        ));

        std::fs::remove_dir_all(&expected_dir).ok();
    }

    #[test]
    fn reclaim_complete_requires_pid_exit_and_no_same_pid_health() {
        let pid = 4242u32;
        let same_pid_health = DaemonHealthSummary {
            pid,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        let other_pid_health = DaemonHealthSummary {
            pid: pid + 1,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };

        assert!(!reclaim_complete(pid, true, None));
        assert!(!reclaim_complete(pid, true, Some(&same_pid_health)));
        assert!(!reclaim_complete(pid, false, Some(&same_pid_health)));
        assert!(reclaim_complete(pid, false, None));
        assert!(reclaim_complete(pid, false, Some(&other_pid_health)));
    }

    #[test]
    fn health_reports_expected_pid_only_when_health_matches_pid() {
        let pid = 5151u32;
        let matching = DaemonHealthSummary {
            pid,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };
        let other = DaemonHealthSummary {
            pid: pid + 1,
            data_root: "/tmp/ctx".to_string(),
            compatibility: DaemonHealthCompatibility::default(),
        };

        assert!(health_reports_expected_pid(pid, Some(&matching)));
        assert!(!health_reports_expected_pid(pid, Some(&other)));
        assert!(!health_reports_expected_pid(pid, None));
    }

    #[test]
    fn reclaim_health_probe_timeout_respects_remaining_budget() {
        let max_probe = Duration::from_millis(250);
        assert_eq!(
            reclaim_health_probe_timeout(Duration::from_millis(900), max_probe),
            max_probe
        );
        assert_eq!(
            reclaim_health_probe_timeout(Duration::from_millis(40), max_probe),
            Duration::from_millis(40)
        );
        assert_eq!(
            reclaim_health_probe_timeout(Duration::ZERO, max_probe),
            Duration::from_millis(1)
        );
    }
}
