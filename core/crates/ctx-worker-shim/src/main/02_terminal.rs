use std::sync::atomic::{AtomicBool, Ordering};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const TERMINAL_PING_INTERVAL: Duration = Duration::from_secs(25);
const TERMINAL_RECONNECT_BASE_MS: u64 = 500;
const TERMINAL_RECONNECT_MAX_MS: u64 = 10_000;
const DAEMON_AUTH_ENV_VARS: &[&str] = &[
    "CTX_AUTH_TOKEN",
    "CTX_MCP_TOKEN",
    "CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN",
];

fn scrub_daemon_auth_env(cmd: &mut CommandBuilder) {
    for key in DAEMON_AUTH_ENV_VARS {
        cmd.env_remove(key);
    }
}

fn resolved_terminal_size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: if rows == 0 { DEFAULT_ROWS } else { rows },
        cols: if cols == 0 { DEFAULT_COLS } else { cols },
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn resolve_terminal_cwd(
    workdir: &std::path::Path,
    requested_cwd: Option<&str>,
) -> std::path::PathBuf {
    let canonical_workdir =
        std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let mut cwd = canonical_workdir.clone();
    if let Some(rel) = requested_cwd {
        let candidate = canonical_workdir.join(rel);
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            if canon.starts_with(&canonical_workdir) {
                cwd = canon;
            }
        }
    }
    cwd
}

fn base_terminal_reconnect_backoff() -> Duration {
    Duration::from_millis(TERMINAL_RECONNECT_BASE_MS)
}

fn next_terminal_reconnect_backoff(current: Duration) -> Duration {
    (current + current).min(Duration::from_millis(TERMINAL_RECONNECT_MAX_MS))
}

fn terminal_status_message(exited: bool, exit_code: Option<i32>) -> TerminalServerMessage {
    TerminalServerMessage::Status {
        status: if exited {
            "exited".to_string()
        } else {
            "running".to_string()
        },
        exit_code: if exited { exit_code } else { None },
    }
}

fn terminal_status_payload(exited: bool, exit_code: Option<i32>) -> String {
    serde_json::to_string(&terminal_status_message(exited, exit_code))
        .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"running\"}".to_string())
}

fn lock_or_recover<'a, T>(
    mutex: &'a std::sync::Mutex<T>,
    name: &str,
) -> std::sync::MutexGuard<'a, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            warn!(mutex = name, "mutex poisoned; recovering");
            poisoned.into_inner()
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
}

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalServerMessage {
    Status {
        status: String,
        exit_code: Option<i32>,
    },
}

#[derive(Debug, Clone)]
struct TerminalOpenSpec {
    terminal_id: String,
    shell: String,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
}

struct TerminalHandle {
    shutdown_tx: mpsc::UnboundedSender<()>,
}

