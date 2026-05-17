use std::time::Instant;

use ctx_core::ids::SessionId;
use ctx_core::models::SessionHeadSnapshot;

use crate::daemon::SessionsHandle;

const SESSION_HEAD_CACHE: &str = "session_head";

#[derive(Debug, Clone, Copy)]
pub struct SessionHeadRouteRequest {
    pub session_id: SessionId,
    pub limit: u32,
    pub include_events: bool,
    pub min_event_seq: Option<i64>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SessionHeadRouteError {
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

impl SessionsHandle {
    pub async fn session_head_for_route(
        &self,
        req: SessionHeadRouteRequest,
    ) -> Result<SessionHeadSnapshot, SessionHeadRouteError> {
        let started_at = Instant::now();
        if matches!(req.min_event_seq, Some(value) if value < 0) {
            return Err(SessionHeadRouteError::BadRequest);
        }

        let workspace_id = match self.workspace_id_for_session(req.session_id).await {
            Ok(Some(workspace_id)) => workspace_id,
            Ok(None) => return Err(SessionHeadRouteError::NotFound),
            Err(_) => return Err(SessionHeadRouteError::Internal),
        };
        if self.is_workspace_deleting(workspace_id).await {
            return Err(SessionHeadRouteError::NotFound);
        }

        if let Some(head) = self
            .cached_session_head_for_request(
                req.session_id,
                req.include_events,
                req.limit,
                req.min_event_seq,
            )
            .await
        {
            self.record_session_head_recovery_metrics(
                "active_snapshot_cache",
                "ok",
                started_at.elapsed(),
                req.limit,
                req.include_events,
                Some(&head),
            );
            return Ok(head);
        }

        self.emit_cache_miss(SESSION_HEAD_CACHE).await;
        match self
            .load_session_head_snapshot_from_store(req.session_id, req.limit, req.include_events)
            .await
        {
            Ok(Some(head)) => {
                if matches!(req.min_event_seq, Some(min_event_seq) if head.last_event_seq < min_event_seq)
                {
                    self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                    self.record_session_head_recovery_metrics(
                        "store_rebuild",
                        "stale",
                        started_at.elapsed(),
                        req.limit,
                        req.include_events,
                        Some(&head),
                    );
                    return Err(SessionHeadRouteError::Conflict);
                }

                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, true).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "ok",
                    started_at.elapsed(),
                    req.limit,
                    req.include_events,
                    Some(&head),
                );
                self.update_session_head_cache(head.clone(), req.include_events)
                    .await;
                Ok(head)
            }
            Ok(None) => {
                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "missing",
                    started_at.elapsed(),
                    req.limit,
                    req.include_events,
                    None,
                );
                Err(SessionHeadRouteError::NotFound)
            }
            Err(_) => {
                self.emit_cache_rehydrate(SESSION_HEAD_CACHE, false).await;
                self.record_session_head_recovery_metrics(
                    "store_rebuild",
                    "error",
                    started_at.elapsed(),
                    req.limit,
                    req.include_events,
                    None,
                );
                Err(SessionHeadRouteError::Internal)
            }
        }
    }
}
