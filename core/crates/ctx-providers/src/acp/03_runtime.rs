
#[cfg(target_os = "linux")]
async fn spawn_acp_child(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<SpawnedAcpChild> {
    if systemd_run_supports_pid().await {
        let child = spawn_acp_direct(agent, workdir, env)?;
        if let Some(pid) = child.id() {
            if let Err(err) = attach_acp_scope(&agent.provider_id, pid).await {
                tracing::warn!(
                    provider_id = %agent.provider_id,
                    pid,
                    "failed to attach ACP process to systemd scope: {err:#}"
                );
            }
        }
        return Ok(SpawnedAcpChild {
            child,
            pid_override: None,
            unit: None,
        });
    }

    if !systemd_run_available().await {
        tracing::warn!(
            provider_id = %agent.provider_id,
            "systemd-run unavailable; spawning ACP without systemd scope isolation"
        );
        let child = spawn_acp_direct(agent, workdir, env)?;
        return Ok(SpawnedAcpChild {
            child,
            pid_override: None,
            unit: None,
        });
    }

    let unit = acp_scope_unit(
        &agent.provider_id,
        &Utc::now().format("%Y%m%d%H%M%S%3f").to_string(),
    );
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--user")
        .arg("--quiet")
        .arg("--pipe")
        .arg("--wait")
        .arg("--unit")
        .arg(&unit)
        .arg("--working-directory")
        .arg(workdir);
    if let Some(max_mb) = acp_memory_max_mb() {
        cmd.arg("--property").arg(format!("MemoryMax={}M", max_mb));
    }
    for (k, v) in std::env::vars() {
        cmd.arg("--setenv").arg(format!("{k}={v}"));
    }
    for (k, v) in env {
        cmd.arg("--setenv").arg(format!("{k}={v}"));
    }
    cmd.arg("--").arg(&agent.command).args(&agent.args);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let child = cmd.spawn()?;
    let pid_override = systemd_unit_main_pid(&unit).await;
    Ok(SpawnedAcpChild {
        child,
        pid_override,
        unit: Some(unit),
    })
}

#[cfg(target_os = "linux")]
fn spawn_acp_direct(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<Child> {
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args);
    cmd.current_dir(workdir);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.spawn().map_err(Into::into)
}

#[cfg(not(target_os = "linux"))]
async fn spawn_acp_child(
    agent: &AcpAgentConfig,
    workdir: &Path,
    env: &HashMap<String, String>,
) -> Result<SpawnedAcpChild> {
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args);
    cmd.current_dir(workdir);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }

    #[cfg(target_os = "macos")]
    if let Some(max_bytes) = acp_memory_max_bytes_macos() {
        unsafe {
            cmd.pre_exec(move || {
                let limit = libc::rlimit {
                    rlim_cur: max_bytes as libc::rlim_t,
                    rlim_max: max_bytes as libc::rlim_t,
                };
                libc::setrlimit(libc::RLIMIT_AS, &limit);
                Ok(())
            });
        }
    }

    let child = cmd.spawn()?;
    Ok(SpawnedAcpChild {
        child,
        pid_override: None,
    })
}

