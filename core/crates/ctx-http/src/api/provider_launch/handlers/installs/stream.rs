use super::*;

pub(in crate::api) async fn install_stream_sse(
    State(state): State<Arc<AppState>>,
    Path(install_id): Path<String>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, axum::Error>>>, StatusCode> {
    let install_id: InstallId =
        uuid::Uuid::parse_str(&install_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    let Some(sender) = state.get_install_sender(install_id).await else {
        return Err(StatusCode::NOT_FOUND);
    };

    let history = state
        .get_install_events(install_id)
        .await
        .unwrap_or_default();
    let initial = futures::stream::iter(history.into_iter().map(|ev| {
        let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
        Ok::<_, axum::Error>(SseEvent::default().event("progress").data(payload))
    }));

    let live = futures::stream::unfold(sender.subscribe(), move |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let payload = serde_json::to_string(&ev).unwrap_or_else(|_| "{}".into());
                    return Some((Ok(SseEvent::default().event("progress").data(payload)), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let stream = initial.chain(live);

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}
