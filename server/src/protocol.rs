//! Server-owned semantic request codec and cache policy types.

// Keep the public server facade on the small wire primitive surface.  The
// generated operation/client projections are consumed by their adapters, not
// imported wholesale into request construction.
pub use crate::contract::{WireRequestLayout, WireRequestStep, wire_request_layout};
use openkache_protocol::{
    ITEM_ID_BYTES, MAX_VALUE_BYTES, MAX_VARUINT_BYTES, NAMESPACE_ID_BYTES, REQUEST_FIXED_BYTES,
    OwnedRange, RequestFrameHeader,
};
pub use openkache_protocol::{ItemId, Opcode, Response, Status};

type WireResult<T> = openkache_protocol::Result<T>;

#[path = "protocol_compat_v1.rs"]
mod compat_v1;
#[path = "protocol_generic.rs"]
mod generic;
#[path = "protocol_policy.rs"]
mod policy;
pub use policy::{
    EvictionDefault, EvictionMode, ExpirationDefault, ExpirationMode, NamespaceDescriptor,
    NamespacePolicy, OverridePolicy, SetCondition, SetOptions,
};

/// Returns the namespace-name limit owned by the protocol-v1 compatibility
/// adapter. Generic operation infrastructure does not consume this value.
pub(crate) const fn compatibility_namespace_name_max_bytes() -> usize {
    compat_v1::namespace_name_max_bytes()
}

/// Returns the complete request-frame admission ceiling for the composed
/// server. Generic layouts contribute the normal bound; compatibility
/// adapters may contribute a larger historical prefix without making that
/// prefix part of the generic operation contract.
pub(crate) const fn max_request_frame_bytes() -> usize {
    let generic = REQUEST_FIXED_BYTES
        .saturating_add(MAX_VARUINT_BYTES)
        .saturating_add(crate::contract::MAX_GENERIC_REQUEST_PAYLOAD_BYTES);
    let exact = crate::contract::MAX_REQUEST_WIRE_FRAME_BYTES;
    let compatibility = openkache_protocol::compat_v1::MAX_COMPATIBILITY_REQUEST_FRAME_BYTES;
    let generic_or_exact = if generic > exact { generic } else { exact };
    if generic_or_exact > compatibility {
        generic_or_exact
    } else {
        compatibility
    }
}

/// A validated variable-length request header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    opcode: Opcode,
    encoded_len: usize,
    value_len: usize,
    compatibility: Option<CompatibilityHeaderMetadata>,
}

/// Metadata decoded only by a compatibility adapter.
///
/// Generic framing never constructs or inspects this representation. Keeping
/// it behind the request header prevents compatibility namespace/item vocabulary from
/// leaking into the generic parser while preserving the public v1 header
/// accessors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompatibilityHeaderMetadata {
    namespace_id: Option<u64>,
    item_id_count: usize,
    has_ttl: bool,
}

/// Supplies operation-neutral request framing at the composition boundary.
///
/// Transport code receives only byte-consumption metadata. It never selects a
/// semantic adapter or learns whether an opcode has a historical public
/// convenience projection.
pub(crate) trait FrameLayoutProvider: Send + Sync {
    fn layout_for(&self, opcode: Opcode) -> WireResult<WireRequestLayout>;
}

/// Generated provider used by the public server protocol facade.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GeneratedFrameLayoutProvider;

impl FrameLayoutProvider for GeneratedFrameLayoutProvider {
    fn layout_for(&self, opcode: Opcode) -> WireResult<WireRequestLayout> {
        Ok(wire_request_layout(opcode))
    }
}

const DEFAULT_FRAME_LAYOUT_PROVIDER: GeneratedFrameLayoutProvider = GeneratedFrameLayoutProvider;

impl RequestHeader {
    pub(super) const fn generic(
        opcode: Opcode,
        encoded_len: usize,
        value_len: usize,
    ) -> Self {
        Self {
            opcode,
            encoded_len,
            value_len,
            compatibility: None,
        }
    }

