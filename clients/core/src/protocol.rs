//! Client-owned request and domain codecs.
//!
//! The public protocol crate only finds opaque frame boundaries.  This module
//! is the Rust client's semantic adapter: it validates the modeled request
//! shapes and encodes namespace policy and SET options.

use crate::contract::operation_wire_spec;
use openkache_protocol::{MAX_VALUE_BYTES, Opcode, encode_varuint};

/// Client-adapter validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("frame is too short: expected at least {expected} bytes, got {actual}")]
    FrameTooShort { expected: usize, actual: usize },
    #[error("frame length does not match header: expected {expected} bytes, got {actual}")]
    FrameLength { expected: usize, actual: usize },
    #[error("frame length overflow")]
    FrameLengthOverflow,
    #[error("{context} uses a non-canonical vu128 encoding")]
    NonCanonicalVaruint { context: &'static str },
    #[error("{context} exceeds the supported 64-bit vu128 range")]
    VaruintOverflow { context: &'static str },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("item ID has {actual} bytes; maximum is {expected}")]
    InvalidItemIdLength { expected: usize, actual: usize },
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
    #[error("invalid operation field sequence: {0}")]
    InvalidFieldSequence(&'static str),
    #[error("invalid operation field codec: {0}")]
    InvalidFieldCodec(&'static str),
    #[error("{opcode:?} requires a fixed item/value shape ({expected_item_id}, {expected_value})")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
        expected_value: &'static str,
    },
    #[error("SET TTL must be greater than zero milliseconds")]
    InvalidSetTtl,
    #[error("SET TTL is required by ExplicitTtl")]
    MissingSetTtl,
    #[error("SET TTL is not allowed by this expiration mode")]
    UnexpectedSetTtl,
    #[error("SET options are not valid for {opcode:?}")]
    InvalidSetOptions { opcode: Opcode },
    #[error("namespace ID is missing")]
    MissingNamespaceId,
    #[error("namespace ID must be a positive non-zero u64")]
    InvalidNamespaceId,
    #[error("namespace name is invalid: {0}")]
    InvalidNamespaceName(&'static str),
    #[error("namespace policy is missing")]
    MissingNamespacePolicy,
    #[error("namespace policy is not allowed")]
    UnexpectedNamespacePolicy,
    #[error("namespace policy is invalid: {0}")]
    InvalidNamespacePolicy(&'static str),
    #[error("namespace revision must be positive")]
    InvalidRevision,
}

impl From<openkache_protocol::ProtocolError> for ProtocolError {
    fn from(error: openkache_protocol::ProtocolError) -> Self {
        match error {
            openkache_protocol::ProtocolError::UnknownOpcode(value) => Self::UnknownOpcode(value),
            openkache_protocol::ProtocolError::UnknownStatus(value) => Self::UnknownStatus(value),
            openkache_protocol::ProtocolError::FrameTooShort { expected, actual } => {
                Self::FrameTooShort { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLength { expected, actual } => {
                Self::FrameLength { expected, actual }
            }
            openkache_protocol::ProtocolError::FrameLengthOverflow => Self::FrameLengthOverflow,
            openkache_protocol::ProtocolError::NonCanonicalVaruint { context } => {
                Self::NonCanonicalVaruint { context }
            }
            openkache_protocol::ProtocolError::VaruintOverflow { context } => {
                Self::VaruintOverflow { context }
            }
            openkache_protocol::ProtocolError::ValueTooLarge { size, maximum } => {
                Self::ValueTooLarge { size, maximum }
            }
            openkache_protocol::ProtocolError::InvalidItemIdLength { expected, actual } => {
                Self::InvalidItemIdLength { expected, actual }
            }
            openkache_protocol::ProtocolError::InvalidFieldSequence(message) => {
                Self::InvalidFieldSequence(message)
            }
        }
    }
}

type Result<T> = std::result::Result<T, ProtocolError>;

#[path = "protocol_adapters.rs"]
mod adapters;
#[path = "protocol_compat_v1.rs"]
pub(crate) mod compat_v1;
#[path = "protocol_generic.rs"]
pub(crate) mod generic;
pub(crate) use self::compat_v1::{
    compact_item_count, uses_compact_item_route, uses_compact_namespace_route,
};
pub use self::compat_v1::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetCondition, SetWireOptions,
};
pub use self::generic::OperationFields;
pub(crate) use self::generic::validate_operation_field;

/// Decodes one modeled response through the generated layout selected by its
/// contract. The client execution core does not inspect operation names or
/// compatibility route metadata.
pub(crate) fn decode_operation_response_fields<'a>(
    operation: Opcode,
    payload: &'a [u8],
) -> Result<OperationFields<'a>> {
    adapters::decode_response_fields(operation, payload)
}

/// Builds one protocol-v1 request from the generated operation contract.
///
/// The caller supplies only the generic request inputs. Framing selection and
/// the compact namespace/item/SET projection stay in this adapter so the
/// client execution surface does not grow a route family for each API.
pub(crate) fn request_from_contract(
    operation: Opcode,
    namespace_id: Option<u64>,
    item_id: &[u8],
    value: Vec<u8>,
    set_options: crate::SetOptions,
) -> crate::Result<Request> {
    adapters::request_from_contract(operation, namespace_id, item_id, value, set_options)
}

/// Builds a request for a generic unary operation from its already encoded
/// body.
///
/// This is the neutral client boundary: callers provide only the opcode and
/// the operation body. Namespace IDs, item IDs, and SET options belong to the
/// protocol-v1 compatibility adapter and are intentionally unavailable here;
/// an exact-plan opcode requires modeled fields instead of one opaque body.
pub(crate) fn request_from_unary(operation: Opcode, body: Vec<u8>) -> crate::Result<Request> {
    adapters::request_from_unary(operation, body)
}

/// Builds a generic or exact request from modeled values in generated field
/// order.
pub(crate) fn request_from_fields(
    operation: Opcode,
    fields: Vec<Option<Vec<u8>>>,
) -> crate::Result<Request> {
    adapters::request_from_fields(operation, fields)
}

/// A validated request owned by the Rust client adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestRetryPolicy {
    /// The request is safe to replay after a connection failure.
    Always,
    /// The request must not be replayed automatically.
    Never,
}

/// Applies the Smithy-generated replay declaration to an owned request.
///
/// Retry safety is attached when the operation adapter constructs the request;
/// the transport only consumes this closed policy and never branches on an
/// operation name or domain-specific mutation. The declaration lives in the
/// client metadata envelope, outside the normative wire layout.
pub(crate) fn generated_retry_policy(
    opcode: Opcode,
    create_if_missing: bool,
) -> RequestRetryPolicy {
    match crate::contract::operation_client_projection(opcode).retry_mode {
        crate::contract::OperationRetryMode::Always => RequestRetryPolicy::Always,
        crate::contract::OperationRetryMode::Never => RequestRetryPolicy::Never,
        crate::contract::OperationRetryMode::WhenNotCreating => {
            if create_if_missing {
                RequestRetryPolicy::Never
            } else {
                RequestRetryPolicy::Always
            }
        }
    }
}

impl RequestRetryPolicy {
    pub(crate) const fn is_safe(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// One validated generic request envelope.
///
/// Protocol-v1 namespace/item/SET fields are assembled and validated by the
/// compatibility adapter before they enter this type. The generic request
/// core therefore carries only generated framing bytes and an owned body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub(crate) opcode: Opcode,
    encoded_prefix: Option<Vec<u8>>,
    pub(crate) value: Vec<u8>,
    pub(crate) retry_policy: RequestRetryPolicy,
}

/// Owned request pieces ready for a transport write.
///
/// Keeping the protocol-v1 prefix separate from the already-owned payload
/// avoids shifting or copying the payload merely to prepend framing.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RequestParts {
    pub(crate) prefix: Vec<u8>,
    pub(crate) payload: Vec<u8>,
}

impl Request {
    /// Builds a route-less request from an already encoded generic body.
    pub(crate) fn new_generic(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        Self::new_generic_with_retry_policy(opcode, value, generated_retry_policy(opcode, false))
    }

    pub(crate) fn new_generic_with_retry_policy(
        opcode: Opcode,
        value: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            encoded_prefix: None,
            value,
            retry_policy,
        };
        request.validate()?;
        Ok(request)
    }

    /// Constructs an ordered request after its generated field plan has
    /// already validated the payload. Keeping this private avoids validating
    /// and allocating field offsets a second time in the client adapter.
    fn new_ordered_unchecked(opcode: Opcode, value: Vec<u8>) -> Self {
        Self {
            opcode,
            encoded_prefix: None,
            value,
            retry_policy: generated_retry_policy(opcode, false),
        }
    }

    /// Constructs an exact-plan request from generic modeled field values.
    ///
    /// The caller has already validated every field and encoded the generated
    /// metadata prefix. Keeping the trailing payload separate preserves the
    /// same two-buffer transport path used by the typed compatibility facade.
    fn new_exact_unchecked(opcode: Opcode, prefix: Vec<u8>, payload: Vec<u8>) -> Self {
        Self {
            opcode,
            encoded_prefix: Some(prefix),
            value: payload,
            retry_policy: generated_retry_policy(opcode, false),
        }
    }

    /// Creates a generic envelope from an adapter-owned prefix and body.
    ///
    /// A compatibility adapter calls this only after validating and encoding
    /// its own semantic fields. Other adapters can use the same boundary
    /// without adding fields or branches to this request type.
    pub(crate) fn new_wire(
        opcode: Opcode,
        prefix: Vec<u8>,
        payload: Vec<u8>,
        retry_policy: RequestRetryPolicy,
    ) -> Self {
        Self {
            opcode,
            encoded_prefix: Some(prefix),
            value: payload,
            retry_policy,
        }
    }

    pub(crate) fn into_parts(mut self) -> Result<RequestParts> {
        // Every constructor validates its generated framing before returning.
        // Avoid replaying the ordered-field decoder at the transport boundary.
        let prefix = if let Some(prefix) = self.encoded_prefix.take() {
            prefix
        } else {
            let mut prefix = vec![self.opcode as u8];
            match operation_wire_spec(self.opcode).generic_request_framing() {
                Some(crate::contract::OperationRequestFraming::Empty) => {}
                Some(crate::contract::OperationRequestFraming::Opaque) => {
                    append_varuint(&mut prefix, self.value.len() as u64);
                }
                Some(crate::contract::OperationRequestFraming::OrderedFields) => {
                    if operation_wire_spec(self.opcode).request.frame
                        == crate::contract::OperationFramePolicy::LengthDelimited
                    {
                        append_varuint(&mut prefix, self.value.len() as u64);
                    }
                }
                None => {
                    return Err(ProtocolError::InvalidFieldSequence(
                        "operation request framing is not generic",
                    ));
                }
            }
            prefix
        };
        validate_value_length(self.value.len())?;
        Ok(RequestParts {
            prefix,
            payload: self.value,
        })
    }

    fn validate(&self) -> Result<()> {
        validate_value_length(self.value.len())?;
        match operation_wire_spec(self.opcode).generic_request_framing() {
            Some(crate::contract::OperationRequestFraming::Empty) => {
                if !self.value.is_empty() {
                    return Err(invalid_shape(self.opcode, 0, "0"));
                }
            }
            Some(crate::contract::OperationRequestFraming::Opaque) => {
                generic::validate_request_body(self.opcode, &self.value)?;
            }
            Some(crate::contract::OperationRequestFraming::OrderedFields) => {
                generic::validate_request_body(self.opcode, &self.value)?;
            }
            None => {
                return Err(ProtocolError::InvalidFieldSequence(
                    "operation request framing is not generic",
                ));
            }
        }
        Ok(())
    }
}

fn append_varuint(output: &mut Vec<u8>, value: u64) {
    let (encoded, length) = encode_varuint(value);
    output.extend_from_slice(&encoded[..length]);
}

fn validate_value_length(value_len: usize) -> Result<()> {
    if value_len > MAX_VALUE_BYTES {
        return Err(ProtocolError::ValueTooLarge {
            size: value_len,
            maximum: MAX_VALUE_BYTES,
        });
    }
    Ok(())
}

fn invalid_shape(
    opcode: Opcode,
    expected_item_id: usize,
    expected_value: &'static str,
) -> ProtocolError {
    ProtocolError::InvalidRequestShape {
        opcode,
        expected_item_id,
        expected_value,
    }
}
