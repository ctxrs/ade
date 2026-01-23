struct AcpProcess {
    agent: AcpAgentConfig,
    child: Mutex<Child>,
    pid_override: AtomicU32,
    #[cfg(target_os = "windows")]
    job: Option<std::os::windows::io::OwnedHandle>,
    write_tx: mpsc::UnboundedSender<String>,
    log_path: Option<PathBuf>,
    log_tx: Option<mpsc::UnboundedSender<String>>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
    supports_load: AtomicBool,
    supports_resume: AtomicBool,
    auth_methods: Mutex<Option<serde_json::Value>>,
    stderr_lines: Mutex<Vec<String>>,
    stdout_non_json: Mutex<Vec<String>>,
    ask_user_question: Option<Arc<AskUserQuestionBroker>>,
    router: SessionRouter,
}

struct SpawnedAcpChild {
    child: Child,
    pid_override: Option<u32>,
    #[cfg(target_os = "linux")]
    unit: Option<String>,
}

#[derive(Default)]
struct SessionRouter {
    sessions: RwLock<HashMap<String, mpsc::UnboundedSender<AcpSessionNotification>>>,
}

enum AcpSessionNotification {
    Update(serde_json::Value),
    AskUserQuestion {
        req_id: u64,
        tool_call_id: String,
        input: serde_json::Value,
    },
    RequestPermission {
        req_id: u64,
        tool_call_id: String,
        tool_call: serde_json::Value,
        options: Vec<serde_json::Value>,
    },
    Raw(serde_json::Value),
    Shutdown {
        message: String,
    },
}

impl SessionRouter {
    async fn register(
        &self,
        session_id: &str,
    ) -> Result<mpsc::UnboundedReceiver<AcpSessionNotification>> {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(session_id) {
            anyhow::bail!("session {session_id} already has an active prompt");
        }
        sessions.insert(session_id.to_string(), tx);
        Ok(rx)
    }

    async fn unregister(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id);
    }

    async fn send(&self, session_id: &str, msg: AcpSessionNotification) -> bool {
        let sender = {
            let sessions = self.sessions.read().await;
            sessions.get(session_id).cloned()
        };
        if let Some(sender) = sender {
            let _ = sender.send(msg);
            true
        } else {
            false
        }
    }

    async fn broadcast_shutdown(&self, message: String) {
        let sessions = self.sessions.read().await;
        for sender in sessions.values() {
            let _ = sender.send(AcpSessionNotification::Shutdown {
                message: message.clone(),
            });
        }
    }
}

impl AcpProcess {
    async fn pid(&self) -> Option<u32> {
        let pid = self.pid_override.load(Ordering::Relaxed);
        if pid != 0 {
            return Some(pid);
        }
        let child = self.child.lock().await;
        child.id()
    }

    async fn shutdown(&self) {
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
    }

