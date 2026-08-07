//! Server-owned extensions for operations outside the built-in data-plane path.
//!
//! Extensions return transport-neutral domain outcomes.  The shared operation
//! handler maps those outcomes through the generated Smithy status contract and
//! owns all wire response framing.  API implementations therefore do not need
//! to construct a `Response`, select a wire `Status`, or encode a sentinel.

use super::super::{KvError, types::StoredItemValue};
use openkache_protocol::Opcode;

use super::operation_handlers::OperationContext;

/// A domain value returned by a server operation extension.
///
/// Storage-backed values retain their zero-copy ownership until the shared
/// response encoder consumes them; transformed values own their output bytes.
#[derive(Clone, Debug)]
pub(super) enum ExtensionValue {
    /// A value read from storage without forcing an eager copy.
    Stored(StoredItemValue),
    /// A value produced by an API transform or a shared scalar codec.
    Bytes(Vec<u8>),
}

impl AsRef<[u8]> for ExtensionValue {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Stored(value) => value.as_ref(),
            Self::Bytes(value) => value,
        }
    }
}

/// Domain-level failures understood by the generated contract adapter.
///
/// None of the variants names a wire byte or owns a protocol response.  The
/// adapter in `operation_handlers` maps these stable meanings to the status
/// allowed by the operation's Smithy contract.
#[derive(Debug)]
pub(super) enum ExtensionError {
    /// Input bytes do not satisfy the operation's modeled domain.
    InvalidRequest(&'static [u8]),
    /// The requested namespace is not present.
    NamespaceNotFound,
    /// The caller is not authorized for this operation.
    Forbidden,
    /// A response or transformed value exceeds the operation budget.
    TooLarge,
    /// The server cannot admit the operation at this time.
    Overloaded,
    /// The operation exceeded its deadline.
    Timeout,
    /// The operation hit a storage failure whose stable mapping is shared.
    Storage(KvError),
    /// A server-side invariant or metadata update failed.
    Internal(&'static [u8]),
    /// A mutation may have crossed its linearization point; close the lane.
    AmbiguousMutation,
    /// A conditional write was rejected by the namespace policy.
    PolicyConflict,
    /// A compare-and-swap or revision precondition did not match.
    Conflict,
    /// A namespace cannot be removed while it still owns items.
    NamespaceNotEmpty,
    /// Admission failed because protected items cannot be evicted.
    NoCapacity,
}

/// A generated response field sequence before wire framing.
///
/// Values are kept in Smithy plan order.  The shared encoder validates the
/// cardinality and requiredness against the generated response descriptor.
pub(super) struct FieldSequenceResponse {
    values: Vec<Option<ExtensionValue>>,
}

impl FieldSequenceResponse {
    /// Creates a response sequence in generated field order.
    pub(super) fn new(values: Vec<Option<ExtensionValue>>) -> Self {
        Self { values }
    }

    /// Returns the ordered domain values for the shared response encoder.
    pub(super) fn values(&self) -> &[Option<ExtensionValue>] {
        &self.values
    }
}

/// Typed builder for a generated response field sequence.
///
/// The builder only constructs domain bytes; the shared response encoder
/// still performs requiredness, cardinality, and aggregate-size validation.
pub(super) struct FieldSequenceBuilder {
    values: Vec<Option<ExtensionValue>>,
}

impl FieldSequenceBuilder {
    /// Allocates one slot per generated response field.
    pub(super) fn new(opcode: Opcode) -> Self {
        let count = crate::contract::operation_response_field_count(opcode);
        Self {
            values: (0..count).map(|_| None).collect(),
        }
    }

    /// Sets an already encoded field value.
    pub(super) fn set(
        &mut self,
        index: usize,
        value: Option<ExtensionValue>,
    ) -> std::result::Result<(), ExtensionError> {
        let Some(slot) = self.values.get_mut(index) else {
            return Err(ExtensionError::Internal(
                b"response field index is outside the generated plan",
            ));
        };
        *slot = value;
        Ok(())
    }

    /// Sets one opaque field value without copying its caller-owned allocation.
    pub(super) fn set_bytes(
        &mut self,
        index: usize,
        value: Vec<u8>,
    ) -> std::result::Result<(), ExtensionError> {
        self.set(index, Some(ExtensionValue::Bytes(value)))
    }

    /// Sets one canonical big-endian unsigned 64-bit field.
    pub(super) fn set_u64(
        &mut self,
        index: usize,
        value: u64,
    ) -> std::result::Result<(), ExtensionError> {
        self.set_bytes(index, super::operation_codecs::encode_u64_be(value))
    }

    /// Sets one canonical big-endian signed 32-bit field.
    pub(super) fn set_i32(
        &mut self,
        index: usize,
        value: i32,
    ) -> std::result::Result<(), ExtensionError> {
        self.set_bytes(index, super::operation_codecs::encode_i32_be(value))
    }

    /// Sets one canonical big-endian binary64 field.
    pub(super) fn set_f64(
        &mut self,
        index: usize,
        value: f64,
    ) -> std::result::Result<(), ExtensionError> {
        self.set_bytes(index, super::operation_codecs::encode_f64_be(value))
    }

    /// Sets one canonical one-byte boolean field.
    pub(super) fn set_bool(
        &mut self,
        index: usize,
        value: bool,
    ) -> std::result::Result<(), ExtensionError> {
        self.set_bytes(index, super::operation_codecs::encode_bool(value))
    }

    /// Finishes the domain response.  The shared encoder validates the output
    /// against `opcode`'s generated requiredness and cardinality plan.
    pub(super) fn finish(self) -> ExtensionResponse {
        ExtensionResponse::field_sequence_values(self.values)
    }
}

/// Domain success statuses understood by the generated contract adapter.
#[derive(Clone, Copy, Debug)]
pub(super) enum ExtensionSuccessStatus {
    /// The ordinary successful result.
    Ok,
    /// A newly created resource or item.
    Created,
    /// An existing resource or item was replaced.
    Replaced,
    /// A resource or item was deleted.
    Deleted,
    /// A conditional mutation did not store a value.
    NotStored,
    /// A requested item was absent.
    NotFound,
}

/// Domain payload returned by a successful operation.
pub(super) enum ExtensionPayload {
    /// One opaque application payload.
    ApplicationValue(Vec<u8>),
    /// One ordered output field sequence.
    FieldSequence(FieldSequenceResponse),
}

/// A response produced by an API-owned behavior implementation.
///
/// The variants deliberately describe domain output only.  In particular,
/// errors are not pre-encoded responses, which keeps wire status selection in
/// one generated adapter.
pub(super) enum ExtensionResponse {
    /// A successful domain result and its transport-neutral status.
    Success {
        status: ExtensionSuccessStatus,
        payload: ExtensionPayload,
    },
    /// A domain-level failure to be mapped by the shared contract adapter.
    Error(ExtensionError),
    /// A mutation whose outcome is intentionally ambiguous.
    Abandoned,
}

impl ExtensionResponse {
    /// Creates a successful application-value response.
    pub(super) fn application_value(value: Vec<u8>) -> Self {
        Self::Success {
            status: ExtensionSuccessStatus::Ok,
            payload: ExtensionPayload::ApplicationValue(value),
        }
    }

    /// Creates a successful field-sequence response.
    pub(super) fn field_sequence(values: Vec<Option<ExtensionValue>>) -> Self {
        Self::field_sequence_values(values)
    }

    fn field_sequence_values(values: Vec<Option<ExtensionValue>>) -> Self {
        Self::Success {
            status: ExtensionSuccessStatus::Ok,
            payload: ExtensionPayload::FieldSequence(FieldSequenceResponse::new(values)),
        }
    }

    /// Creates a successful result with an explicit domain status.
    pub(super) fn success(status: ExtensionSuccessStatus, payload: ExtensionPayload) -> Self {
        Self::Success { status, payload }
    }

    /// Creates a domain validation failure.
    pub(super) fn invalid_request(message: &'static [u8]) -> Self {
        Self::Error(ExtensionError::InvalidRequest(message))
    }
}

/// One synchronous application-value implementation.
///
/// A table slot is indexed by the generated dense opcode index.  It avoids a
/// per-request string lookup or a linear search through operation names while
/// keeping behavior registration outside the generated protocol contract.
pub(super) type ApplicationValueHandler =
    fn(Vec<u8>) -> std::result::Result<Vec<u8>, ExtensionError>;

/// One application-value behavior and the codecs it promises to implement.
///
/// Codec names are model metadata, not a closed Rust enum.  The generated
/// contract validator compares them during server startup so a renamed or
/// asymmetric Smithy codec cannot silently reach the wrong handler.
#[derive(Clone, Copy)]
pub(super) struct ApplicationValueExtension {
    pub(super) opcode: Opcode,
    pub(super) request_codec: &'static str,
    pub(super) response_codec: &'static str,
    pub(super) handler: ApplicationValueHandler,
}

/// Static operation extension table owned by a server API layer.
///
/// The parent foundation exposes the table shape; a stacked API PR supplies
/// its behavior entries.  The array is intentionally fixed-size and
/// allocation-free on the hot path.
pub(super) struct ExtensionTable {
    application_values: [Option<ApplicationValueHandler>; Opcode::COUNT],
    application_codecs: [Option<(&'static str, &'static str)>; Opcode::COUNT],
}

impl ExtensionTable {
    /// Creates an empty extension table.
    pub(super) const fn empty() -> Self {
        Self {
            application_values: [None; Opcode::COUNT],
            application_codecs: [None; Opcode::COUNT],
        }
    }

    /// Builds a table from generated opcode-indexed behavior entries.
    pub(super) const fn with_application_values(entries: &[ApplicationValueExtension]) -> Self {
        let mut table = Self::empty();
        let mut index = 0;
        while index < entries.len() {
            let entry = entries[index];
            table.application_values[entry.opcode.index()] = Some(entry.handler);
            table.application_codecs[entry.opcode.index()] =
                Some((entry.request_codec, entry.response_codec));
            index += 1;
        }
        table
    }

    /// Registers one application-value behavior before the server starts.
    pub(super) fn register_application_value(
        &mut self,
        opcode: Opcode,
        request_codec: &'static str,
        response_codec: &'static str,
        handler: ApplicationValueHandler,
    ) {
        self.application_values[opcode.index()] = Some(handler);
        self.application_codecs[opcode.index()] = Some((request_codec, response_codec));
    }

    /// Executes one application-value behavior without a linear registry scan.
    pub(super) fn application_value(
        &self,
        opcode: Opcode,
        value: Vec<u8>,
    ) -> Option<ExtensionResponse> {
        let handler = self.application_values[opcode.index()]?;
        Some(match handler(value) {
            Ok(value) => ExtensionResponse::application_value(value),
            Err(error) => ExtensionResponse::Error(error),
        })
    }

    /// Reports whether a synchronous behavior is registered for this opcode.
    pub(super) fn handles_application_value(&self, opcode: Opcode) -> bool {
        self.application_values[opcode.index()].is_some()
    }

    /// Verifies every registered application behavior against the generated
    /// request/response codec declarations.
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        for opcode in Opcode::ALL {
            let Some((request_codec, response_codec)) = self.application_codecs[opcode.index()]
            else {
                continue;
            };
            validate_operation_descriptor(
                opcode,
                crate::contract::OperationRequestRoute::ApplicationValue,
                crate::contract::OperationResponseRoute::ApplicationValue,
                1,
                1,
                &[request_codec],
                &[response_codec],
            )?;
        }
        Ok(())
    }
}

/// Verifies one API-owned behavior descriptor against Smithy-derived metadata.
///
/// This is called at server bind/startup, never per request.  It keeps codec
/// capability and route/cardinality declarations synchronized without adding a
/// closed operation enum to the server infrastructure.
pub(super) fn validate_operation_descriptor(
    opcode: Opcode,
    request_route: crate::contract::OperationRequestRoute,
    response_route: crate::contract::OperationResponseRoute,
    request_field_count: usize,
    response_field_count: usize,
    request_codecs: &[&str],
    response_codecs: &[&str],
) -> Result<(), &'static str> {
    let contract = crate::contract::operation_contract(opcode);
    if contract.request_route != request_route || contract.response_route != response_route {
        return Err("extension route does not match its generated operation plan");
    }
    if contract.request_plan.len() != request_field_count
        || contract.response_plan.len() != response_field_count
    {
        return Err("extension field cardinality does not match its generated operation plan");
    }
    if request_codecs.iter().any(|codec| {
        !contract
            .request_plan
            .iter()
            .any(|field| field.codecs.contains(codec))
    }) {
        return Err("extension request codec does not match its generated operation plan");
    }
    if response_codecs.iter().any(|codec| {
        !contract
            .response_plan
            .iter()
            .any(|field| field.codecs.contains(codec))
    }) {
        return Err("extension response codec does not match its generated operation plan");
    }
    Ok(())
}

/// The default table for the parent foundation.
///
/// A dependent API branch can replace this constant with its own static table
/// while retaining the same transport and response adapter.
pub(super) const APPLICATION_VALUE_EXTENSIONS: ExtensionTable = ExtensionTable::empty();

/// Executes a non-immediate operation extension.
///
/// The parent foundation has no API-owned behavior.  A dependent API branch
/// overrides this hook for its asynchronous storage operations; the shared
/// caller and result adapter remain unchanged.
pub(super) async fn execute(_context: &OperationContext<'_, '_>) -> Option<ExtensionResponse> {
    None
}

/// Executes a registered application-value operation.
pub(super) fn application_value(opcode: Opcode, value: Vec<u8>) -> Option<ExtensionResponse> {
    APPLICATION_VALUE_EXTENSIONS.application_value(opcode, value)
}

/// Reports whether this extension owns an operation outside the built-in path.
pub(super) fn handles(opcode: Opcode) -> bool {
    APPLICATION_VALUE_EXTENSIONS.handles_application_value(opcode)
}

/// Validates the behavior table against generated operation metadata.
pub(super) fn validate_registry() -> Result<(), &'static str> {
    APPLICATION_VALUE_EXTENSIONS.validate()
}
