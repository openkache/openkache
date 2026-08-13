//! Server-owned semantic request codec and cache policy types.

// Keep the public server facade on the small wire primitive surface.  The
// generated operation/client projections are consumed by their adapters, not
// imported wholesale into request construction.
pub use crate::contract::{WireRequestLayout, WireRequestStep, wire_request_layout};
use openkache_protocol::{
    MAX_ITEM_ID_BYTES, MAX_VALUE_BYTES, MAX_VARUINT_BYTES, NAMESPACE_ID_BYTES,
    REQUEST_FIXED_BYTES, RequestFrameHeader,
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
    let compatibility = openkache_protocol::compat_v1::MAX_COMPATIBILITY_REQUEST_FRAME_BYTES;
    if generic > compatibility {
        generic
    } else {
        compatibility
    }
}

/// A validated variable-length request header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestHeader {
    adapter: RequestAdapter,
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
    item_id_start: Option<usize>,
    item_id_count: usize,
    item_id_lengths: [u8; 2],
    set_options: SetOptions,
    has_ttl: bool,
}

/// The selected parser boundary for a request frame.
///
/// The header carries this decision so the server hot path does not classify
/// the opcode a second time after frame admission.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestAdapter {
    name: &'static str,
    request_frame_layout: fn(Opcode) -> WireResult<WireRequestLayout>,
    decode_header: fn(&[u8], Opcode, RequestAdapter) -> Result<Option<RequestHeader>>,
    encode_request_prefix: fn(&Request, &mut Vec<u8>) -> Result<bool>,
    validate_request: fn(&Request) -> Result<()>,
    decode_request: fn(&[u8], RequestHeader) -> Result<Request>,
    decode_owned_request: fn(Vec<u8>, RequestHeader) -> Result<Request>,
    decode_server_request: fn(Vec<u8>, RequestHeader) -> Result<ServerRequest>,
}

impl RequestAdapter {
    /// Generic generated framing. The adapter owns no domain route metadata.
    #[allow(non_upper_case_globals)]
    pub(crate) const Generic: Self = Self {
        name: "generic",
        request_frame_layout: generic::request_frame_layout,
        decode_header: generic::decode_header,
        encode_request_prefix: generic::encode_request_prefix,
        validate_request: generic::validate_request,
        decode_request: generic::decode_request,
        decode_owned_request: generic::decode_owned_request,
        decode_server_request: generic::decode_server_request,
    };

    /// Historical protocol-v1 compact framing.
    ///
    /// This remains a compatibility adapter rather than a generic operation
    /// variant. A future wire family can add another descriptor without
    /// changing the request parser's control flow.
    #[allow(non_upper_case_globals)]
    pub(crate) const Compatibility: Self = Self {
        name: "compatibility-v1",
        request_frame_layout: compat_v1::request_frame_layout,
        decode_header: compat_v1::decode_header,
        encode_request_prefix: compat_v1::encode_request_prefix,
        validate_request: compat_v1::validate_request,
        decode_request: compat_v1::decode_request,
        decode_owned_request: compat_v1::decode_owned_request,
        decode_server_request: compat_v1::decode_server_request,
    };

    fn request_frame_layout(self, opcode: Opcode) -> WireResult<WireRequestLayout> {
        (self.request_frame_layout)(opcode)
    }

    fn decode_header(self, prefix: &[u8], opcode: Opcode) -> Result<Option<RequestHeader>> {
        (self.decode_header)(prefix, opcode, self)
    }

    fn encode_request_prefix(self, request: &Request, output: &mut Vec<u8>) -> Result<bool> {
        (self.encode_request_prefix)(request, output)
    }

    fn validate_request(self, request: &Request) -> Result<()> {
        (self.validate_request)(request)
    }

    fn decode_request(self, frame: &[u8], header: RequestHeader) -> Result<Request> {
        (self.decode_request)(frame, header)
    }

    fn decode_owned_request(self, frame: Vec<u8>, header: RequestHeader) -> Result<Request> {
        (self.decode_owned_request)(frame, header)
    }

    fn decode_server_request(self, frame: Vec<u8>, header: RequestHeader) -> Result<ServerRequest> {
        (self.decode_server_request)(frame, header)
    }
}