    async fn spawn(
        agent: AcpAgentConfig,
        client: AcpClientConfig,
        workdir: PathBuf,
        env: HashMap<String, String>,
        ask_user_question: Option<Arc<AskUserQuestionBroker>>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<Arc<Self>> {
        let (log_tx, log_path) = match acp_log_path(&env, &agent.provider_id) {
            Some(path) => match spawn_acp_log_writer(path.clone()).await {
                Some(tx) => (Some(tx), Some(path)),
                None => (None, None),
            },
            None => (None, None),
        };

        let agent = apply_system_prompt_append_args(agent, &client);

        let spawned = spawn_acp_child(&agent, &workdir, &env)
            .await
            .with_context(|| {
                format!(
                    "spawning ACP agent {} ({})",
                    agent.provider_id, agent.command
                )
            })?;
        let mut child = spawned.child;
        let pid_override = spawned.pid_override;
        #[cfg(target_os = "linux")]
        let unit = spawned.unit;

        #[cfg(target_os = "windows")]
        let job = match attach_acp_job(&agent.provider_id, child.id().unwrap_or(0)) {
            Ok(handle) => handle,
            Err(err) => {
                tracing::warn!(
                    provider_id = %agent.provider_id,
                    "failed to attach ACP process to job object: {err:#}"
                );
                None
            }
        };

        if let Some(tx) = log_tx.as_ref() {
            let pid = pid_override.or(child.id()).unwrap_or(0);
            let _ = tx.send(format!(
                "[meta] started provider={} pid={}",
                agent.provider_id, pid
            ));
        }

        let stdin = child.stdin.take().context("capturing agent stdin")?;
        let stdout = child.stdout.take().context("capturing agent stdout")?;
        let stderr = child.stderr.take().context("capturing agent stderr")?;

        let mut stdin = tokio::io::BufWriter::new(stdin);
        let stdout_reader = BufReader::new(stdout).lines();
        let stderr_reader = BufReader::new(stderr).lines();

        let (write_tx, mut write_rx) = mpsc::unbounded_channel::<String>();
        let _writer = tokio::spawn(async move {
            while let Some(line) = write_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                let _ = stdin.flush().await;
            }
        });

        let process = Arc::new(Self {
            agent,
            child: Mutex::new(child),
            pid_override: AtomicU32::new(pid_override.unwrap_or(0)),
            #[cfg(target_os = "windows")]
            job,
            write_tx,
            log_path,
            log_tx,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            supports_load: AtomicBool::new(false),
            supports_resume: AtomicBool::new(false),
            auth_methods: Mutex::new(None),
            stderr_lines: Mutex::new(Vec::new()),
            stdout_non_json: Mutex::new(Vec::new()),
            ask_user_question,
            router: SessionRouter::default(),
        });

        #[cfg(target_os = "linux")]
        if let Some(unit) = unit {
            if pid_override.is_none() {
                let process = Arc::clone(&process);
                tokio::spawn(async move {
                    if let Some(pid) = wait_for_systemd_main_pid(&unit).await {
                        process.pid_override.store(pid, Ordering::Relaxed);
                    }
                });
            }
        }

        let stdout_process = Arc::clone(&process);
        tokio::spawn(async move {
            stdout_pump(stdout_process, stdout_reader).await;
        });

        let stderr_process = Arc::clone(&process);
        tokio::spawn(async move {
            stderr_pump(stderr_process, stderr_reader).await;
        });

        process.initialize(client, event_sink).await?;
        Ok(process)
    }

