//! Server-owned extensions for operations outside the built-in data-plane path.
//!
//! The parent protocol implementation intentionally provides no operation
//! behavior here. A server or stacked API PR can replace this module with its
//! own implementation while keeping framing, storage primitives, and client
//! projections unchanged.

use super::operation_handlers::OperationContext;
use super::protocol::Response;
use openkache_protocol::Opcode;

/// Domain-level result returned by a server operation extension.
///
/// Extensions return values, not wire framing. The shared operation handler
/// encodes application-value and ordered optional-value results after the
/// extension returns.
pub(super) enum ExtensionResponse {
    /// A response whose status and payload are already selected by the server
    /// behavior (for example, a storage error or authorization failure).
    Response(Response),
    /// One application payload. The shared handler adds the protocol status
    /// and value framing.
    ApplicationValue(Vec<u8>),
    /// Ordered nullable values. The shared handler owns the length prefixes
    /// and missing-value sentinel.
    OptionalValues(Vec<Option<Vec<u8>>>),
}

/// Executes a non-immediate operation extension.
pub(super) async fn execute(_context: &OperationContext<'_, '_>) -> Option<ExtensionResponse> {
    None
}

/// Executes an application-value operation extension.
pub(super) fn application_value(_opcode: Opcode, _value: Vec<u8>) -> Option<ExtensionResponse> {
    None
}

/// Reports whether this extension owns an operation that is not handled by
/// the built-in server path.
pub(super) const fn handles(_opcode: Opcode) -> bool {
    false
}
