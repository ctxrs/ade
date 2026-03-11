#[derive(Clone, Copy, Debug)]
enum TerminalSide {
    Daemon,
    Worker,
}

enum TerminalRelayAction {
    Forward(mpsc::UnboundedSender<Message>, Message),
    Queued,
    Drop,
}

fn attach_terminal_side(
    relay_state: &mut TerminalRelayState,
    terminal_id: &str,
    side: TerminalSide,
    tx: mpsc::UnboundedSender<Message>,
) -> Vec<Message> {
    let entry = relay_state
        .sessions
        .entry(terminal_id.to_string())
        .or_default();
    match side {
        TerminalSide::Daemon => entry.daemon_tx = Some(tx),
        TerminalSide::Worker => entry.worker_tx = Some(tx),
    }
    match side {
        TerminalSide::Daemon => entry.pending_for_daemon.drain(..).collect::<Vec<_>>(),
        TerminalSide::Worker => entry.pending_for_worker.drain(..).collect::<Vec<_>>(),
    }
}

fn route_terminal_message(
    relay_state: &mut TerminalRelayState,
    terminal_id: &str,
    side: TerminalSide,
    msg: Message,
) -> TerminalRelayAction {
    let Some(entry) = relay_state.sessions.get_mut(terminal_id) else {
        return TerminalRelayAction::Drop;
    };
    let other_tx = match side {
        TerminalSide::Daemon => entry.worker_tx.clone(),
        TerminalSide::Worker => entry.daemon_tx.clone(),
    };
    if let Some(tx) = other_tx {
        return TerminalRelayAction::Forward(tx, msg);
    }
    let pending = match side {
        TerminalSide::Daemon => &mut entry.pending_for_worker,
        TerminalSide::Worker => &mut entry.pending_for_daemon,
    };
    if pending.len() >= MAX_PENDING_TERMINAL_MESSAGES {
        pending.pop_front();
    }
    pending.push_back(msg);
    TerminalRelayAction::Queued
}

fn detach_terminal_side(relay_state: &mut TerminalRelayState, terminal_id: &str, side: TerminalSide) {
    if let Some(entry) = relay_state.sessions.get_mut(terminal_id) {
        match side {
            TerminalSide::Daemon => entry.daemon_tx = None,
            TerminalSide::Worker => entry.worker_tx = None,
        }
        if entry.daemon_tx.is_none() && entry.worker_tx.is_none() {
            relay_state.sessions.remove(terminal_id);
        }
    }
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
        attach_terminal_side(&mut relay_guard, &terminal_id, side, tx)
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
        detach_terminal_side(&mut relay_guard, &terminal_id_for_sender, side);
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
        let action = {
            let mut relay_guard = relay.lock().await;
            if let Some(msg) = msg.take() {
                let action = route_terminal_message(&mut relay_guard, &terminal_id, side, msg);
                if matches!(action, TerminalRelayAction::Queued) {
                    debug!(
                        worker_id = %worker_id,
                        terminal_id = %terminal_id,
                        side = ?side,
                        "queued terminal data"
                    );
                }
                action
            } else {
                TerminalRelayAction::Drop
            }
        };
        if let TerminalRelayAction::Forward(tx, msg) = action {
            let _ = tx.send(msg);
        }
    }

    let _ = send_task.await;
    let mut relay_guard = relay.lock().await;
    detach_terminal_side(&mut relay_guard, &terminal_id, side);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_message(body: &str) -> Message {
        Message::Binary(body.as_bytes().to_vec())
    }

    fn message_body(msg: &Message) -> String {
        match msg {
            Message::Binary(data) => String::from_utf8(data.to_vec()).expect("binary utf8"),
            Message::Text(text) => text.to_string(),
            other => panic!("unexpected message variant: {other:?}"),
        }
    }

    #[test]
    fn reconnecting_side_receives_queued_terminal_data_in_order() {
        let (daemon_tx, _daemon_rx) = mpsc::unbounded_channel();
        let mut relay_state = TerminalRelayState::default();
        let pending = attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Daemon, daemon_tx);
        assert!(pending.is_empty());

        for body in ["first", "second", "third"] {
            assert!(matches!(
                route_terminal_message(
                    &mut relay_state,
                    "term-1",
                    TerminalSide::Daemon,
                    binary_message(body),
                ),
                TerminalRelayAction::Queued
            ));
        }

        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        let pending = attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Worker, worker_tx);
        let received = pending.iter().map(message_body).collect::<Vec<_>>();
        assert_eq!(received, vec!["first", "second", "third"]);
        assert!(relay_state.sessions["term-1"].pending_for_worker.is_empty());
    }

    #[test]
    fn terminal_pending_queue_is_bounded_and_drops_oldest_messages() {
        let (daemon_tx, _daemon_rx) = mpsc::unbounded_channel();
        let mut relay_state = TerminalRelayState::default();
        attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Daemon, daemon_tx);

        for idx in 0..(MAX_PENDING_TERMINAL_MESSAGES + 2) {
            assert!(matches!(
                route_terminal_message(
                    &mut relay_state,
                    "term-1",
                    TerminalSide::Daemon,
                    binary_message(&format!("msg-{idx}")),
                ),
                TerminalRelayAction::Queued
            ));
        }

        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        let pending = attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Worker, worker_tx);
        assert_eq!(pending.len(), MAX_PENDING_TERMINAL_MESSAGES);
        assert_eq!(message_body(&pending[0]), "msg-2");
        assert_eq!(
            message_body(pending.last().expect("last pending message")),
            format!("msg-{}", MAX_PENDING_TERMINAL_MESSAGES + 1)
        );
    }

    #[test]
    fn terminal_message_forwards_immediately_when_both_sides_are_connected() {
        let (daemon_tx, _daemon_rx) = mpsc::unbounded_channel();
        let (worker_tx, mut worker_rx) = mpsc::unbounded_channel();
        let mut relay_state = TerminalRelayState::default();
        attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Daemon, daemon_tx);
        attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Worker, worker_tx);

        let action = route_terminal_message(
            &mut relay_state,
            "term-1",
            TerminalSide::Daemon,
            binary_message("live"),
        );
        match action {
            TerminalRelayAction::Forward(tx, msg) => {
                tx.send(msg).expect("worker receiver available");
            }
            TerminalRelayAction::Queued => panic!("message should forward when worker is connected"),
            TerminalRelayAction::Drop => panic!("message should not drop when session exists"),
        }

        let received = worker_rx.try_recv().expect("forwarded message");
        assert_eq!(message_body(&received), "live");
        assert!(relay_state.sessions["term-1"].pending_for_worker.is_empty());
    }

    #[test]
    fn relay_session_is_removed_after_both_sides_disconnect() {
        let (daemon_tx, _daemon_rx) = mpsc::unbounded_channel();
        let (worker_tx, _worker_rx) = mpsc::unbounded_channel();
        let mut relay_state = TerminalRelayState::default();
        attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Daemon, daemon_tx);
        attach_terminal_side(&mut relay_state, "term-1", TerminalSide::Worker, worker_tx);

        detach_terminal_side(&mut relay_state, "term-1", TerminalSide::Daemon);
        assert!(relay_state.sessions.contains_key("term-1"));

        detach_terminal_side(&mut relay_state, "term-1", TerminalSide::Worker);
        assert!(!relay_state.sessions.contains_key("term-1"));
    }
}