    pub(super) const fn compatibility(
        opcode: Opcode,
        encoded_len: usize,
        value_len: usize,
        namespace_id: Option<u64>,
        item_id_count: usize,
        has_ttl: bool,
    ) -> Self {
        Self {
            opcode,
            encoded_len,
            value_len,
            compatibility: Some(CompatibilityHeaderMetadata {
                namespace_id,
                item_id_count,
                has_ttl,
            }),
        }
    }

    /// Returns the decoded operation.
    pub const fn opcode(self) -> Opcode {
        self.opcode
    }

    /// Returns the number of encoded bytes before a SET value.
    pub const fn encoded_len(self) -> usize {
        self.encoded_len
    }

    /// Returns the fixed item ID length for operations carrying an item ID.
    pub const fn item_id_len(self) -> usize {
        ITEM_ID_BYTES * self.item_id_count()
    }

    /// Returns the number of item IDs carried by this request.
    pub const fn item_id_count(self) -> usize {
        match self.compatibility {
            Some(metadata) => metadata.item_id_count,
            None => 0,
        }
    }

    /// Returns the opaque SET or application-value length, or zero for other operations.
    pub const fn value_len(self) -> usize {
        self.value_len
    }

    /// Returns the namespace ID carried by this request, when applicable.
    pub const fn namespace_id(self) -> Option<u64> {
        match self.compatibility {
            Some(metadata) => metadata.namespace_id,
            None => None,
        }
    }

    /// Returns whether a TTL varuint follows the SET item ID.
    pub const fn has_ttl(self) -> bool {
        match self.compatibility {
            Some(metadata) => metadata.has_ttl,
            None => false,
        }
    }

    /// Reports the complete frame length once all metadata is available.
    pub fn frame_len(self, _prefix: &[u8]) -> Result<Option<usize>> {
        self.encoded_len
            .checked_add(self.value_len)
            .map(Some)
            .ok_or(ProtocolError::FrameLengthOverflow)
    }
}

/// A complete v1 request viewed as an opaque operation call.
///
/// The server adapter supplies generated byte-consumption metadata to the
/// operation-neutral parser. It deliberately does not decode namespace IDs,
/// item IDs, values, or operation semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestFrame<'a> {
    inner: openkache_protocol::OpaqueRequestFrame<'a>,
}

impl<'a> RequestFrame<'a> {
    fn layout_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> WireResult<Option<WireRequestLayout>> {
        let Some(&opcode_byte) = prefix.first() else {
            return Ok(None);
        };
        let opcode = Opcode::try_from(opcode_byte)?;
        let layout = provider.layout_for(opcode)?;
        Ok(Some(layout))
    }

