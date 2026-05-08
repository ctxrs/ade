use super::*;
use crate::api::sessions;
use crate::api::shared;
use ctx_session_service::session_creation::{
    session_matches_creation_identity, validate_create_session_request, CreateSessionRequestError,
    CreateSessionRequestPolicy, SessionCreationIdentity,
};
use ctx_session_tools::model_resolution::{compose_model_id, resolve_model_id};

#[path = "creation_session/cleanup.rs"]
mod cleanup;
#[path = "creation_session/create.rs"]
mod create;
#[path = "creation_session/existing.rs"]
mod existing;
#[path = "creation_session/initial_prompt.rs"]
mod initial_prompt;
#[path = "creation_session/replay.rs"]
mod replay;
#[path = "creation_session/request.rs"]
mod request;
#[path = "creation_session/telemetry.rs"]
mod telemetry;
#[path = "creation_session/worktree.rs"]
mod worktree;

use cleanup::cleanup_orphaned_provisioned_worktree;
pub(super) use create::create_session_for_loaded_task_inner;
pub(in crate::api) use create::{
    create_default_session_for_task, create_session_for_task, DefaultSessionSeed,
};
use existing::{resolve_existing_requested_session, ExistingRequestedSession};
use initial_prompt::{seed_initial_prompt, InitialPromptSeed};
pub(super) use replay::{
    create_requested_default_session_for_task, replay_requested_default_session_for_task,
};
pub(in crate::api) use request::CreateSessionReq;
pub(in crate::api::tasks) use request::CreateTaskDefaultSessionReq;
use telemetry::emit_session_started_observability;
use worktree::resolve_session_worktree_for_task;