    async fn initialize(
        &self,
        client: AcpClientConfig,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let init_resp = self
            .send_request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": client.client_capabilities,
                    "clientInfo": {
                        "name": client.client_name,
                        "title": client.client_title,
                        "version": client.client_version,
                    }
                }),
            )
            .await
            .context("waiting for initialize response")?;
        if let Some(err) = init_resp.get("error") {
            anyhow::bail!("ACP initialize error: {err}");
        }

        let auth_methods = init_resp
            .get("result")
            .and_then(|v| v.get("authMethods").or_else(|| v.get("auth_methods")))
            .cloned();
        let capabilities = init_resp
            .get("result")
            .and_then(|v| v.get("capabilities").or_else(|| v.get("agentCapabilities")));
        let supports_load = capabilities
            .and_then(|v| v.get("loadSession"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let supports_resume = capabilities
            .and_then(|v| {
                v.get("sessionCapabilities")
                    .or_else(|| v.get("session_capabilities"))
            })
            .and_then(|v| v.get("resume"))
            .map(|v| match v {
                serde_json::Value::Bool(value) => *value,
                serde_json::Value::Null => false,
                _ => true,
            })
            .unwrap_or(false);

        {
            let mut guard = self.auth_methods.lock().await;
            *guard = auth_methods.clone();
        }
        self.supports_load.store(supports_load, Ordering::SeqCst);
        self.supports_resume
            .store(supports_resume, Ordering::SeqCst);

        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "auth_methods": auth_methods.clone(),
                    "authMethods": auth_methods,
                    "supports_load": supports_load,
                    "supports_resume": supports_resume,
                }),
            })
            .await;

        Ok(())
    }

    async fn default_auth_method_id(&self) -> Option<String> {
        let methods = { self.auth_methods.lock().await.clone() }?;
        let list = methods.as_array()?;
        for m in list {
            if let Some(id) = m
                .get("methodId")
                .or_else(|| m.get("method_id"))
                .or_else(|| m.get("id"))
                .and_then(|v| v.as_str())
            {
                if !id.trim().is_empty() {
                    return Some(id.to_string());
                }
            }
        }
        None
    }

    async fn authenticate(&self, method_id: String) -> Result<()> {
        let resp = self
            .send_request("authenticate", json!({"methodId": method_id}))
            .await
            .context("waiting for authenticate response")?;

        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP authenticate error: {err}");
        }

        Ok(())
    }

    async fn create_or_load_session(
        &self,
        workdir: &Path,
        client: &AcpClientConfig,
        resume_session_id: Option<String>,
        event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<CreatedAcpSession> {
        let cwd = workdir
            .canonicalize()
            .unwrap_or_else(|_| workdir.to_path_buf())
            .to_string_lossy()
            .to_string();
        let mcp_servers = client
            .mcp_servers
            .iter()
            .map(|s| {
                let mut server = json!({
                    "name": s.name,
                    "command": s.command,
                    "args": s.args,
                    "env": s.env.iter().map(|(name, value)| json!({"name": name, "value": value})).collect::<Vec<_>>(),
                });
                if let Some(meta) = &s.meta {
                    if let Some(obj) = server.as_object_mut() {
                        obj.insert("_meta".to_string(), meta.clone());
                    }
                }
                server
            })
            .collect::<Vec<_>>();

        let mut resumed = false;
        let mut session_id: Option<String> = None;
        let mut modes: Option<serde_json::Value> = None;
        let mut models: Option<serde_json::Value> = None;

        if let Some(resume_id) = resume_session_id.clone() {
            if self.supports_resume.load(Ordering::SeqCst) {
                let resume_resp = self
                    .send_request(
                        "session/resume",
                        json!({"sessionId": resume_id, "cwd": cwd, "mcpServers": mcp_servers}),
                    )
                    .await
                    .context("waiting for session/resume response")?;
                if resume_resp.get("error").is_none() {
                    resumed = true;
                    session_id = Some(resume_id);
                    modes = resume_resp
                        .get("result")
                        .and_then(|v| v.get("modes"))
                        .cloned();
                    models = resume_resp
                        .get("result")
                        .and_then(|v| v.get("models"))
                        .cloned();
                } else if let Some(err) = resume_resp.get("error") {
                    if is_auth_required_error(err) {
                        let auth_methods = self.auth_methods.lock().await.clone();
                        let _ = event_sink
                            .send(NormalizedEvent {
                                event_type: SessionEventType::AuthRequired,
                                payload_json: json!({
                                    "kind": "auth_required",
                                    "provider": self.agent.provider_id,
                                    "message": "Provider requires authentication before resuming a session.",
                                    "auth_methods": auth_methods.clone(),
                                    "authMethods": auth_methods,
                                    "acp_error": err,
                                }),
                            })
                            .await;
                        anyhow::bail!("authentication required");
                    }
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::Error,
                            payload_json: json!({
                                "provider": self.agent.provider_id,
                                "message": "session/resume failed; starting a new provider session",
                                "acp_error": resume_resp.get("error"),
                            }),
                        })
                        .await;
                }
            }
        }

        if session_id.is_none() {
            let mut new_payload = json!({"cwd": cwd, "mcpServers": mcp_servers});
            if let Some(append) = client.system_prompt_append.as_deref() {
                let trimmed = append.trim();
                if !trimmed.is_empty() {
                    if let Some(obj) = new_payload.as_object_mut() {
                        obj.insert(
                            "_meta".to_string(),
                            json!({"systemPrompt": {"append": trimmed}}),
                        );
                    }
                }
            }
            let new_resp = self
                .send_request("session/new", new_payload)
                .await
                .context("waiting for session/new response")?;
            if let Some(err) = new_resp.get("error") {
                if is_auth_required_error(err) {
                    let auth_methods = self.auth_methods.lock().await.clone();
                    let _ = event_sink
                        .send(NormalizedEvent {
                            event_type: SessionEventType::AuthRequired,
                            payload_json: json!({
                                "kind": "auth_required",
                                "provider": self.agent.provider_id,
                                "message": "Provider requires authentication before starting a session.",
                                "auth_methods": auth_methods.clone(),
                                "authMethods": auth_methods,
                                "acp_error": err,
                            }),
                        })
                        .await;
                    anyhow::bail!("authentication required");
                }
                anyhow::bail!("ACP session/new error: {err}");
            }
            let id = new_resp
                .get("result")
                .and_then(|v| v.get("sessionId"))
                .and_then(|v| v.as_str())
                .context("missing sessionId in session/new response")?
                .to_string();
            session_id = Some(id.clone());
            modes = new_resp.get("result").and_then(|v| v.get("modes")).cloned();
            models = new_resp
                .get("result")
                .and_then(|v| v.get("models"))
                .cloned();
        }

        let session_id = session_id.context("missing ACP sessionId")?;
        let auth_methods = self.auth_methods.lock().await.clone();
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Init,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "acp_session_id": session_id,
                    "resumed": resumed,
                    "supports_load": self.supports_load.load(Ordering::SeqCst),
                    "supports_resume": self.supports_resume.load(Ordering::SeqCst),
                    "modes": modes,
                    "models": models,
                    "auth_methods": auth_methods.clone(),
                    "authMethods": auth_methods,
                }),
            })
            .await;
        Ok(CreatedAcpSession { session_id })
    }

    async fn prompt(
        &self,
        context_session_id: &str,
        acp_session_id: &str,
        prompt: Vec<serde_json::Value>,
        event_sink: mpsc::Sender<NormalizedEvent>,
        mut cancel_rx: oneshot::Receiver<()>,
    ) -> Result<()> {
        let mut state = StreamState::default();
        let mut session_rx = self.router.register(acp_session_id).await?;

        let mut prompt_rx = self
            .send_request_raw(
                "session/prompt",
                json!({"sessionId": acp_session_id, "prompt": prompt}),
            )
            .await?;

        let emit_raw_notifications = true;
        let mut ask_req_id: Option<u64> = None;
        let mut ask_tool_call_id: Option<String> = None;
        let mut ask_rx: Option<oneshot::Receiver<AskUserQuestionAnswer>> = None;
        let mut perm_req_id: Option<u64> = None;
        let mut perm_tool_call_id: Option<String> = None;
        let mut perm_broker_id: Option<String> = None;
        let mut perm_question: Option<String> = None;
        let mut perm_options: Vec<PermissionOption> = Vec::new();
        let mut perm_rx: Option<oneshot::Receiver<AskUserQuestionAnswer>> = None;

        let prompt_resp = loop {
            tokio::select! {
                answer = async {
                    if let Some(rx) = ask_rx.as_mut() {
                        rx.await
                    } else {
                        std::future::pending::<
                            Result<AskUserQuestionAnswer, tokio::sync::oneshot::error::RecvError>,
                        >()
                        .await
                    }
                } => {
                    let req_id = ask_req_id.take().context("missing AskUserQuestion request id")?;
                    let tool_call_id = ask_tool_call_id.take().unwrap_or_default();

                    let answer = match answer {
                        Ok(v) => v,
                        Err(_) => AskUserQuestionAnswer {
                            outcome: AskUserQuestionOutcome::Cancelled,
                            answers: Default::default(),
                        },
                    };

                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "outcome": answer.outcome.as_str(),
                            "answers": answer.answers,
                        }
                    });
                    let line = serde_json::to_string(&resp).context("serializing AskUserQuestion response")?;
                    let _ = self.write_tx.send(line);

                    if let Some(broker) = self.ask_user_question.as_ref() {
                        broker.abandon(context_session_id, &tool_call_id).await;
                    }

                    ask_rx = None;
                    continue;
                }
                perm_answer = async {
                    if let Some(rx) = perm_rx.as_mut() {
                        rx.await
                    } else {
                        std::future::pending::<
                            Result<AskUserQuestionAnswer, tokio::sync::oneshot::error::RecvError>,
                        >()
                        .await
                    }
                } => {
                    let req_id = perm_req_id.take().context("missing permission request id")?;
                    let tool_call_id = perm_tool_call_id.take().unwrap_or_default();
                    let broker_id = perm_broker_id.take().unwrap_or_else(|| tool_call_id.clone());
                    let question = perm_question.take().unwrap_or_default();
                    let options = std::mem::take(&mut perm_options);

                    let answer = match perm_answer {
                        Ok(v) => v,
                        Err(_) => AskUserQuestionAnswer {
                            outcome: AskUserQuestionOutcome::Cancelled,
                            answers: Default::default(),
                        },
                    };

                    let selected_label = answer
                        .answers
                        .get(&question)
                        .map(|s| s.as_str())
                        .or_else(|| answer.answers.values().next().map(|s| s.as_str()));

                    let option_id = match answer.outcome {
                        AskUserQuestionOutcome::Submitted => select_permission_option_id(
                            &options,
                            selected_label,
                            &["allow_once", "allow_always"],
                        ),
                        AskUserQuestionOutcome::Cancelled => select_permission_option_id(
                            &options,
                            None,
                            &["reject_once", "reject_always"],
                        ),
                    }
                    .unwrap_or_else(|| "reject".to_string());

                    let resp = json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": {
                            "_meta": {
                                "context": {
                                    "provider": self.agent.provider_id,
                                    "autoApproved": false,
                                }
                            },
                            "outcome": {
                                "outcome": "selected",
                                "optionId": option_id
                            }
                        }
                    });
                    let line = serde_json::to_string(&resp).context("serializing permission response")?;
                    let _ = self.write_tx.send(line);

                    if let Some(broker) = self.ask_user_question.as_ref() {
                        broker.abandon(context_session_id, &broker_id).await;
                    }

                    perm_rx = None;
                    continue;
                }
                _ = &mut cancel_rx => {
                    let _ = self.send_cancel_notification(acp_session_id);

                    if let Some(req_id) = ask_req_id.take() {
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "outcome": AskUserQuestionOutcome::Cancelled.as_str(),
                                "answers": {},
                            }
                        });
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = self.write_tx.send(line);
                        }
                    }
                    if let (Some(tool_call_id), Some(broker)) =
                        (ask_tool_call_id.as_deref(), self.ask_user_question.as_ref())
                    {
                        broker.abandon(context_session_id, tool_call_id).await;
                    }

                    if let Some(req_id) = perm_req_id.take() {
                        let options = std::mem::take(&mut perm_options);
                        let option_id = select_permission_option_id(
                            &options,
                            None,
                            &["reject_once", "reject_always"],
                        )
                        .unwrap_or_else(|| "reject".to_string());
                        let resp = json!({
                            "jsonrpc": "2.0",
                            "id": req_id,
                            "result": {
                                "outcome": {
                                    "outcome": "selected",
                                    "optionId": option_id
                                }
                            }
                        });
                        if let Ok(line) = serde_json::to_string(&resp) {
                            let _ = self.write_tx.send(line);
                        }
                    }
                    if let (Some(broker_id), Some(broker)) =
                        (perm_broker_id.as_deref(), self.ask_user_question.as_ref())
                    {
                        broker.abandon(context_session_id, broker_id).await;
                    }

                    let _ = event_sink.send(NormalizedEvent {
                        event_type: SessionEventType::InterruptRequested,
                        payload_json: json!({"provider": self.agent.provider_id}),
                    }).await;
                    break json!({"result": { "stopReason": "cancelled" }});
                }
                msg = session_rx.recv() => {
                    match msg {
                        Some(AcpSessionNotification::Update(parsed)) => {
                            let events = normalize_session_update(&parsed, &mut state);
                            for ev in events {
                                let _ = event_sink.send(ev).await;
                            }
                        }
                        Some(AcpSessionNotification::AskUserQuestion { req_id, tool_call_id, input }) => {
                            if ask_rx.is_some() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "nested AskUserQuestion not supported"}});
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let Some(broker) = self.ask_user_question.as_ref() else {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32601, "message": "AskUserQuestion not supported by this client"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            };

                            ask_req_id = Some(req_id);
                            ask_tool_call_id = Some(tool_call_id.clone());
                            ask_rx = Some(broker.begin(context_session_id.to_string(), tool_call_id.clone()).await);

                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Notice,
                                payload_json: json!({
                                    "kind": "ask_user_question",
                                    "provider": self.agent.provider_id,
                                    "tool_call_id": tool_call_id,
                                    "input": input,
                                }),
                            }).await;
                        }
                        Some(AcpSessionNotification::RequestPermission { req_id, tool_call_id, tool_call, options }) => {
                            if perm_rx.is_some() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "nested permission request not supported"}});
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let Some(broker) = self.ask_user_question.as_ref() else {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32601, "message": "permission requests not supported by this client"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            };

                            let normalized = normalize_permission_options(&options);
                            if normalized.is_empty() {
                                let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32602, "message": "permission request missing options"}} );
                                let line = serde_json::to_string(&resp)?;
                                let _ = self.write_tx.send(line);
                                continue;
                            }

                            let question = build_permission_question(&tool_call);
                            let prompt_options = normalized
                                .iter()
                                .map(|opt| {
                                    let mut obj = json!({
                                        "label": opt.label,
                                    });
                                    if let Some(desc) = opt.description.as_ref() {
                                        if let Some(map) = obj.as_object_mut() {
                                            map.insert("description".to_string(), json!(desc));
                                        }
                                    }
                                    obj
                                })
                                .collect::<Vec<_>>();
                            let input = json!({
                                "questions": [{
                                    "header": "Permission",
                                    "question": question,
                                    "options": prompt_options,
                                    "multiSelect": false,
                                }],
                                "tool_call": tool_call,
                                "request_type": "permission",
                            });
                            let broker_id = format!("permission:{}", tool_call_id);

                            perm_req_id = Some(req_id);
                            perm_tool_call_id = Some(tool_call_id.clone());
                            perm_broker_id = Some(broker_id.clone());
                            perm_question = Some(question);
                            perm_options = normalized;
                            perm_rx = Some(broker.begin(context_session_id.to_string(), broker_id.clone()).await);

                            let _ = event_sink.send(NormalizedEvent {
                                event_type: SessionEventType::Notice,
                                payload_json: json!({
                                    "kind": "ask_user_question",
                                    "subkind": "permission_request",
                                    "provider": self.agent.provider_id,
                                    "tool_call_id": broker_id,
                                    "input": input,
                                }),
                            }).await;
                        }
                        Some(AcpSessionNotification::Raw(parsed)) => {
                            if emit_raw_notifications {
                                let _ = event_sink.send(NormalizedEvent {
                                    event_type: SessionEventType::Init,
                                    payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                                }).await;
                            }
                        }
                        Some(AcpSessionNotification::Shutdown { message }) => {
                            anyhow::bail!("ACP process closed: {message}");
                        }
                        None => {
                            anyhow::bail!("ACP session channel closed");
                        }
                    }
                }
                resp = &mut prompt_rx => {
                    let resp = resp.context("awaiting ACP response")?;
                    loop {
                        match session_rx.try_recv() {
                            Ok(msg) => {
                                match msg {
                                    AcpSessionNotification::Update(parsed) => {
                                        let events = normalize_session_update(&parsed, &mut state);
                                        for ev in events {
                                            let _ = event_sink.send(ev).await;
                                        }
                                    }
                                    AcpSessionNotification::AskUserQuestion { req_id, tool_call_id, .. } => {
                                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "AskUserQuestion received after prompt completed"}});
                                        let line = serde_json::to_string(&resp)?;
                                        let _ = self.write_tx.send(line);
                                        if let Some(broker) = self.ask_user_question.as_ref() {
                                            broker.abandon(context_session_id, &tool_call_id).await;
                                        }
                                    }
                                    AcpSessionNotification::RequestPermission { req_id, tool_call_id, .. } => {
                                        let resp = json!({"jsonrpc":"2.0","id": req_id, "error": {"code": -32603, "message": "permission request received after prompt completed"}});
                                        let line = serde_json::to_string(&resp)?;
                                        let _ = self.write_tx.send(line);
                                        if let Some(broker) = self.ask_user_question.as_ref() {
                                            let broker_id = format!("permission:{}", tool_call_id);
                                            broker.abandon(context_session_id, &broker_id).await;
                                        }
                                    }
                                    AcpSessionNotification::Raw(parsed) => {
                                        if emit_raw_notifications {
                                            let _ = event_sink.send(NormalizedEvent {
                                                event_type: SessionEventType::Init,
                                                payload_json: json!({"provider": self.agent.provider_id, "acp_event": parsed}),
                                            }).await;
                                        }
                                    }
                                    AcpSessionNotification::Shutdown { message } => {
                                        anyhow::bail!("ACP process closed: {message}");
                                    }
                                }
                            }
                            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                        }
                    }
                    break resp;
                }
            }
        };

        self.router.unregister(acp_session_id).await;
        state.saw_done = true;

        if let Some(err) = prompt_resp.get("error") {
            if is_auth_required_error(err) {
                let auth_methods = self.auth_methods.lock().await.clone();
                let _ = event_sink
                    .send(NormalizedEvent {
                        event_type: SessionEventType::AuthRequired,
                        payload_json: json!({
                            "kind": "auth_required",
                            "provider": self.agent.provider_id,
                            "message": "Provider requires authentication to continue.",
                            "auth_methods": auth_methods.clone(),
                            "authMethods": auth_methods,
                            "acp_error": err,
                        }),
                    })
                    .await;
            }
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::Error,
                    payload_json: json!({"provider": self.agent.provider_id, "acp_error": err}),
                })
                .await;
        }

        if !state.saw_assistant_complete && !state.assistant_buf.trim().is_empty() {
            state.saw_assistant_complete = true;
            let _ = event_sink
                .send(NormalizedEvent {
                    event_type: SessionEventType::AssistantComplete,
                    payload_json: json!({"full_content": state.assistant_buf}),
                })
                .await;
        }

        let stop_reason = prompt_resp
            .get("result")
            .and_then(|v| v.get("stopReason"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let _ = event_sink
            .send(NormalizedEvent {
                event_type: SessionEventType::Done,
                payload_json: json!({
                    "provider": self.agent.provider_id,
                    "acp_session_id": acp_session_id,
                    "status": if prompt_resp.get("error").is_some() { "error" } else { "success" },
                    "stop_reason": stop_reason,
                }),
            })
            .await;

        // Give the agent a short grace period to flush any last session/update notifications.
        tokio::time::sleep(Duration::from_millis(200)).await;
        Ok(())
    }

    async fn set_model(
        &self,
        acp_session_id: &str,
        model_id: String,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let resp = self
            .send_request(
                "session/set_model",
                json!({"sessionId": acp_session_id, "modelId": model_id}),
            )
            .await
            .context("waiting for session/set_model response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_model error: {err}");
        }
        Ok(())
    }

    async fn set_mode(
        &self,
        acp_session_id: &str,
        mode_id: String,
        _event_sink: mpsc::Sender<NormalizedEvent>,
    ) -> Result<()> {
        let resp = self
            .send_request(
                "session/set_mode",
                json!({"sessionId": acp_session_id, "modeId": mode_id}),
            )
            .await
            .context("waiting for session/set_mode response")?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("ACP set_mode error: {err}");
        }
        Ok(())
    }

    async fn send_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let rx = self.send_request_raw(method, params).await?;
        rx.await.context("awaiting ACP response")
    }

    async fn send_request_raw(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<oneshot::Receiver<serde_json::Value>> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let line = serde_json::to_string(&msg).context("serializing ACP request")?;
        if self.write_tx.send(line).is_err() {
            let mut pending = self.pending.lock().await;
            pending.remove(&id);
            anyhow::bail!("ACP writer task unavailable");
        }
        Ok(rx)
    }

    fn send_cancel_notification(&self, acp_session_id: &str) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": "session/cancel",
            "params": { "sessionId": acp_session_id }
        });
        let line = serde_json::to_string(&msg).context("serializing ACP cancel notification")?;
        let _ = self.write_tx.send(line);
        Ok(())
    }

    async fn is_alive(&self) -> Result<bool> {
        let mut child = self.child.lock().await;
        Ok(child.try_wait()?.is_none())
    }
}
