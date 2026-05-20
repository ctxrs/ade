mod common;
mod control;
mod read_models;
mod title_model_mode;

pub use common::{parse_session_route_id, SessionRouteParams, SessionTurnToolsRouteParams};
pub use control::{
    AuthenticateSessionRouteRequest, SessionControlRouteError, SessionControlRouteErrorKind,
    SessionFileCompletionsRouteQuery, SessionFileCompletionsRouteResponse,
    SubmitAskUserQuestionRouteRequest, SubmitAskUserQuestionRouteResponse,
};
pub use read_models::{
    parse_boolish_flag, parse_session_id, parse_turn_id, SessionEventsRouteQuery,
    SessionEventsRouteResponse, SessionHeadRouteQuery, SessionHeadRouteResponse,
    SessionHistoryRouteQuery, SessionHistoryRouteResponse, SessionReadModelRouteError,
    SessionReadModelRouteErrorKind, SessionSnapshotRouteQuery, SessionSnapshotRouteResponse,
    SessionStateRouteResponse, SessionTurnToolsRouteResponse, SESSION_EVENTS_DEFAULT_LIMIT,
    SESSION_EVENTS_MAX_LIMIT,
};
pub use title_model_mode::{
    GenerateSessionTitleRouteRequest, GenerateSessionTitleRouteResponse,
    SessionTitleModelModeRouteError, SessionTitleModelModeRouteErrorKind,
    SetSessionModeRouteRequest, SetSessionModelRouteRequest, SetSessionModelRouteResponse,
};

#[cfg(test)]
mod tests;
