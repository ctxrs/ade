use super::*;

use self::records::build_session_artifacts;

#[path = "set/records.rs"]
mod records;

#[derive(Debug, Deserialize)]
struct ArtifactInput {
    absolute_file_path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(in crate::api) struct SetSessionArtifactsReq {
    #[serde(default)]
    artifacts: Vec<ArtifactInput>,
}

pub(in crate::api) async fn set_session_artifacts(
    State(state): State<Arc<AppState>>,
    mcp_auth: Option<Extension<crate::daemon::McpAuthContext>>,
    Path(id): Path<String>,
    Json(req): Json<SetSessionArtifactsReq>,
) -> Result<Json<Vec<Artifact>>, (StatusCode, Json<ApiErrorResp>)> {
    let session_id = SessionId(uuid::Uuid::parse_str(&id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiErrorResp {
                error: "invalid session id".to_string(),
            }),
        )
    })?);

    if let Some(Extension(mcp_auth)) = mcp_auth {
        validate_scoped_mcp_session_context(&state, mcp_auth, session_id).await?;
    }

    let store = state.store_for_session(session_id).await.map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: logs::redact_sensitive(&e.to_string()),
            }),
        )
    })?;
    let session = store
        .get_session(session_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResp {
                error: "session not found".to_string(),
            }),
        ))?;

    let artifacts = build_session_artifacts(&state, &store, &session, req.artifacts).await?;

    store
        .replace_session_artifacts(session.id, &artifacts)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    let event = store
        .append_session_event(
            session.id,
            None,
            None,
            SessionEventType::ArtifactsSet,
            serde_json::json!({ "artifacts": artifacts }),
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiErrorResp {
                    error: logs::redact_sensitive(&e.to_string()),
                }),
            )
        })?;
    state.publish_event(event).await;

    Ok(Json(artifacts))
}