    /// Decodes only the frame metadata needed to delimit one request.
    pub fn decode_header(prefix: &[u8]) -> WireResult<Option<RequestFrameHeader>> {
        Self::decode_header_with(prefix, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes frame metadata with an explicitly selected layout provider.
    pub(crate) fn decode_header_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> WireResult<Option<RequestFrameHeader>> {
        let Some(layout) = Self::layout_with(prefix, provider)? else {
            return Ok(None);
        };
        openkache_protocol::OpaqueRequestFrame::decode_header(prefix, layout)
    }

    /// Returns the exact additional metadata bound for an explicitly selected
    /// layout provider.
    pub(crate) fn header_bytes_needed_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> WireResult<usize> {
        let Some(layout) = Self::layout_with(prefix, provider)? else {
            return Ok(1);
        };
        openkache_protocol::OpaqueRequestFrame::header_bytes_needed(prefix, layout)
    }

    /// Reports the complete frame length once enough metadata is available.
    pub fn frame_len(prefix: &[u8]) -> WireResult<Option<usize>> {
        Self::frame_len_with(prefix, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Reports frame length with an explicitly selected layout provider.
    pub(crate) fn frame_len_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> WireResult<Option<usize>> {
        Self::decode_header_with(prefix, provider)?
            .map(RequestFrameHeader::frame_len)
            .transpose()
    }

    /// Decodes one complete request without interpreting its operation body.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the opcode, generated wire layout, or
    /// complete frame length is invalid.
    pub fn decode(frame: &'a [u8]) -> WireResult<Self> {
        Self::decode_with(frame, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes one complete request with an explicitly selected layout
    /// provider.
    pub(crate) fn decode_with<P: FrameLayoutProvider + ?Sized>(
        frame: &'a [u8],
        provider: &P,
    ) -> WireResult<Self> {
        let layout = Self::layout_with(frame, provider)?.ok_or(
            openkache_protocol::ProtocolError::FrameTooShort {
                expected: REQUEST_FIXED_BYTES,
                actual: frame.len(),
            },
        )?;
        Ok(Self {
            inner: openkache_protocol::OpaqueRequestFrame::decode(frame, layout)?,
        })
    }

    /// Returns the operation discriminator.
    pub const fn opcode(self) -> Opcode {
        self.inner.opcode()
    }

    /// Returns the opaque operation body after the opcode.
    pub fn body(self) -> &'a [u8] {
        self.inner.body()
    }

    /// Returns the original complete encoded frame.
    pub const fn encoded(self) -> &'a [u8] {
        self.inner.encoded()
    }
}

/// A decoded OpenKache request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request {
    pub opcode: Opcode,
    pub namespace_id: Option<u64>,
    pub item_ids: Vec<ItemId>,
    pub set_options: SetOptions,
    pub value: Vec<u8>,
    pub namespace_name: Option<Vec<u8>>,
    pub namespace_policy: Option<NamespacePolicy>,
    pub expected_revision: Option<u64>,
    pub create_if_missing: bool,
}

/// One transport-owned request passed to every API decoder.
///
/// Prefix and payload ownership are retained independently. The envelope does
/// not classify the operation by wire family or materialize a semantic
/// request; generated or API-owned field projection happens only after
/// registration selects the decoder.
pub(crate) struct ServerRequest {
    opcode: Opcode,
    prefix: Vec<u8>,
    payload: OwnedRange,
}

impl ServerRequest {
    fn new(opcode: Opcode, prefix: Vec<u8>, payload: OwnedRange) -> Self {
        Self {
            opcode,
            prefix,
            payload,
        }
    }

    pub(crate) const fn opcode(&self) -> Opcode {
        self.opcode
    }

    /// Moves the retained payload into a generated field view.
    pub(crate) fn into_payload(self) -> (Opcode, OwnedRange) {
        (self.opcode, self.payload)
    }

    /// Moves the complete retained wire parts into the generated request-plan
    /// decoder.
    pub(crate) fn into_wire_parts(self) -> (Opcode, Vec<u8>, OwnedRange) {
        (self.opcode, self.prefix, self.payload)
    }

}

impl Request {
    /// Creates a request for a route-less operation using its already encoded
    /// generic body.
    ///
    /// Generic callers do not need to construct the historical namespace/item
    /// facade fields. The operation contract still validates the framing and
    /// generated field shape before the request is returned.
    pub fn new_generic(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        if crate::contract::request_wire_plan(opcode).is_some() {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "generic operation",
            });
        }
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a route-less request from generated ordered field values.
    ///
    /// Field presence, codecs, and dense/sequence layout are selected from
    /// the operation contract. API callers provide only the modeled field
    /// bytes in plan order; compatibility operations intentionally reject this
    /// constructor and use their typed adapter instead.
    pub fn new_generic_fields(opcode: Opcode, fields: Vec<Option<Vec<u8>>>) -> Result<Self> {
        if crate::contract::request_wire_plan(opcode).is_some() {
            return Err(ProtocolError::InvalidRequestShape {
                opcode,
                expected_item_id: 0,
                expected_value: "generic operation",
            });
        }
        let value = generic::encode_fields(opcode, fields)?;
        Self::new_generic(opcode, value)
    }

    /// Creates a request for an operation that has no namespace or item fields.
    pub fn new(opcode: Opcode, item_id: Option<ItemId>, value: Vec<u8>) -> Result<Self> {
        if item_id.is_none() {
            return Self::new_generic(opcode, value);
        }
        let request = Self {
            opcode,
            namespace_id: None,
            item_ids: item_id.into_iter().collect(),
            set_options: SetOptions::NONE,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request with a namespace ID.
    pub fn new_scoped(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(opcode, namespace_id, item_id, SetOptions::NONE, value)
    }

    /// Creates a data-plane request with explicit SET options.
    pub fn new_scoped_with_options(
        opcode: Opcode,
        namespace_id: u64,
        item_id: Option<ItemId>,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids: item_id.into_iter().collect(),
            set_options,
            value,
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a data-plane request carrying one or more exact item IDs.
    pub fn new_scoped_items(
        opcode: Opcode,
        namespace_id: u64,
        item_ids: Vec<ItemId>,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: Some(namespace_id),
            item_ids,
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: None,
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a SET request with explicit options.
    pub fn new_set(
        namespace_id: u64,
        item_id: ItemId,
        set_options: SetOptions,
        value: Vec<u8>,
    ) -> Result<Self> {
        Self::new_scoped_with_options(Opcode::Set, namespace_id, Some(item_id), set_options, value)
    }

    /// Creates a namespace-open request. An empty name is a valid name.
    pub fn namespace_open(
        name: impl AsRef<[u8]>,
        create_if_missing: bool,
        policy: Option<NamespacePolicy>,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceOpen,
            namespace_id: None,
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: Some(name.as_ref().to_vec()),
            namespace_policy: policy,
            expected_revision: None,
            create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates a namespace-policy update request.
    pub fn namespace_update_policy(
        namespace_id: u64,
        expected_revision: u64,
        policy: NamespacePolicy,
    ) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceUpdatePolicy,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: Some(policy),
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Creates an empty-only namespace-delete request.
    pub fn namespace_delete(namespace_id: u64, expected_revision: u64) -> Result<Self> {
        let request = Self {
            opcode: Opcode::NamespaceDelete,
            namespace_id: Some(namespace_id),
            item_ids: Vec::new(),
            set_options: SetOptions::NONE,
            value: Vec::new(),
            namespace_name: None,
            namespace_policy: None,
            expected_revision: Some(expected_revision),
            create_if_missing: false,
        };
        request.validate()?;
        Ok(request)
    }

    /// Encodes this request into one complete stream frame.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if openkache_protocol::operation::request_wire_plan(self.opcode).is_some() {
            self.validate()?;
            return compat_v1::encode_request(self);
        }
        let mut frame = self.encode_prefix()?;
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
        if openkache_protocol::operation::request_wire_plan(self.opcode).is_some() {
            return self.encode();
        }
        let prefix = self.encode_prefix()?;
        let value_len = self.value.len();
        self.value.reserve(prefix.len());
        self.value.resize(prefix.len() + value_len, 0);
        self.value.copy_within(0..value_len, prefix.len());
        self.value[..prefix.len()].copy_from_slice(&prefix);
        Ok(self.value)
    }

    fn encode_prefix(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::new();
        output.push(self.opcode as u8);
        let encoded = if crate::contract::request_wire_plan(self.opcode).is_some() {
            compat_v1::encode_request_prefix(self, &mut output)?
        } else {
            generic::encode_request_prefix(self, &mut output)?
        };
        if !encoded {
            return Err(ProtocolError::InvalidFieldSequence(
                "generated request plan did not encode the modeled operation",
            ));
        }
        Ok(output)
    }

    /// Decodes and validates one complete request frame.
    pub fn decode(frame: &[u8]) -> Result<Self> {
        Self::decode_with(frame, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes and validates one complete request frame with an explicitly
    /// selected layout provider.
    pub(crate) fn decode_with<P: FrameLayoutProvider + ?Sized>(
        frame: &[u8],
        provider: &P,
    ) -> Result<Self> {
        let header = Self::validated_header_with(frame, provider)?;
        if crate::contract::request_wire_plan(header.opcode).is_some() {
            compat_v1::decode_request(frame, header)
        } else {
            generic::decode_request(frame, header)
        }
    }

    /// Decodes a request while reusing the frame allocation for its value.
    pub fn decode_owned(frame: Vec<u8>) -> Result<Self> {
        Self::decode_owned_impl(frame, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes independently owned request-prefix and payload buffers.
    ///
    /// Network backends use this entry point so a contiguous receive chunk can
    /// remain reference-counted through generic execution. Compatibility
    /// adapters may materialize the payload only at their own semantic
    /// boundary.
    pub(crate) fn decode_received_for_server_with<P: FrameLayoutProvider + ?Sized>(
        prefix: Vec<u8>,
        payload: OwnedRange,
        provider: &P,
    ) -> Result<ServerRequest> {
        let header = RequestFrame::decode_header_with(&prefix, provider)?.ok_or(
            ProtocolError::FrameTooShort {
                expected: REQUEST_FIXED_BYTES,
                actual: prefix.len(),
            },
        )?;
        let actual = prefix
            .len()
            .checked_add(payload.len())
            .ok_or(ProtocolError::FrameLengthOverflow)?;
        let expected = header.frame_len()?;
        if prefix.len() != header.encoded_len() || payload.len() != header.value_len() {
            return Err(ProtocolError::FrameLength { expected, actual });
        }
        Ok(ServerRequest::new(header.opcode(), prefix, payload))
    }

    fn decode_owned_impl<P: FrameLayoutProvider + ?Sized>(
        frame: Vec<u8>,
        provider: &P,
    ) -> Result<Self> {
        let header = Self::validated_header_with(&frame, provider)?;
        if crate::contract::request_wire_plan(header.opcode).is_some() {
            let decode_owned_request: fn(Vec<u8>, RequestHeader) -> Result<Self> =
                compat_v1::decode_owned_request;
            decode_owned_request(frame, header)
        } else {
            generic::decode_owned_request(frame, header)
        }
    }

    fn validated_header_with<P: FrameLayoutProvider + ?Sized>(
        frame: &[u8],
        provider: &P,
    ) -> Result<RequestHeader> {
        let header =
            Self::decode_header_with(frame, provider)?.ok_or(ProtocolError::FrameTooShort {
                expected: REQUEST_FIXED_BYTES,
                actual: frame.len(),
            })?;
        let expected = header
            .frame_len(&frame)?
            .ok_or(ProtocolError::FrameTooShort {
                expected: header.encoded_len,
                actual: frame.len(),
            })?;
        if frame.len() != expected {
            return Err(ProtocolError::FrameLength {
                expected,
                actual: frame.len(),
            });
        }
        Ok(header)
    }

    /// Decodes a request header when enough metadata bytes are available.
    pub fn decode_header(prefix: &[u8]) -> Result<Option<RequestHeader>> {
        Self::decode_header_with(prefix, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes a request header with an explicitly selected layout provider.
    pub(crate) fn decode_header_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> Result<Option<RequestHeader>> {
        let Some(&opcode_byte) = prefix.first() else {
            return Ok(None);
        };
        let opcode = Opcode::try_from(opcode_byte)?;
        if crate::contract::request_wire_plan(opcode).is_some() {
            // Compact requests have a semantic adapter-owned header.  Let it
            // validate packed policy bits and project compatibility errors
            // before the operation-neutral parser is asked only to delimit a
            // generic frame.
            return compat_v1::decode_header(prefix, opcode);
        }
        let layout = provider.layout_for(opcode)?;
        let Some(frame) = openkache_protocol::OpaqueRequestFrame::decode_header(prefix, layout)?
        else {
            return Ok(None);
        };
        Ok(Some(RequestHeader::generic(
            opcode,
            frame.encoded_len(),
            frame.value_len(),
        )))
    }

    /// Reports the complete request frame length once metadata is available.
    pub fn frame_len(prefix: &[u8]) -> Result<Option<usize>> {
        Self::frame_len_with(prefix, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Reports frame length with an explicitly selected layout provider.
    pub(crate) fn frame_len_with<P: FrameLayoutProvider + ?Sized>(
        prefix: &[u8],
        provider: &P,
    ) -> Result<Option<usize>> {
        Self::decode_header_with(prefix, provider)?
            .map(|header| header.frame_len(prefix))
            .transpose()
            .map(|value| value.flatten())
    }

    fn validate(&self) -> Result<()> {
        if crate::contract::request_wire_plan(self.opcode).is_some() {
            compat_v1::validate_request(self)
        } else {
            generic::validate_request(self)
        }
    }
}

impl Request {
    /// Returns the payload view accepted by a generic framing adapter.
    ///
    /// Compatibility-only semantic metadata stays in the v1 projection. The
    /// generic adapter receives this narrow view instead of inspecting the
    /// compatibility facade fields itself.
    pub(super) fn generic_payload(&self) -> Result<&[u8]> {
        if self.namespace_id.is_some()
            || !self.item_ids.is_empty()
            || self.set_options != SetOptions::NONE
            || self.namespace_name.is_some()
            || self.namespace_policy.is_some()
            || self.expected_revision.is_some()
            || self.create_if_missing
        {
            return Err(ProtocolError::InvalidRequestShape {
                opcode: self.opcode,
                expected_item_id: 0,
                expected_value: "generic fields",
            });
        }
        Ok(&self.value)
    }

    fn from_generic_parts(opcode: Opcode, value: Vec<u8>) -> Result<Self> {
        Self::new_generic(opcode, value)
    }

    fn from_decoded_parts(
        metadata: compat_v1::DecodedRequestMetadata,
        value: Vec<u8>,
        opcode: Opcode,
    ) -> Result<Self> {
        let request = Self {
            opcode,
            namespace_id: metadata.namespace_id,
            item_ids: metadata.item_ids,
            set_options: metadata.set_options,
            value,
            namespace_name: metadata.namespace_name,
            namespace_policy: metadata.namespace_policy,
            expected_revision: metadata.expected_revision,
            create_if_missing: metadata.create_if_missing,
        };
        request.validate()?;
        Ok(request)
    }
}

fn read_u64_be(input: &[u8]) -> Result<u64> {
    let bytes: [u8; NAMESPACE_ID_BYTES] = input
        .get(..NAMESPACE_ID_BYTES)
        .ok_or(ProtocolError::FrameTooShort {
            expected: NAMESPACE_ID_BYTES,
            actual: input.len(),
        })?
        .try_into()
        .expect("slice length checked");
    Ok(u64::from_be_bytes(bytes))
}

fn encode_varuint(value: u64) -> ([u8; MAX_VARUINT_BYTES], usize) {
    openkache_protocol::encode_varuint(value)
}

fn decode_varuint(input: &[u8], context: &'static str) -> Result<Option<(u64, usize)>> {
    openkache_protocol::decode_varuint(input, context).map_err(Into::into)
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

/// Protocol framing and validation errors.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("unknown opcode 0x{0:02x}")]
    UnknownOpcode(u8),
    #[error("unknown status 0x{0:02x}")]
    UnknownStatus(u8),
    #[error("request flags contain unknown bits 0x{0:02x}")]
    UnknownRequestFlags(u8),
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
    #[error("{opcode:?} requires a {expected}-byte item ID, received {actual} item ID bytes")]
    InvalidItemIdLength {
        opcode: Opcode,
        expected: usize,
        actual: usize,
    },
    #[error("value is too large: {size} bytes exceeds {maximum}")]
    ValueTooLarge { size: usize, maximum: usize },
    #[error("optional-value payload is invalid: {0}")]
    InvalidOptionalValues(&'static str),
    #[error("operation field sequence is invalid: {0}")]
    InvalidFieldSequence(&'static str),
    #[error("{opcode:?} requires a fixed item/value shape ({expected_item_id}, {expected_value})")]
    InvalidRequestShape {
        opcode: Opcode,
        expected_item_id: usize,
        expected_value: &'static str,
    },
    #[error("unsupported compact protocol-v1 request route {0}")]
    UnsupportedCompactV1Route(&'static str),
    #[error("if-absent and if-present conditions cannot be combined")]
    ConflictingSetConditions,
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
            openkache_protocol::ProtocolError::InvalidOptionalValues(message) => {
                Self::InvalidOptionalValues(message)
            }
            openkache_protocol::ProtocolError::InvalidFieldSequence(message) => {
                Self::InvalidFieldSequence(message)
            }
        }
    }
}

/// Convenience result type for protocol operations.
pub type Result<T> = std::result::Result<T, ProtocolError>;