#[cfg(target_os = "macos")]
fn acp_memory_max_bytes_macos() -> Option<u64> {
    let mut value: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    let name = std::ffi::CString::new("hw.memsize").ok()?;
    let res = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            &mut value as *mut _ as *mut libc::c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if res != 0 || value == 0 {
        return None;
    }
    let mut max_bytes = ((value as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
    let min_bytes = ACP_MEMORY_MIN_MB * 1024 * 1024;
    max_bytes = max_bytes.max(min_bytes).min(value);
    Some(max_bytes)
}

#[cfg(target_os = "linux")]
fn sanitize_unit_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            out.push(ch);
        } else {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(target_os = "linux")]
fn acp_scope_unit(provider_id: &str, suffix: &str) -> String {
    let name = sanitize_unit_component(provider_id);
    format!("ctx-acp-{}-{}", name, suffix)
}

#[cfg(target_os = "linux")]
fn acp_memory_max_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            let kb_str = rest.split_whitespace().next()?;
            let kb: u64 = kb_str.parse().ok()?;
            let total_mb = kb / 1024;
            if total_mb == 0 {
                return None;
            }
            let mut max_mb = ((total_mb as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
            max_mb = max_mb.max(ACP_MEMORY_MIN_MB).min(total_mb);
            return Some(max_mb);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn acp_memory_max_bytes_windows() -> Option<u64> {
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 || status.ullTotalPhys == 0 {
        return None;
    }
    let total = status.ullTotalPhys;
    let mut max_bytes = ((total as f64) * ACP_MEMORY_MAX_FRACTION).round() as u64;
    let min_bytes = ACP_MEMORY_MIN_MB * 1024 * 1024;
    max_bytes = max_bytes.max(min_bytes).min(total);
    Some(max_bytes)
}

#[cfg(target_os = "windows")]
fn attach_acp_job(
    provider_id: &str,
    pid: u32,
) -> Result<Option<std::os::windows::io::OwnedHandle>> {
    let Some(max_bytes) = acp_memory_max_bytes_windows() else {
        return Ok(None);
    };

    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job == 0 {
        anyhow::bail!("CreateJobObjectW failed");
    }

    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    info.ProcessMemoryLimit = max_bytes as usize;
    let set_ok = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set_ok == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("SetInformationJobObject failed");
    }

    let process = unsafe {
        OpenProcess(
            PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if process == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("OpenProcess failed for pid {pid}");
    }

    let assign_ok = unsafe { AssignProcessToJobObject(job, process) };
    unsafe { CloseHandle(process) };
    if assign_ok == 0 {
        unsafe { CloseHandle(job) };
        anyhow::bail!("AssignProcessToJobObject failed");
    }

    let owned = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(job as *mut _) };
    tracing::debug!(
        provider_id = %provider_id,
        pid,
        max_bytes,
        "attached ACP process to job object"
    );
    Ok(Some(owned))
}

#[cfg(target_os = "linux")]
async fn run_command(cmd: &mut Command, label: &str) -> Result<()> {
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output())
        .await
        .context("command timed out")?
        .with_context(|| format!("running {label}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    anyhow::bail!(
        "{label} failed ({}): {}{}",
        output.status,
        stderr,
        if stdout.trim().is_empty() {
            String::new()
        } else {
            format!(" ({stdout})")
        }
    );
}

#[cfg(target_os = "linux")]
async fn systemd_run_supports_pid() -> bool {
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--help");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await;
    let Ok(Ok(output)) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .to_lowercase();
    help.contains("--pid")
}

#[cfg(target_os = "linux")]
async fn systemd_run_available() -> bool {
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--version");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await;
    let Ok(Ok(output)) = output else {
        return false;
    };
    output.status.success()
}

#[cfg(target_os = "linux")]
async fn systemd_unit_main_pid(unit: &str) -> Option<u32> {
    if let Some(pid) = systemd_unit_main_pid_once(unit).await {
        return Some(pid);
    }
    if unit.contains('.') {
        return None;
    }
    let service = format!("{unit}.service");
    if let Some(pid) = systemd_unit_main_pid_once(&service).await {
        return Some(pid);
    }
    let scope = format!("{unit}.scope");
    systemd_unit_main_pid_once(&scope).await
}

#[cfg(target_os = "linux")]
async fn systemd_unit_main_pid_once(unit: &str) -> Option<u32> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user")
        .arg("show")
        .arg(unit)
        .arg("-p")
        .arg("MainPID")
        .arg("--value");
    let output = timeout(SYSTEMD_TIMEOUT, cmd.output()).await.ok()?;
    let output = output.ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let parsed = value.trim().parse::<u32>().ok()?;
    if parsed == 0 {
        None
    } else {
        Some(parsed)
    }
}

#[cfg(target_os = "linux")]
async fn wait_for_systemd_main_pid(unit: &str) -> Option<u32> {
    for _ in 0..10 {
        if let Some(pid) = systemd_unit_main_pid(unit).await {
            return Some(pid);
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    None
}

#[cfg(target_os = "linux")]
async fn attach_acp_scope(provider_id: &str, pid: u32) -> Result<()> {
    if !systemd_run_supports_pid().await {
        anyhow::bail!("systemd-run lacks --pid; ACP scope isolation unavailable");
    }

    let unit = acp_scope_unit(provider_id, &pid.to_string());
    let mut cmd = Command::new("systemd-run");
    cmd.arg("--user").arg("--scope").arg("--unit").arg(unit);
    if let Some(max_mb) = acp_memory_max_mb() {
        cmd.arg("--property").arg(format!("MemoryMax={}M", max_mb));
    }
    cmd.arg("--pid").arg(pid.to_string());
    run_command(&mut cmd, "systemd-run").await
}

impl Drop for AcpProcess {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.try_lock() {
            let _ = child.start_kill();
        }
    }
}

async fn stdout_pump(
    process: Arc<AcpProcess>,
    mut stdout_reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) {
    loop {
        match stdout_reader.next_line().await {
            Ok(Some(line)) => {
                let parsed = match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(v) => v,
                    Err(_) => {
                        log_acp_line(&process, "stdout", &line);
                        push_capped(&process.stdout_non_json, line, 500).await;
                        continue;
                    }
                };

                if parsed.get("method").is_none() {
                    if let Some(id) = parsed.get("id").and_then(jsonrpc_id_u64) {
                        let tx = {
                            let mut pending = process.pending.lock().await;
                            pending.remove(&id)
                        };
                        if let Some(tx) = tx {
                            let _ = tx.send(parsed);
                        }
                        continue;
                    }
                }

                if parsed.get("method").and_then(|v| v.as_str())
                    == Some("session/request_permission")
                {
                    let req_id = match parsed.get("id").and_then(jsonrpc_id_u64) {
                        Some(id) => id,
                        None => continue,
                    };
                    let params = parsed.get("params").cloned().unwrap_or(json!({}));
                    let session_id = params
                        .get("sessionId")
                        .or_else(|| params.get("session_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let tool_call = params.get("toolCall").cloned().unwrap_or(json!({}));
                    let tool_call_id = tool_call
                        .get("toolCallId")
                        .or_else(|| tool_call.get("tool_call_id"))
                        .or_else(|| params.get("toolCallId"))
                        .or_else(|| params.get("tool_call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let options = params
                        .get("options")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();

                    if session_id.is_empty() || tool_call_id.is_empty() {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "missing sessionId or toolCallId"}});
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                        continue;
                    }

                    let routed = process
                        .router
                        .send(
                            &session_id,
                            AcpSessionNotification::RequestPermission {
                                req_id,
                                tool_call_id,
                                tool_call,
                                options,
                            },
                        )
                        .await;
                    if !routed {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "request_permission received outside of an active session prompt"}});
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                    }
                    continue;
                }

                let is_ask_user_question = matches!(
                    parsed.get("method").and_then(|v| v.as_str()),
                    Some("_claude_code_acp/ask_user_question")
                        | Some("__claude_code_acp/ask_user_question")
                );
                if is_ask_user_question {
                    let req_id = match parsed.get("id").and_then(jsonrpc_id_u64) {
                        Some(id) => id,
                        None => continue,
                    };
                    let params = parsed.get("params").cloned().unwrap_or(json!({}));
                    let session_id = params
                        .get("sessionId")
                        .or_else(|| params.get("session_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let tool_call_id = params
                        .get("toolCallId")
                        .or_else(|| params.get("tool_call_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    let input = params.get("input").cloned().unwrap_or(json!({}));

                    if session_id.is_empty() || tool_call_id.is_empty() {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "missing sessionId or toolCallId"}} );
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                        continue;
                    }

                    let routed = process
                        .router
                        .send(
                            &session_id,
                            AcpSessionNotification::AskUserQuestion {
                                req_id,
                                tool_call_id,
                                input,
                            },
                        )
                        .await;
                    if !routed {
                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "AskUserQuestion received outside of an active session prompt"}} );
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = process.write_tx.send(line);
                        }
                    }
                    continue;
                }

                if parsed.get("method").and_then(|v| v.as_str()) == Some("session/update") {
                    let session_id = parsed
                        .get("params")
                        .and_then(|v| v.get("sessionId").or_else(|| v.get("session_id")))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if !session_id.is_empty() {
                        let _ = process
                            .router
                            .send(&session_id, AcpSessionNotification::Update(parsed))
                            .await;
                    }
                    continue;
                }

                let session_id = parsed
                    .get("params")
                    .and_then(|v| v.get("sessionId").or_else(|| v.get("session_id")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !session_id.is_empty() {
                    let _ = process
                        .router
                        .send(&session_id, AcpSessionNotification::Raw(parsed))
                        .await;
                }
            }
            Ok(None) => {
                let status_note = describe_exit_status(&process).await;
                log_acp_line(
                    &process,
                    "meta",
                    &format!("agent stdout closed (exit status: {status_note})"),
                );
                let message = shutdown_message(&process, "agent stdout closed");
                fail_pending(&process, &message).await;
                process.router.broadcast_shutdown(message).await;
                break;
            }
            Err(e) => {
                let status_note = describe_exit_status(&process).await;
                log_acp_line(
                    &process,
                    "meta",
                    &format!("agent stdout read error (exit status: {status_note}): {e}"),
                );
                let message = shutdown_message(&process, &format!("agent stdout read error: {e}"));
                fail_pending(&process, &message).await;
                process.router.broadcast_shutdown(message).await;
                break;
            }
        }
    }
}