impl PartialEq for RequestAdapter {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for RequestAdapter {}

/// Supplies the request adapter selected at the composition boundary.
///
/// The parser consumes this provider result and never needs to know how a
/// route was classified. The default implementation is generated from the
/// Smithy compatibility projection; tests and future protocol versions can
/// supply another provider without changing generic framing code.
pub(crate) trait FrameLayoutProvider: Send + Sync {
    fn adapter_for(&self, opcode: Opcode) -> RequestAdapter;
}

#[derive(Clone, Copy)]
struct FrameAdapterRegistration {
    adapter: RequestAdapter,
    accepts: fn(Opcode) -> bool,
}

impl FrameAdapterRegistration {
    const fn new(adapter: RequestAdapter, accepts: fn(Opcode) -> bool) -> Self {
        Self { adapter, accepts }
    }
}

fn accepts_any_opcode(_opcode: Opcode) -> bool {
    true
}

/// Generated adapter registry ordered from the most specific projection to
/// the generic fallback. The parser consumes this table through the provider;
/// adding another wire family adds one registration and leaves framing logic
/// unchanged.
const FRAME_ADAPTER_REGISTRY: &[FrameAdapterRegistration] = &[
    FrameAdapterRegistration::new(
        RequestAdapter::Compatibility,
        compat_v1::compatibility_route_for_opcode,
    ),
    FrameAdapterRegistration::new(RequestAdapter::Generic, accepts_any_opcode),
];

/// Generated provider used by the public server protocol facade.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GeneratedFrameLayoutProvider;

impl FrameLayoutProvider for GeneratedFrameLayoutProvider {
    fn adapter_for(&self, opcode: Opcode) -> RequestAdapter {
        FRAME_ADAPTER_REGISTRY
            .iter()
            .find(|registration| (registration.accepts)(opcode))
            .map(|registration| registration.adapter)
            .expect("generated frame adapter registry must have a fallback")
    }
}

const DEFAULT_FRAME_LAYOUT_PROVIDER: GeneratedFrameLayoutProvider = GeneratedFrameLayoutProvider;

impl RequestHeader {
    pub(super) const fn generic(
        adapter: RequestAdapter,
        opcode: Opcode,
        encoded_len: usize,
        value_len: usize,
    ) -> Self {
        Self {
            adapter,
            opcode,
            encoded_len,
            value_len,
            compatibility: None,
        }
    }

