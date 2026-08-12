//! Status tokens owned by the protocol-v1 compatibility adapter.
//!
//! Namespace and SET meanings stay behind this adapter. The generic dispatcher
//! only receives the opaque status token carried by an outcome.

use openkache_protocol::Status;
use super::operation_outcome::StatusToken;

pub(super) const OK: StatusToken = StatusToken::new(Status::Ok as u8);
pub(super) const NOT_FOUND: StatusToken = StatusToken::new(Status::NotFound as u8);
pub(super) const CREATED: StatusToken = StatusToken::new(Status::Created as u8);
pub(super) const REPLACED: StatusToken = StatusToken::new(Status::Replaced as u8);
pub(super) const DELETED: StatusToken = StatusToken::new(Status::Deleted as u8);
pub(super) const NOT_STORED: StatusToken = StatusToken::new(Status::NotStored as u8);
pub(super) const INVALID_REQUEST: StatusToken =
    StatusToken::new(Status::InvalidRequest as u8);
pub(super) const TOO_LARGE: StatusToken = StatusToken::new(Status::TooLarge as u8);
pub(super) const OVERLOADED: StatusToken = StatusToken::new(Status::Overloaded as u8);
pub(super) const TIMEOUT: StatusToken = StatusToken::new(Status::Timeout as u8);
pub(super) const INTERNAL_ERROR: StatusToken =
    StatusToken::new(Status::InternalError as u8);
pub(super) const NO_CAPACITY: StatusToken = StatusToken::new(Status::NoCapacity as u8);
pub(super) const POLICY_CONFLICT: StatusToken =
    StatusToken::new(Status::PolicyConflict as u8);
pub(super) const CONFLICT: StatusToken = StatusToken::new(Status::Conflict as u8);
pub(super) const NAMESPACE_NOT_FOUND: StatusToken =
    StatusToken::new(Status::NamespaceNotFound as u8);
pub(super) const NAMESPACE_NOT_EMPTY: StatusToken =
    StatusToken::new(Status::NamespaceNotEmpty as u8);