async fn run_terminal_control(args: ResolvedArgs) -> Result<()> {
    let base = websocket_base(&args.gateway_url);
    let url = format!(
        "{}/workers/{}/terminals/control/worker",
        base, args.worker_id
    );
    debug!(url = %url, "connecting terminal control");
    let mut req = url
        .as_str()
        .into_client_request()
        .context("building terminal control request")?;
    if let Some(token) = args.gateway_token.as_deref() {
        req.headers_mut().insert(
            "x-ctx-gateway-token",
            token.parse().context("parsing gateway token")?,
        );
    }

    let (ws_stream, _) = if let Some(pem) = args.gateway_ca_pem.as_deref() {
        let connector = gateway_ws_connector(pem)?;
        connect_async_tls_with_config(req, None, false, Some(connector))
            .await
            .context("connecting to gateway terminal control")?
    } else {
        connect_async(req)
            .await
            .context("connecting to gateway terminal control")?
    };
    debug!("terminal control connected");
    let (_, mut ws_read) = ws_stream.split();

    let terminals: Arc<Mutex<HashMap<String, TerminalHandle>>> =
        Arc::new(Mutex::new(HashMap::new()));

    while let Some(msg) = ws_read.next().await {
        let msg = msg.context("reading terminal control")?;
        if let Message::Text(text) = msg {
            let control: TerminalControlMessage = match serde_json::from_str(&text) {
                Ok(control) => control,
                Err(err) => {
                    warn!("invalid terminal control message: {err:#}");
                    continue;
                }
            };

            match control {
                TerminalControlMessage::Open {
                    terminal_id,
                    shell,
                    cwd,
                    cols,
                    rows,
                } => {
                    debug!(terminal_id = %terminal_id, "opening terminal");
                    let mut guard = terminals.lock().await;
                    if guard.contains_key(&terminal_id) {
                        continue;
                    }
                    let (shutdown_tx, shutdown_rx) = mpsc::unbounded_channel::<()>();
                    guard.insert(terminal_id.clone(), TerminalHandle { shutdown_tx });
                    let spec = TerminalOpenSpec {
                        terminal_id,
                        shell,
                        cwd,
                        cols,
                        rows,
                    };
                    let args_clone = args.clone();
                    let terminals_clone = terminals.clone();
                    let terminal_id_clone = spec.terminal_id.clone();
                    tokio::spawn(async move {
                        if let Err(err) = run_terminal_session(args_clone, spec, shutdown_rx).await
                        {
                            warn!("terminal session error: {err:#}");
                        }
                        let mut guard = terminals_clone.lock().await;
                        guard.remove(&terminal_id_clone);
                    });
                }
                TerminalControlMessage::Close { terminal_id } => {
                    let mut guard = terminals.lock().await;
                    if let Some(handle) = guard.remove(&terminal_id) {
                        let _ = handle.shutdown_tx.send(());
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_terminal_session(
    args: ResolvedArgs,
    spec: TerminalOpenSpec,
    mut shutdown_rx: mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    let size = resolved_terminal_size(spec.cols, spec.rows);
    let pty_system = NativePtySystem::default();
    let pair = pty_system.openpty(size).context("open pty")?;

    let cwd = resolve_terminal_cwd(&args.workdir, spec.cwd.as_deref());

    let mut cmd = CommandBuilder::new(spec.shell.clone());
    cmd.cwd(cwd);
    scrub_daemon_auth_env(&mut cmd);
    cmd.env("TERM", "xterm-256color");

    let child = pair.slave.spawn_command(cmd).context("spawn terminal")?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("clone pty reader")?;
    let mut writer = pair.master.take_writer().context("take pty writer")?;
    let master = Arc::new(StdMutex::new(pair.master));
    let child_arc = Arc::new(StdMutex::new(child));

    let url = format!(
        "{}/workers/{}/terminals/{}/worker",
        websocket_base(&args.gateway_url),
        args.worker_id,
        spec.terminal_id
    );

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();

    let out_tx_clone = out_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = out_tx_clone.send(Message::Binary(buf[..n].to_vec().into()));
                }
                Err(_) => break,
            }
        }
    });

    let (input_tx, mut input_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });

    let exit_code = Arc::new(StdMutex::new(None));
    let exited = Arc::new(AtomicBool::new(false));
    let out_tx_status = out_tx.clone();
    let child_for_status = child_arc.clone();
    let exit_code_status = exit_code.clone();
    let exited_status = exited.clone();
    std::thread::spawn(move || loop {
        let exit: Option<portable_pty::ExitStatus> = {
            let mut child = lock_or_recover(child_for_status.as_ref(), "terminal child");
            child.try_wait().ok().flatten()
        };
        if let Some(status) = exit {
            let exit_code = i32::try_from(status.exit_code()).ok();
            {
                let mut guard = lock_or_recover(exit_code_status.as_ref(), "exit code");
                *guard = exit_code;
            }
            exited_status.store(true, Ordering::Relaxed);
            let payload = terminal_status_payload(true, exit_code);
            let _ = out_tx_status.send(Message::Text(payload.into()));
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    });

    let mut backoff = base_terminal_reconnect_backoff();
    'outer: loop {
        debug!(terminal_id = %spec.terminal_id, url = %url, "connecting terminal session");
        let mut req = url
            .as_str()
            .into_client_request()
            .context("building terminal websocket")?;
        if let Some(token) = args.gateway_token.as_deref() {
            req.headers_mut().insert(
                "x-ctx-gateway-token",
                token.parse().context("parsing gateway token")?,
            );
        }
        let connect_result = if let Some(pem) = args.gateway_ca_pem.as_deref() {
            let connector = gateway_ws_connector(pem)?;
            connect_async_tls_with_config(req, None, false, Some(connector)).await
        } else {
            connect_async(req).await
        };
        let (ws_stream, _) = match connect_result {
            Ok(result) => result,
            Err(err) => {
                warn!("terminal relay connect failed: {err:#}");
                let sleep = tokio::time::sleep(backoff);
                tokio::pin!(sleep);
                tokio::select! {
                    _ = &mut sleep => {},
                    _ = shutdown_rx.recv() => break,
                }
                backoff = next_terminal_reconnect_backoff(backoff);
                continue;
            }
        };
        debug!(terminal_id = %spec.terminal_id, "terminal session connected");
        backoff = base_terminal_reconnect_backoff();
        let (mut ws_write, mut ws_read) = ws_stream.split();

        let payload = terminal_status_payload(
            exited.load(Ordering::Relaxed),
            *lock_or_recover(exit_code.as_ref(), "exit code"),
        );
        let _ = ws_write.send(Message::Text(payload.into())).await;

        let mut ping = tokio::time::interval(TERMINAL_PING_INTERVAL);
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    let _ = ws_write.send(Message::Close(None)).await;
                    break 'outer;
                }
                outbound = out_rx.recv() => {
                    let Some(outbound) = outbound else {
                        break 'outer;
                    };
                    if ws_write.send(outbound).await.is_err() {
                        break;
                    }
                }
                msg = ws_read.next() => {
                    let msg = match msg {
                        Some(msg) => msg.context("reading terminal relay")?,
                        None => break,
                    };
                    match msg {
                        Message::Binary(data) => {
                            let _ = input_tx.send(data.to_vec());
                        }
                        Message::Text(text) => {
                            if let Ok(parsed) =
                                serde_json::from_str::<TerminalClientMessage>(text.as_str())
                            {
                                match parsed {
                                    TerminalClientMessage::Resize { cols, rows } => {
                                        let master = lock_or_recover(master.as_ref(), "terminal master");
                                        let _ = master.resize(PtySize {
                                            rows,
                                            cols,
                                            pixel_width: 0,
                                            pixel_height: 0,
                                        });
                                    }
                                    TerminalClientMessage::Input { data } => {
                                        let _ = input_tx.send(data.into_bytes());
                                    }
                                }
                            } else {
                                let _ = input_tx.send(text.as_bytes().to_vec());
                            }
                        }
                        Message::Ping(payload) => {
                            let _ = ws_write.send(Message::Pong(payload)).await;
                        }
                        Message::Pong(_) => {}
                        Message::Close(_) => break,
                        Message::Frame(_) => {}
                    }
                }
                _ = ping.tick() => {
                    if ws_write.send(Message::Ping(Vec::new().into())).await.is_err() {
                        break;
                    }
                }
            }
        }

        let sleep = tokio::time::sleep(backoff);
        tokio::pin!(sleep);
        tokio::select! {
            _ = &mut sleep => {},
            _ = shutdown_rx.recv() => break,
        }
        backoff = next_terminal_reconnect_backoff(backoff);
    }

    {
        let mut child = lock_or_recover(child_arc.as_ref(), "terminal child");
        let _ = child.kill();
    }

    drop(input_tx);
    drop(out_tx);
    Ok(())
}