    pub(super) const fn compatibility(
        adapter: RequestAdapter,
        opcode: Opcode,
        encoded_len: usize,
        value_len: usize,
        namespace_id: Option<u64>,
        item_id_start: Option<usize>,
        item_id_count: usize,
        item_id_lengths: [u8; 2],
        set_options: SetOptions,
        has_ttl: bool,
    ) -> Self {
        Self {
            adapter,
            opcode,
            encoded_len,
            value_len,
            compatibility: Some(CompatibilityHeaderMetadata {
                namespace_id,
                item_id_start,
                item_id_count,
                item_id_lengths,
                set_options,
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

    /// Returns the total item ID bytes carried by this request.
    pub const fn item_id_len(self) -> usize {
        let mut length = 0;
        let mut index = 0;
        while index < self.item_id_count() && index < self.item_id_lengths().len() {
            length += self.item_id_lengths()[index] as usize;
            index += 1;
        }
        length
    }

    /// Returns the number of item IDs carried by this request.
    pub const fn item_id_count(self) -> usize {
        match self.compatibility {
            Some(metadata) => metadata.item_id_count,
            None => 0,
        }
    }

    /// Returns the individual item ID lengths in wire order.
    pub const fn item_id_lengths(self) -> [u8; 2] {
        match self.compatibility {
            Some(metadata) => metadata.item_id_lengths,
            None => [0; 2],
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

    pub(super) const fn item_id_start(self) -> Option<usize> {
        match self.compatibility {
            Some(metadata) => metadata.item_id_start,
            None => None,
        }
    }

    pub(super) const fn set_options(self) -> SetOptions {
        match self.compatibility {
            Some(metadata) => metadata.set_options,
            None => SetOptions::NONE,
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
        let layout = provider.adapter_for(opcode).request_frame_layout(opcode)?;
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

/// Server-only request envelope.
///
/// Generic opaque/ordered requests retain their original frame allocation and
/// expose the payload range to the generated operation view. Compact v1
/// requests are materialized into [`Request`] by the compatibility adapter.
/// Keeping this distinction here avoids forcing the public semantic request
/// type to own a second payload copy on the generic hot path.
pub(crate) enum ServerRequest {
    Generic {
        opcode: Opcode,
        frame: Vec<u8>,
        payload_range: (usize, usize),
    },
    Compatibility(Request),
}

impl ServerRequest {
    pub(crate) fn from_request(request: Request) -> Self {
        Self::Compatibility(request)
    }

    pub(crate) fn opcode(&self) -> Opcode {
        match self {
            Self::Generic { opcode, .. } => *opcode,
            Self::Compatibility(request) => request.opcode,
        }
    }

    pub(crate) fn into_request(self) -> Request {
        match self {
            Self::Compatibility(request) => request,
            Self::Generic { .. } => {
                unreachable!("generic requests never enter the compatibility adapter")
            }
        }
    }

    /// Returns the operation discriminator and an owned body.
    ///
    /// The normal generic hot path consumes [`Self::into_generic_frame`]
    /// without copying. This fallback is retained for small adapter/test
    /// callers that already own a semantic request.
    pub(crate) fn into_generic_parts(self) -> (Opcode, Vec<u8>) {
        match self {
            Self::Compatibility(request) => (request.opcode, request.value),
            Self::Generic {
                opcode,
                frame,
                payload_range: (start, end),
            } => (opcode, frame[start..end].to_vec()),
        }
    }

    pub(crate) fn has_generic_frame(&self) -> bool {
        matches!(self, Self::Generic { .. })
    }

    pub(crate) fn into_generic_frame(self) -> Option<(Opcode, Vec<u8>, usize, usize)> {
        match self {
            Self::Generic {
                opcode,
                frame,
                payload_range: (start, end),
            } => Some((opcode, frame, start, end)),
            Self::Compatibility(_) => None,
        }
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
        if DEFAULT_FRAME_LAYOUT_PROVIDER.adapter_for(opcode) != RequestAdapter::Generic {
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
        if DEFAULT_FRAME_LAYOUT_PROVIDER.adapter_for(opcode) != RequestAdapter::Generic {
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
        let mut frame = self.encode_prefix()?;
        frame.extend_from_slice(&self.value);
        Ok(frame)
    }

    /// Encodes this request while reusing its value allocation when practical.
    pub fn into_encoded(mut self) -> Result<Vec<u8>> {
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
        let adapter = DEFAULT_FRAME_LAYOUT_PROVIDER.adapter_for(self.opcode);
        if !adapter.encode_request_prefix(self, &mut output)? {
            return Err(ProtocolError::InvalidFieldSequence(
                "request adapter did not encode the modeled operation",
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
        header.adapter.decode_request(frame, header)
    }

    /// Decodes a request while reusing the frame allocation for its value.
    pub fn decode_owned(frame: Vec<u8>) -> Result<Self> {
        Self::decode_owned_impl(frame, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes a server request after the frame boundary has been checked.
    ///
    /// The server's generated operation view performs the single generic
    /// ordered-field validation/decode pass. Keeping this internal entry point
    /// separate preserves the public `decode_owned` validation contract while
    /// avoiding a second scan on the request hot path.
    #[allow(dead_code)]
    pub(crate) fn decode_owned_for_server(frame: Vec<u8>) -> Result<ServerRequest> {
        Self::decode_owned_for_server_with(frame, &DEFAULT_FRAME_LAYOUT_PROVIDER)
    }

    /// Decodes a server request with an explicitly selected layout provider.
    pub(crate) fn decode_owned_for_server_with<P: FrameLayoutProvider + ?Sized>(
        frame: Vec<u8>,
        provider: &P,
    ) -> Result<ServerRequest> {
        let header = Self::validated_header_with(&frame, provider)?;
        header.adapter.decode_server_request(frame, header)
    }

    fn decode_owned_impl<P: FrameLayoutProvider + ?Sized>(
        frame: Vec<u8>,
        provider: &P,
    ) -> Result<Self> {
        let header = Self::validated_header_with(&frame, provider)?;
        header.adapter.decode_owned_request(frame, header)
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
        provider.adapter_for(opcode).decode_header(prefix, opcode)
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
        DEFAULT_FRAME_LAYOUT_PROVIDER
            .adapter_for(self.opcode)
            .validate_request(self)
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
