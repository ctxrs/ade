//! Compatibility shim for the old `provider_probe` module name.
//!
//! New architecture work should use `provider_launch::probe`, which more accurately describes the
//! domain boundary around provider launch/probe/auth runtime preparation.

pub(crate) use crate::provider_launch::probe::*;
