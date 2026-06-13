use super::*;
use ctx_core::ids::WorkspaceId;

mod fixtures;
// The encrypted mobile pairing/proxy implementation is not part of the public build.
// Public tests cover the local profile API plus secure-route rejection/validation.
mod secure_channel;
mod workspace_stream;

use fixtures::*;
use secure_channel::*;