async fn stderr_pump(
    process: Arc<AcpProcess>,
    mut stderr_reader: tokio::io::Lines<BufReader<tokio::process::ChildStderr>>,
) {
    loop {
        match stderr_reader.next_line().await {
            Ok(Some(line)) => {
                log_acp_line(&process, "stderr", &line);
                push_capped(&process.stderr_lines, line, 500).await;
            }
            Ok(None) => break,
            Err(e) => {
                let message = format!("stderr read error: {e}");
                log_acp_line(&process, "stderr", &message);
                push_capped(&process.stderr_lines, message, 500).await;
                break;
            }
        }
    }
}

async fn push_capped(store: &Mutex<Vec<String>>, line: String, cap: usize) {
    let mut guard = store.lock().await;
    guard.push(line);
    if guard.len() > cap {
        let start = guard.len().saturating_sub(cap / 2);
        *guard = guard[start..].to_vec();
    }
}

async fn fail_pending(process: &AcpProcess, message: &str) {
    let mut pending = process.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(json!({"error": {"message": message}}));
    }
}

fn filter_process_env(env: HashMap<String, String>) -> HashMap<String, String> {
    // Per-session CTX_* vars must not be set on a shared provider process.
    // Session-specific values are passed via ACP `session/new` mcpServers env instead.
    env.into_iter()
        .filter(|(k, _)| {
            !matches!(
                k.as_str(),
                "CTX_SESSION_ID" | "CTX_PROVIDER_SESSION_REF" | "CTX_SYSTEM_PROMPT_APPEND"
            )
        })
        .collect()
}

