#[derive(Clone, Copy, Debug)]
enum TerminalSide {
    Daemon,
    Worker,
}

async fn handle_terminal_data_socket(
    state: AppState,
    worker_id: String,
    terminal_id: String,
    side: TerminalSide,
    socket: WebSocket,
) {
    let relay = terminal_relay_for(&state, &worker_id).await;
    debug!(
        worker_id = %worker_id,
        terminal_id = %terminal_id,
        side = ?side,
        "terminal data connected"
    );
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let tx_for_pending = tx.clone();
    let tx_for_ping = tx.clone();
    let pending = {
        let mut relay_guard = relay.lock().await;
        let entry = relay_guard
            .sessions
            .entry(terminal_id.clone())
            .or_insert_with(TerminalSessionRelay::default);
        match side {
            TerminalSide::Daemon => entry.daemon_tx = Some(tx),
            TerminalSide::Worker => entry.worker_tx = Some(tx),
        }
        match side {
            TerminalSide::Daemon => entry.pending_for_daemon.drain(..).collect::<Vec<_>>(),
            TerminalSide::Worker => entry.pending_for_worker.drain(..).collect::<Vec<_>>(),
        }
    };
    for msg in pending {
        let _ = tx_for_pending.send(msg);
    }

    let relay_for_sender = relay.clone();
    let terminal_id_for_sender = terminal_id.clone();
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
        let mut relay_guard = relay_for_sender.lock().await;
        if let Some(entry) = relay_guard.sessions.get_mut(&terminal_id_for_sender) {
            match side {
                TerminalSide::Daemon => entry.daemon_tx = None,
                TerminalSide::Worker => entry.worker_tx = None,
            }
            if entry.daemon_tx.is_none() && entry.worker_tx.is_none() {
                relay_guard.sessions.remove(&terminal_id_for_sender);
            }
        }
    });

    while let Some(Ok(msg)) = receiver.next().await {
        let mut msg = match msg {
            Message::Ping(payload) => {
                let _ = tx_for_ping.send(Message::Pong(payload));
                None
            }
            Message::Pong(_) => None,
            Message::Text(_) | Message::Binary(_) | Message::Close(_) => Some(msg),
        };
        let other_tx = {
            let mut relay_guard = relay.lock().await;
            let Some(entry) = relay_guard.sessions.get_mut(&terminal_id) else {
                msg.take();
                continue;
            };
            let other_tx = match side {
                TerminalSide::Daemon => entry.worker_tx.clone(),
                TerminalSide::Worker => entry.daemon_tx.clone(),
            };
            if other_tx.is_none() {
                let pending = match side {
                    TerminalSide::Daemon => &mut entry.pending_for_worker,
                    TerminalSide::Worker => &mut entry.pending_for_daemon,
                };
                if let Some(msg) = msg.take() {
                    if pending.len() >= MAX_PENDING_TERMINAL_MESSAGES {
                        pending.pop_front();
                    }
                    pending.push_back(msg);
                    debug!(
                        worker_id = %worker_id,
                        terminal_id = %terminal_id,
                        side = ?side,
                        "queued terminal data"
                    );
                }
            }
            other_tx
        };
        if let (Some(tx), Some(msg)) = (other_tx, msg) {
            let _ = tx.send(msg);
        }
    }

    let _ = send_task.await;
    let mut relay_guard = relay.lock().await;
    if let Some(entry) = relay_guard.sessions.get_mut(&terminal_id) {
        match side {
            TerminalSide::Daemon => entry.daemon_tx = None,
            TerminalSide::Worker => entry.worker_tx = None,
        }
        if entry.daemon_tx.is_none() && entry.worker_tx.is_none() {
            relay_guard.sessions.remove(&terminal_id);
        }
    }
}

async fn relay_for(state: &AppState, worker_id: &str) -> Arc<Mutex<RelayState>> {
    let mut relays = state.relays.write().await;
    relays
        .entry(worker_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(RelayState::new())))
        .clone()
}

async fn terminal_relay_for(state: &AppState, worker_id: &str) -> Arc<Mutex<TerminalRelayState>> {
    let mut relays = state.terminal_relays.write().await;
    relays
        .entry(worker_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(TerminalRelayState::default())))
        .clone()
}

async fn update_state(state: &AppState, worker_id: &str, new_state: WorkerState) {
    let mut workers = state.store.workers.write().await;
    if let Some(record) = workers.get_mut(worker_id) {
        record.state = new_state;
        record.updated_at = Utc::now();
    }
}

async fn update_state_with_ssh(
    state: &AppState,
    worker_id: &str,
    new_state: WorkerState,
    ssh: Option<ctx_worker_protocol::SshInfo>,
) {
    let mut workers = state.store.workers.write().await;
    if let Some(record) = workers.get_mut(worker_id) {
        record.state = new_state;
        if ssh.is_some() {
            record.ssh = ssh;
        }
        record.updated_at = Utc::now();
    }
}

async fn ensure_exists(
    state: &AppState,
    worker_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    workers.get(worker_id).ok_or_else(not_found)?;
    Ok(())
}

async fn get_request_spec(
    state: &AppState,
    worker_id: &str,
) -> Result<StartWorkerRequest, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(worker_id).ok_or_else(not_found)?;

    Ok(record.spec.clone())
}

async fn get_base_commit(
    state: &AppState,
    worker_id: &str,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
    let workers = state.store.workers.read().await;
    let record = workers.get(worker_id).ok_or_else(not_found)?;
    Ok(record.base_commit_sha.clone())
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new("worker_not_found")),
    )
}

#[derive(Debug, serde::Serialize)]
struct ErrorResponse {
    error: String,
}

impl ErrorResponse {
    fn new(error: &str) -> Self {
        Self {
            error: error.to_string(),
        }
    }
}
