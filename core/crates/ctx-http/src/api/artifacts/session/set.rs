use super::*;
use ctx_daemon::daemon::SessionsHandle;

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
    State(state): State<SessionsHandle>,
    mcp_auth: Option<Extension<ctx_mcp_auth::McpAuthContext>>,
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

    let artifacts = state
        .set_session_artifacts_for_route(
            session_id,
            req.artifacts
                .into_iter()
                .map(|artifact| SessionArtifactInput {
                    absolute_file_path: artifact.absolute_file_path,
                    name: artifact.name,
                    mime_type: artifact.mime_type,
                })
                .collect(),
        )
        .await
        .map_err(session_artifact_api_error)?;

    Ok(Json(artifacts))
}
