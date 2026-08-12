//! Status tokens used by the operation-neutral example API.
//!
//! The wire status vocabulary is an adapter detail. Generic bindings consume
//! this small set of opaque tokens; a different API can provide a different
//! status map without changing the operation executor.

use super::operation_outcome::StatusToken;
use openkache_protocol::Status;

pub(super) const OK: StatusToken = StatusToken::new(Status::Ok as u8);
pub(super) const NOT_FOUND: StatusToken = StatusToken::new(Status::NotFound as u8);
pub(super) const ACCEPTED: StatusToken = StatusToken::new(Status::Accepted as u8);
pub(super) const INVALID_REQUEST: StatusToken =
    StatusToken::new(Status::InvalidRequest as u8);
pub(super) const OVERLOADED: StatusToken = StatusToken::new(Status::Overloaded as u8);
pub(super) const TIMEOUT: StatusToken = StatusToken::new(Status::Timeout as u8);
pub(super) const INTERNAL_ERROR: StatusToken =
    StatusToken::new(Status::InternalError as u8);