fn acp_log_path(env: &HashMap<String, String>, provider_id: &str) -> Option<PathBuf> {
    let data_root = env.get("CTX_DATA_ROOT")?;
    let timestamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
    Some(
        Path::new(data_root)
            .join("logs")
            .join("providers")
            .join(format!("acp-{}-{}.log", provider_id, timestamp)),
    )
}

async fn spawn_acp_log_writer(path: PathBuf) -> Option<mpsc::UnboundedSender<String>> {
    if let Some(parent) = path.parent() {
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return None;
        }
    }

    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .ok()?;

    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        let mut file = file;
        while let Some(line) = rx.recv().await {
            let redacted = redact_sensitive(&line);
            if file.write_all(redacted.as_bytes()).await.is_err() {
                break;
            }
            if !redacted.ends_with('\n') && file.write_all(b"\n").await.is_err() {
                break;
            }
            let _ = file.flush().await;
        }
    });

    Some(tx)
}

fn redact_sensitive(input: &str) -> String {
    fn redact_after_marker(mut s: String, marker: &str) -> String {
        let redacted = "[REDACTED]";
        let mut search_from = 0usize;
        while let Some(rel) = s[search_from..].find(marker) {
            let marker_start = search_from + rel;
            let start = marker_start + marker.len();
            if start >= s.len() {
                break;
            }
            if s[start..].starts_with(redacted) {
                search_from = start + redacted.len();
                continue;
            }

            let mut end = s.len();
            for (i, ch) in s[start..].char_indices() {
                if ch.is_whitespace() || ch == '"' || ch == '\'' || ch == '&' {
                    end = start + i;
                    break;
                }
            }

            s.replace_range(start..end, redacted);
            search_from = start + redacted.len();
        }
        s
    }

    let mut out = input.to_string();
    out = redact_after_marker(out, "Bearer ");
    out = redact_after_marker(out, "bearer ");
    out = redact_after_marker(out, "Authorization: Bearer ");
    out = redact_after_marker(out, "authorization: Bearer ");
    out = redact_after_marker(out, "token=");
    out = redact_after_marker(out, "TOKEN=");
    out = redact_after_marker(out, "CTX_AUTH_TOKEN=");
    out = redact_after_marker(out, "ctxAuthToken\":\"");
    out = redact_after_marker(out, "ctx_auth_token\":\"");
    out
}

