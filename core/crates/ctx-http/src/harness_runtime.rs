//! Compatibility shim for the old `harness_runtime` name.
//!
//! New architecture work should prefer `workspace_runtime`, which is the clearer term for the
//! workspace/container execution environment owned by ctx. This module remains so the rest of the
//! crate can migrate without a repo-wide import rewrite in the same changeset.

pub use crate::workspace_runtime::*;