#[cfg(test)]
mod terminal_tests {
    use super::*;

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<String>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            unsafe {
                if let Some(value) = &self.previous {
                    std::env::set_var(self.key, value);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn test_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ctx-worker-shim-{prefix}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create test dir");
        path
    }

    #[test]
    fn shim_terminal_size_defaults_zero_dimensions() {
        let size = resolved_terminal_size(0, 0);
        assert_eq!(size.cols, DEFAULT_COLS);
        assert_eq!(size.rows, DEFAULT_ROWS);
        assert_eq!(size.pixel_width, 0);
        assert_eq!(size.pixel_height, 0);
    }

    #[test]
    fn shim_terminal_size_preserves_explicit_dimensions() {
        let size = resolved_terminal_size(132, 48);
        assert_eq!(size.cols, 132);
        assert_eq!(size.rows, 48);
    }

    #[test]
    fn shim_scrub_daemon_auth_env_removes_sensitive_tokens() {
        let _auth = ScopedEnvVar::set("CTX_AUTH_TOKEN", "daemon-token");
        let _mcp = ScopedEnvVar::set("CTX_MCP_TOKEN", "mcp-token");
        let _shutdown =
            ScopedEnvVar::set("CTX_LOCAL_DAEMON_SHUTDOWN_TOKEN", "shutdown-token");
        let mut cmd = CommandBuilder::new("/bin/sh");

        scrub_daemon_auth_env(&mut cmd);

        for key in DAEMON_AUTH_ENV_VARS {
            assert_eq!(cmd.get_env(key), None, "expected {key} to be removed");
        }
    }

    #[test]
    fn shim_resolve_terminal_cwd_confines_to_workdir() {
        let workdir = test_dir("cwd-confined");
        let nested = workdir.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        let canonical_workdir = std::fs::canonicalize(&workdir).expect("canonical workdir");

        let resolved = resolve_terminal_cwd(&workdir, Some("nested"));
        assert_eq!(
            resolved,
            std::fs::canonicalize(&nested).expect("canonical nested")
        );

        let escaped = resolve_terminal_cwd(&workdir, Some("../"));
        assert_eq!(escaped, canonical_workdir);

        let missing = resolve_terminal_cwd(&workdir, Some("does-not-exist"));
        assert_eq!(missing, canonical_workdir);

        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[test]
    fn shim_reconnect_backoff_doubles_and_caps() {
        let mut backoff = base_terminal_reconnect_backoff();
        assert_eq!(backoff, Duration::from_millis(TERMINAL_RECONNECT_BASE_MS));

        backoff = next_terminal_reconnect_backoff(backoff);
        assert_eq!(backoff, Duration::from_millis(1_000));

        for _ in 0..8 {
            backoff = next_terminal_reconnect_backoff(backoff);
        }
        assert_eq!(backoff, Duration::from_millis(TERMINAL_RECONNECT_MAX_MS));
        assert_eq!(
            next_terminal_reconnect_backoff(backoff),
            Duration::from_millis(TERMINAL_RECONNECT_MAX_MS)
        );
        assert_eq!(
            base_terminal_reconnect_backoff(),
            Duration::from_millis(TERMINAL_RECONNECT_BASE_MS)
        );
    }

    #[test]
    fn shim_terminal_status_message_tracks_running_and_exit_state() {
        assert_eq!(
            terminal_status_message(false, Some(7)),
            TerminalServerMessage::Status {
                status: "running".to_string(),
                exit_code: None,
            }
        );
        assert_eq!(
            terminal_status_message(true, Some(7)),
            TerminalServerMessage::Status {
                status: "exited".to_string(),
                exit_code: Some(7),
            }
        );
        let payload = terminal_status_payload(true, Some(3));
        assert!(payload.contains("\"status\":\"exited\""));
        assert!(payload.contains("\"exit_code\":3"));
    }

    #[test]
    fn shim_lock_or_recover_recovers_poisoned_mutex() {
        let mutex = Arc::new(std::sync::Mutex::new(123_i32));
        let mutex_for_poison = mutex.clone();
        let _ = std::thread::spawn(move || {
            let _guard = mutex_for_poison.lock().expect("lock before poison");
            panic!("intentional poison for test");
        })
        .join();

        let mut guard = lock_or_recover(mutex.as_ref(), "test mutex");
        assert_eq!(*guard, 123);
        *guard = 456;
        drop(guard);

        assert_eq!(*lock_or_recover(mutex.as_ref(), "test mutex"), 456);
    }
}