fn log_acp_line(process: &AcpProcess, prefix: &str, line: &str) {
    if let Some(tx) = process.log_tx.as_ref() {
        let _ = tx.send(format!("[{prefix}] {line}"));
    }
}

fn shutdown_message(process: &AcpProcess, base: &str) -> String {
    match process.log_path.as_ref() {
        Some(path) => format!("{base} (see {})", path.display()),
        None => base.to_string(),
    }
}

async fn describe_exit_status(process: &AcpProcess) -> String {
    let mut child = process.child.lock().await;
    match child.try_wait() {
        Ok(Some(status)) => format_exit_status(&status),
        Ok(None) => "still running".to_string(),
        Err(err) => format!("unavailable: {err}"),
    }
}

fn format_exit_status(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    #[cfg(unix)]
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    "unknown".to_string()
}

fn normalize_session_model_id(model_id: Option<&str>) -> Option<String> {
    let trimmed = model_id?.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_session_mode_id(mode_id: Option<&str>) -> Option<String> {
    let trimmed = mode_id?.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalize_permission_options(options: &[serde_json::Value]) -> Vec<PermissionOption> {
    options
        .iter()
        .filter_map(|opt| {
            let option_id = opt
                .get("optionId")
                .or_else(|| opt.get("option_id"))
                .or_else(|| opt.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if option_id.is_empty() {
                return None;
            }
            let label = opt
                .get("name")
                .or_else(|| opt.get("label"))
                .or_else(|| opt.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or(option_id)
                .trim();
            let kind = opt
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let description = opt
                .get("description")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string());
            Some(PermissionOption {
                option_id: option_id.to_string(),
                label: label.to_string(),
                kind,
                description,
            })
        })
        .collect()
}

fn build_permission_question(tool_call: &serde_json::Value) -> String {
    let title = tool_call
        .get("title")
        .or_else(|| tool_call.get("name"))
        .or_else(|| tool_call.get("toolName"))
        .and_then(|v| v.as_str())
        .unwrap_or("tool call")
        .trim();
    let kind = tool_call
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if kind.is_empty() {
        format!("Allow tool call: {}?", title)
    } else {
        format!("Allow tool call: {} ({})?", title, kind)
    }
}

fn select_permission_option_id(
    options: &[PermissionOption],
    selected_label: Option<&str>,
    preferred_kinds: &[&str],
) -> Option<String> {
    if let Some(label) = selected_label {
        if let Some(opt) = options.iter().find(|o| o.label == label) {
            return Some(opt.option_id.clone());
        }
    }
    for kind in preferred_kinds {
        if let Some(opt) = options.iter().find(|o| o.kind == *kind) {
            return Some(opt.option_id.clone());
        }
    }
    options.first().map(|opt| opt.option_id.clone())
}

pub(crate) fn build_request_permission_response(
    provider_id: &str,
    msg: &serde_json::Value,
) -> Result<Option<String>> {
    let id = msg
        .get("id")
        .and_then(jsonrpc_id_u64)
        .context("request_permission missing id")?;
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let options = params
        .get("options")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let selected = options
        .iter()
        .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_once"))
        .or_else(|| {
            options
                .iter()
                .find(|o| o.get("kind").and_then(|k| k.as_str()) == Some("allow_always"))
        })
        .or_else(|| options.first());

    let option_id = selected
        .and_then(|o| o.get("optionId").and_then(|v| v.as_str()))
        .unwrap_or("allow_once");

    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "_meta": {
                "context": {
                    "autoApproved": true,
                    "provider": provider_id,
                }
            },
            "outcome": {
                "outcome": "selected",
                "optionId": option_id
            }
        }
    });
    let line = serde_json::to_string(&resp)?;
    Ok(Some(line))
}

fn extract_update_field(update: &Value, key: &str) -> Option<Value> {
    update
        .get(key)
        .cloned()
        .or_else(|| update.get("_meta").and_then(|meta| meta.get(key)).cloned())
}

fn add_update_meta_fields(
    payload: &mut Map<String, Value>,
    context_window: &Option<Value>,
    usage: &Option<Value>,
) {
    if let Some(context_window) = context_window.clone() {
        payload.insert("context_window".to_string(), context_window);
    }
    if let Some(usage) = usage.clone() {
        payload.insert("usage".to_string(), usage);
    }
}
