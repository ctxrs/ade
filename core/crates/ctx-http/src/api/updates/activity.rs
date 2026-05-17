use super::*;
use ctx_daemon::daemon::{CoreHandle, UpdateActivitySnapshot};

pub(in crate::api) async fn update_activity(
    State(core): State<CoreHandle>,
) -> Result<Json<UpdateActivitySnapshot>, (StatusCode, Json<ApiErrorResp>)> {
    core.update_activity_snapshot()
        .await
        .map(Json)
        .map_err(update_route_error)
}
