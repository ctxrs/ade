mod common;
mod creation;
mod lifecycle;
mod listing;
mod responses;

#[cfg(test)]
use common::route_error_kind_for_internal_error;
#[cfg(test)]
mod tests;
