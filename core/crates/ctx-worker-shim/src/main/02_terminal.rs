const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalClientMessage {
    Resize { cols: u16, rows: u16 },
    Input { data: String },
}

#[derive(Debug, serde::Serialize)]
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
    let cols = if spec.cols == 0 {
        DEFAULT_COLS
    } else {
        spec.cols
    };
    let rows = if spec.rows == 0 {
        DEFAULT_ROWS
    } else {
        spec.rows
    };
    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("open pty")?;

    let mut cwd = args.workdir.clone();
    if let Some(rel) = spec.cwd.as_ref() {
        let candidate = args.workdir.join(rel);
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            if canon.starts_with(&args.workdir) {
                cwd = canon;
            }
        }
    }

    let mut cmd = CommandBuilder::new(spec.shell.clone());
    cmd.cwd(cwd);
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
    let (ws_stream, _) = if let Some(pem) = args.gateway_ca_pem.as_deref() {
        let connector = gateway_ws_connector(pem)?;
        connect_async_tls_with_config(req, None, false, Some(connector))
            .await
            .context("connecting to gateway terminal relay")?
    } else {
        connect_async(req)
            .await
            .context("connecting to gateway terminal relay")?
    };
    debug!(terminal_id = %spec.terminal_id, "terminal session connected");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if ws_write.send(msg).await.is_err() {
                break;
            }
        }
    });

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

    let out_tx_status = out_tx.clone();
    let child_for_status = child_arc.clone();
    std::thread::spawn(move || loop {
        let exit: Option<portable_pty::ExitStatus> = {
            let mut child = child_for_status.lock().expect("terminal child lock");
            child.try_wait().ok().flatten()
        };
        if let Some(status) = exit {
            let exit_code = i32::try_from(status.exit_code()).ok();
            let payload = serde_json::to_string(&TerminalServerMessage::Status {
                status: "exited".to_string(),
                exit_code,
            })
            .unwrap_or_else(|_| "{\"type\":\"status\",\"status\":\"exited\"}".to_string());
            let _ = out_tx_status.send(Message::Text(payload.into()));
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    });

    loop {
        tokio::select! {
            Some(msg) = ws_read.next() => {
                let msg = msg.context("reading terminal relay")?;
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
                                    let master = master.lock().expect("terminal master lock");
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
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
            _ = shutdown_rx.recv() => {
                break;
            }
        }
    }

    {
        let mut child = child_arc.lock().expect("terminal child lock");
        let _ = child.kill();
    }

    drop(input_tx);
    drop(out_tx);
    let _ = send_task.await;
    Ok(())
}

