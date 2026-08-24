/// Maximum number of ordered request fields in any modeled operation.
///
/// Server operation views use this generated bound to reject unbounded shapes.
/// Runtime metadata uses bounded inline storage and spills only when a valid
/// model actually exceeds its inline capacity.
pub const MAX_OPERATION_REQUEST_FIELDS: usize = 8;

/// Maximum ordered fields in either request or response plans.
///
/// Client and server views use this generated bound for offset-storage
/// validation; ordinary plans keep their offsets inline and larger valid plans
/// spill through the same bounded representation.
pub const MAX_OPERATION_FIELDS: usize = 10;

use crate::{
    RequestFrameLayout as WireRequestLayout,
    RequestFramePackedField as WireRequestPackedField,
    RequestFramePackedValue as WireRequestPackedValue,
    RequestFrameStep as WireRequestStep,
};

/// Maximum complete generated request frame across all modeled operations.
pub const MAX_REQUEST_FRAME_BYTES: usize =
    67_108_934;

/// Returns the wire-level request layout for one assigned opcode.
const WIRE_REQUEST_LAYOUTS: [WireRequestLayout; Opcode::COUNT] = [
    WireRequestLayout {
            steps: &[],
            field_count: 0,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }, WireRequestStep::ByteLengthPrefix {
                slot: 0,
                field: 1,
            }, WireRequestStep::ByteLengthBodyField {
                slot: 0,
                field: 1,
            }],
            field_count: 2,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }, WireRequestStep::Packed {
                fields: &[WireRequestPackedField {
                    slot: 0,
                    field: 3,
                    mask: 0x03,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x61, 0x6e, 0x79],
                }, WireRequestPackedValue {
                    bits: 0x01,
                    bytes: &[0x69, 0x66, 0x5f, 0x61, 0x62, 0x73, 0x65, 0x6e, 0x74],
                }, WireRequestPackedValue {
                    bits: 0x02,
                    bytes: &[0x69, 0x66, 0x5f, 0x70, 0x72, 0x65, 0x73, 0x65, 0x6e, 0x74],
                }],
                }, WireRequestPackedField {
                    slot: 1,
                    field: 4,
                    mask: 0x0c,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x69, 0x6e, 0x68, 0x65, 0x72, 0x69, 0x74],
                }, WireRequestPackedValue {
                    bits: 0x04,
                    bytes: &[0x6e, 0x6f, 0x5f, 0x65, 0x78, 0x70, 0x69, 0x72, 0x79],
                }, WireRequestPackedValue {
                    bits: 0x08,
                    bytes: &[0x65, 0x78, 0x70, 0x6c, 0x69, 0x63, 0x69, 0x74, 0x5f, 0x74, 0x74, 0x6c],
                }],
                }, WireRequestPackedField {
                    slot: 2,
                    field: 5,
                    mask: 0x30,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x69, 0x6e, 0x68, 0x65, 0x72, 0x69, 0x74],
                }, WireRequestPackedValue {
                    bits: 0x10,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x61, 0x62, 0x6c, 0x65],
                }, WireRequestPackedValue {
                    bits: 0x20,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x72, 0x6f, 0x74, 0x65, 0x63, 0x74, 0x65, 0x64],
                }],
                }],
                reserved_mask: 0xc0,
                constant_bits: 0x00,
            }, WireRequestStep::ByteLengthPrefix {
                slot: 0,
                field: 1,
            }, WireRequestStep::ValueLengthPrefixField { field: 2 }, WireRequestStep::Conditional {
                selector: 1,
                expected: 0x08,
                steps: &[WireRequestStep::VarUIntField { field: 6 }],
            }, WireRequestStep::ByteLengthBodyField {
                slot: 0,
                field: 1,
            }],
            field_count: 7,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }, WireRequestStep::ByteLengthPrefix {
                slot: 0,
                field: 1,
            }, WireRequestStep::ByteLengthBodyField {
                slot: 0,
                field: 1,
            }],
            field_count: 2,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }],
            field_count: 1,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }],
            field_count: 1,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::Packed {
                fields: &[WireRequestPackedField {
                    slot: 0,
                    field: 1,
                    mask: 0x01,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x00],
                }, WireRequestPackedValue {
                    bits: 0x01,
                    bytes: &[0x01],
                }],
                }],
                reserved_mask: 0xfe,
                constant_bits: 0x00,
            }, WireRequestStep::ByteLengthField { field: 0 }, WireRequestStep::Conditional {
                selector: 0,
                expected: 0x01,
                steps: &[WireRequestStep::Packed {
                fields: &[WireRequestPackedField {
                    slot: 1,
                    field: 3,
                    mask: 0x03,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x6e, 0x6f, 0x5f, 0x65, 0x78, 0x70, 0x69, 0x72, 0x79],
                }, WireRequestPackedValue {
                    bits: 0x01,
                    bytes: &[0x66, 0x69, 0x78, 0x65, 0x64, 0x5f, 0x74, 0x74, 0x6c],
                }],
                }, WireRequestPackedField {
                    slot: 2,
                    field: 5,
                    mask: 0x04,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }, WireRequestPackedValue {
                    bits: 0x04,
                    bytes: &[0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }],
                }, WireRequestPackedField {
                    slot: 3,
                    field: 6,
                    mask: 0x08,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x61, 0x62, 0x6c, 0x65],
                }, WireRequestPackedValue {
                    bits: 0x08,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x72, 0x6f, 0x74, 0x65, 0x63, 0x74, 0x65, 0x64],
                }],
                }, WireRequestPackedField {
                    slot: 4,
                    field: 7,
                    mask: 0x10,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }, WireRequestPackedValue {
                    bits: 0x10,
                    bytes: &[0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }],
                }],
                reserved_mask: 0xe0,
                constant_bits: 0x00,
            }, WireRequestStep::Conditional {
                selector: 1,
                expected: 0x01,
                steps: &[WireRequestStep::VarUIntField { field: 4 }],
            }],
            }],
            field_count: 8,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }, WireRequestStep::FixedField {
                field: 1,
                bytes: 8,
            }, WireRequestStep::Packed {
                fields: &[WireRequestPackedField {
                    slot: 0,
                    field: 3,
                    mask: 0x03,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x6e, 0x6f, 0x5f, 0x65, 0x78, 0x70, 0x69, 0x72, 0x79],
                }, WireRequestPackedValue {
                    bits: 0x01,
                    bytes: &[0x66, 0x69, 0x78, 0x65, 0x64, 0x5f, 0x74, 0x74, 0x6c],
                }],
                }, WireRequestPackedField {
                    slot: 1,
                    field: 5,
                    mask: 0x04,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }, WireRequestPackedValue {
                    bits: 0x04,
                    bytes: &[0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }],
                }, WireRequestPackedField {
                    slot: 2,
                    field: 6,
                    mask: 0x08,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x61, 0x62, 0x6c, 0x65],
                }, WireRequestPackedValue {
                    bits: 0x08,
                    bytes: &[0x65, 0x76, 0x69, 0x63, 0x74, 0x69, 0x6f, 0x6e, 0x5f, 0x70, 0x72, 0x6f, 0x74, 0x65, 0x63, 0x74, 0x65, 0x64],
                }],
                }, WireRequestPackedField {
                    slot: 3,
                    field: 7,
                    mask: 0x10,
                    values: &[WireRequestPackedValue {
                    bits: 0x00,
                    bytes: &[0x64, 0x69, 0x73, 0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }, WireRequestPackedValue {
                    bits: 0x10,
                    bytes: &[0x61, 0x6c, 0x6c, 0x6f, 0x77, 0x65, 0x64],
                }],
                }],
                reserved_mask: 0xe0,
                constant_bits: 0x00,
            }, WireRequestStep::Conditional {
                selector: 0,
                expected: 0x01,
                steps: &[WireRequestStep::VarUIntField { field: 4 }],
            }],
            field_count: 8,
        },
    WireRequestLayout {
            steps: &[WireRequestStep::Constant { bytes: &[0x00] }, WireRequestStep::FixedField {
                field: 0,
                bytes: 8,
            }, WireRequestStep::FixedField {
                field: 1,
                bytes: 8,
            }],
            field_count: 2,
        }
];

/// Returns the wire-level request layout for one assigned opcode.
pub const fn wire_request_layout(opcode: Opcode) -> WireRequestLayout {
    WIRE_REQUEST_LAYOUTS[opcode.index()]
}


/// Generated numeric request field handles.
///
/// API-owned bindings import their operation module directly. The generic
/// dispatcher sees only indexes and never scans model role strings.
pub mod request_fields {
    /// Generated field handles for Ping.
    pub mod op_ping {

    }
    /// Generated field handles for Get.
    pub mod op_get {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
        /// Numeric index for itemId.
        pub const ITEM_ID: usize = 1;
    }
    /// Generated field handles for Set.
    pub mod op_set {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
        /// Numeric index for itemId.
        pub const ITEM_ID: usize = 1;
        /// Numeric index for value.
        pub const VALUE: usize = 2;
        /// Numeric index for condition.
        pub const CONDITION: usize = 3;
        /// Numeric index for expirationMode.
        pub const EXPIRATION_MODE: usize = 4;
        /// Numeric index for evictionMode.
        pub const EVICTION_MODE: usize = 5;
        /// Numeric index for ttlMilliseconds.
        pub const TTL_MILLISECONDS: usize = 6;
    }
    /// Generated field handles for Delete.
    pub mod op_delete {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
        /// Numeric index for itemId.
        pub const ITEM_ID: usize = 1;
    }
    /// Generated field handles for ExperimentalStats.
    pub mod op_experimental_stats {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
    }
    /// Generated field handles for ExperimentalSync.
    pub mod op_experimental_sync {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
    }
    /// Generated field handles for NamespaceOpen.
    pub mod op_namespace_open {
        /// Numeric index for name.
        pub const NAME: usize = 0;
        /// Numeric index for createIfMissing.
        pub const CREATE_IF_MISSING: usize = 1;
        /// Numeric index for policy.
        pub const POLICY: usize = 2;
        /// Numeric index for policy.defaultExpiration.
        pub const POLICY_DEFAULT_EXPIRATION: usize = 3;
        /// Numeric index for policy.defaultTtlMilliseconds.
        pub const POLICY_DEFAULT_TTL_MILLISECONDS: usize = 4;
        /// Numeric index for policy.expirationOverride.
        pub const POLICY_EXPIRATION_OVERRIDE: usize = 5;
        /// Numeric index for policy.defaultEviction.
        pub const POLICY_DEFAULT_EVICTION: usize = 6;
        /// Numeric index for policy.evictionOverride.
        pub const POLICY_EVICTION_OVERRIDE: usize = 7;
    }
    /// Generated field handles for NamespaceUpdatePolicy.
    pub mod op_namespace_update_policy {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
        /// Numeric index for expectedRevision.
        pub const EXPECTED_REVISION: usize = 1;
        /// Numeric index for policy.
        pub const POLICY: usize = 2;
        /// Numeric index for policy.defaultExpiration.
        pub const POLICY_DEFAULT_EXPIRATION: usize = 3;
        /// Numeric index for policy.defaultTtlMilliseconds.
        pub const POLICY_DEFAULT_TTL_MILLISECONDS: usize = 4;
        /// Numeric index for policy.expirationOverride.
        pub const POLICY_EXPIRATION_OVERRIDE: usize = 5;
        /// Numeric index for policy.defaultEviction.
        pub const POLICY_DEFAULT_EVICTION: usize = 6;
        /// Numeric index for policy.evictionOverride.
        pub const POLICY_EVICTION_OVERRIDE: usize = 7;
    }
    /// Generated field handles for NamespaceDelete.
    pub mod op_namespace_delete {
        /// Numeric index for namespaceId.
        pub const NAMESPACE_ID: usize = 0;
        /// Numeric index for expectedRevision.
        pub const EXPECTED_REVISION: usize = 1;
    }
}

/// Generated numeric response field handles.
///
/// API-owned bindings import their operation module directly. The generic
/// dispatcher sees only indexes and never scans model role strings.
pub mod response_fields {
    /// Generated field handles for Ping.
    pub mod op_ping {
        /// Numeric index for payload.
        pub const PAYLOAD: usize = 0;
    }
    /// Generated field handles for Get.
    pub mod op_get {
        /// Numeric index for value.
        pub const VALUE: usize = 0;
    }
    /// Generated field handles for Set.
    pub mod op_set {
        /// Numeric index for outcome.
        pub const OUTCOME: usize = 0;
    }
    /// Generated field handles for Delete.
    pub mod op_delete {
        /// Numeric index for deleted.
        pub const DELETED: usize = 0;
    }
    /// Generated field handles for ExperimentalStats.
    pub mod op_experimental_stats {
        /// Numeric index for json.
        pub const JSON: usize = 0;
    }
    /// Generated field handles for ExperimentalSync.
    pub mod op_experimental_sync {

    }
    /// Generated field handles for NamespaceOpen.
    pub mod op_namespace_open {
        /// Numeric index for descriptor.
        pub const DESCRIPTOR: usize = 0;
        /// Numeric index for descriptor.namespaceId.
        pub const DESCRIPTOR_NAMESPACE_ID: usize = 1;
        /// Numeric index for descriptor.revision.
        pub const DESCRIPTOR_REVISION: usize = 2;
        /// Numeric index for descriptor.policy.
        pub const DESCRIPTOR_POLICY: usize = 3;
        /// Numeric index for descriptor.policy.defaultExpiration.
        pub const DESCRIPTOR_POLICY_DEFAULT_EXPIRATION: usize = 4;
        /// Numeric index for descriptor.policy.defaultTtlMilliseconds.
        pub const DESCRIPTOR_POLICY_DEFAULT_TTL_MILLISECONDS: usize = 5;
        /// Numeric index for descriptor.policy.expirationOverride.
        pub const DESCRIPTOR_POLICY_EXPIRATION_OVERRIDE: usize = 6;
        /// Numeric index for descriptor.policy.defaultEviction.
        pub const DESCRIPTOR_POLICY_DEFAULT_EVICTION: usize = 7;
        /// Numeric index for descriptor.policy.evictionOverride.
        pub const DESCRIPTOR_POLICY_EVICTION_OVERRIDE: usize = 8;
        /// Numeric index for created.
        pub const CREATED: usize = 9;
    }
    /// Generated field handles for NamespaceUpdatePolicy.
    pub mod op_namespace_update_policy {
        /// Numeric index for descriptor.
        pub const DESCRIPTOR: usize = 0;
        /// Numeric index for descriptor.namespaceId.
        pub const DESCRIPTOR_NAMESPACE_ID: usize = 1;
        /// Numeric index for descriptor.revision.
        pub const DESCRIPTOR_REVISION: usize = 2;
        /// Numeric index for descriptor.policy.
        pub const DESCRIPTOR_POLICY: usize = 3;
        /// Numeric index for descriptor.policy.defaultExpiration.
        pub const DESCRIPTOR_POLICY_DEFAULT_EXPIRATION: usize = 4;
        /// Numeric index for descriptor.policy.defaultTtlMilliseconds.
        pub const DESCRIPTOR_POLICY_DEFAULT_TTL_MILLISECONDS: usize = 5;
        /// Numeric index for descriptor.policy.expirationOverride.
        pub const DESCRIPTOR_POLICY_EXPIRATION_OVERRIDE: usize = 6;
        /// Numeric index for descriptor.policy.defaultEviction.
        pub const DESCRIPTOR_POLICY_DEFAULT_EVICTION: usize = 7;
        /// Numeric index for descriptor.policy.evictionOverride.
        pub const DESCRIPTOR_POLICY_EVICTION_OVERRIDE: usize = 8;
    }
    /// Generated field handles for NamespaceDelete.
    pub mod op_namespace_delete {

    }
}


/// Generic response payload framing selected by the modeled operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationResponseFraming {
    Empty,
    Opaque,
    OptionalValues,
    FieldSequence,
}

/// Generic request framing consumed by transport-neutral executors.
///
/// Historical protocol-v1 routes are handled by adapters; generic server/client
/// code only needs this byte-shape class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationRequestFraming {
    Empty,
    Opaque,
    OrderedFields,
}

/// Generic field payload layout selected by the generated shape plan.
///
/// Empty and opaque are explicit non-field layouts. Dense is used only for
/// all-required flattened fixed-width plans. Sequence is the general fallback
/// for optional, variable, repeated, and nested values. OptionalValues is an
/// explicit fixed presence-table layout selected by the operation descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFieldLayout {
    Empty,
    Opaque,
    Sequence,
    Dense,
    OptionalValues,
}

/// Generic frame policy selected by the same shape plan as the payload
/// layout. Fixed-body framing is safe only when the generated plan has an
/// exact width; all variable shapes remain length-delimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationFramePolicy {
    LengthDelimited,
    FixedBody,
}

/// Framing-neutral view of one request or response layout plan.
///
/// The fields and widths are generated together with the frame policy so
/// parsers, encoders, and clients cannot independently rediscover the shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationLayoutPlan {
    pub framing: OperationLayoutFraming,
    pub frame: OperationFramePolicy,
    pub layout: OperationFieldLayout,
    /// Layout-owned optional-value framing, absent for every other layout.
    pub optional_value_codec: Option<OptionalValueCodec>,
    pub fields: &'static [OperationFieldPlan],
    pub exact_width: usize,
    pub max_width: usize,
    /// An API-owned adapter explicitly permits a composite opaque aggregate.
    pub opaque_aggregate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationLayoutFraming {
    Empty,
    Opaque,
    OrderedFields,
    OptionalValues,
    FieldSequence,
}

/// One ordered field in a generated request or response plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationFieldPlan {
    pub index: usize,
    pub role: &'static str,
    pub required: bool,
    pub parent_index: usize,
    pub encoded_width: usize,
    pub shape: &'static str,
    pub path: &'static [&'static str],
    pub codecs: &'static [&'static str],
    pub nested_codecs: &'static [&'static str],
    /// Fixed widths known for nested codecs; zero means variable/unknown.
    pub nested_widths: &'static [usize],
    pub nested_enum_values: &'static [&'static [&'static str]],
    pub nested_union_tags: &'static [&'static [u8]],
    pub union_tags: &'static [u8],
    pub enum_values: &'static [&'static str],
}

/// Canonical wire framing, field, and status metadata for one operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationWireSpec {
    pub request: OperationLayoutPlan,
    pub response: OperationLayoutPlan,
    /// Conservative maximum response payload bytes derived from the output shape.
    pub response_payload_bound: usize,
    /// Experimental operations are disabled unless the adapter explicitly
    /// enables the matching draft revision.
    pub experimental: bool,
    pub experimental_revision: Option<&'static str>,
    /// Out-of-band operations are not admitted on the protocol data plane.
    pub out_of_band: bool,
    pub success_statuses: &'static [Status],
    pub error_statuses: &'static [Status],
}

impl OperationWireSpec {
    /// Returns the request framing enum derived from the canonical layout.
    pub const fn request_framing(self) -> OperationRequestFraming {
        match self.request.framing {
            OperationLayoutFraming::Empty => OperationRequestFraming::Empty,
            OperationLayoutFraming::Opaque => OperationRequestFraming::Opaque,
            OperationLayoutFraming::OrderedFields
            | OperationLayoutFraming::FieldSequence
            | OperationLayoutFraming::OptionalValues => OperationRequestFraming::OrderedFields,
        }
    }

    /// Returns the response framing enum derived from the canonical layout.
    pub const fn response_framing(self) -> OperationResponseFraming {
        match self.response.framing {
            OperationLayoutFraming::Empty => OperationResponseFraming::Empty,
            OperationLayoutFraming::Opaque => OperationResponseFraming::Opaque,
            OperationLayoutFraming::OptionalValues => OperationResponseFraming::OptionalValues,
            OperationLayoutFraming::FieldSequence
            | OperationLayoutFraming::OrderedFields => OperationResponseFraming::FieldSequence,
        }
    }

    pub const fn request_layout(self) -> OperationFieldLayout {
        self.request.layout
    }

    pub const fn response_layout(self) -> OperationFieldLayout {
        self.response.layout
    }

    pub const fn request_plan(self) -> &'static [OperationFieldPlan] {
        self.request.fields
    }

    pub const fn response_plan(self) -> &'static [OperationFieldPlan] {
        self.response.fields
    }

    /// Returns whether this operation belongs to the stable v1 surface.
    pub const fn is_stable(self) -> bool {
        !self.experimental && !self.out_of_band
    }

    /// Returns whether an adapter-enabled experimental revision admits this
    /// operation.
    pub fn enabled(self, enable_experimental_api: bool, revision: Option<&str>) -> bool {
        if self.out_of_band {
            return false;
        }
        if !self.experimental {
            return true;
        }
        enable_experimental_api
            && self.experimental_revision.is_some_and(|expected| {
                revision.is_some_and(|actual| actual == expected)
            })
    }
}

/// Returns the canonical wire spec for one protocol operation.
pub const fn operation_wire_spec(opcode: Opcode) -> OperationWireSpec {
    OPERATION_WIRE_SPECS[opcode.index()]
}

/// Generated operation registry entry used by server bind-time validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OperationRegistryEntry {
    pub opcode: Opcode,
    pub wire: OperationWireSpec,
}

/// Wire-only operation descriptors in opcode order.
pub const OPERATION_WIRE_SPECS: [OperationWireSpec; Opcode::COUNT] = [
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::Empty,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Empty,
            optional_value_codec: None,
            fields: &[],
            exact_width: 0,
            max_width: 0,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Opaque,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Opaque,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "payload", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "PongPayload", path: &["payload"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response_payload_bound: 67_108_864,
        experimental: false,
        experimental_revision: None,
        out_of_band: false,
        success_statuses: &[Status::Ok],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Sequence,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "item_id", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "ItemId", path: &["itemId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Opaque,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Opaque,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "value", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "Value", path: &["value"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response_payload_bound: 67_108_864,
        experimental: false,
        experimental_revision: None,
        out_of_band: false,
        success_statuses: &[Status::Ok, Status::NotFound],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Sequence,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "item_id", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "ItemId", path: &["itemId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 2, role: "value", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "Value", path: &["value"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 3, role: "condition", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "SetCondition", path: &["condition"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["any", "if_absent", "if_present"] }, OperationFieldPlan { index: 4, role: "expiration_mode", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "ExpirationMode", path: &["expirationMode"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["inherit", "no_expiry", "explicit_ttl"] }, OperationFieldPlan { index: 5, role: "eviction_mode", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "EvictionMode", path: &["evictionMode"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["inherit", "evictable", "eviction_protected"] }, OperationFieldPlan { index: 6, role: "ttl_milliseconds", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "Long", path: &["ttlMilliseconds"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Empty,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Empty,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "outcome", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "SetOutcome", path: &["outcome"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["created", "replaced", "not_stored"] }],
            exact_width: 0,
            max_width: 0,
            opaque_aggregate: false,
        },
        response_payload_bound: 0,
        experimental: false,
        experimental_revision: None,
        out_of_band: false,
        success_statuses: &[Status::Created, Status::Replaced, Status::NotStored],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::NoCapacity, Status::PolicyConflict, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Sequence,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "item_id", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "ItemId", path: &["itemId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Empty,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Empty,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "deleted", required: true, parent_index: usize::MAX, encoded_width: 1, shape: "Boolean", path: &["deleted"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 1,
            max_width: 0,
            opaque_aggregate: false,
        },
        response_payload_bound: 0,
        experimental: false,
        experimental_revision: None,
        out_of_band: false,
        success_statuses: &[Status::Deleted, Status::NotFound],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::Conflict, Status::NamespaceNotFound, Status::NamespaceNotEmpty],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::FixedBody,
            layout: OperationFieldLayout::Dense,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 8,
            max_width: 8,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Opaque,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Opaque,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "json", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "String", path: &["json"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response_payload_bound: 67_108_864,
        experimental: true,
        experimental_revision: Some("draft-2026-08-19.4"),
        out_of_band: false,
        success_statuses: &[Status::Ok],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::FixedBody,
            layout: OperationFieldLayout::Dense,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 8,
            max_width: 8,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Empty,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Empty,
            optional_value_codec: None,
            fields: &[],
            exact_width: 0,
            max_width: 0,
            opaque_aggregate: false,
        },
        response_payload_bound: 0,
        experimental: true,
        experimental_revision: Some("draft-2026-08-19.4"),
        out_of_band: false,
        success_statuses: &[Status::Ok],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Sequence,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "name", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "String", path: &["name"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "create_if_missing", required: true, parent_index: usize::MAX, encoded_width: 1, shape: "Boolean", path: &["createIfMissing"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 2, role: "policy", required: false, parent_index: usize::MAX, encoded_width: 0, shape: "NamespacePolicy", path: &["policy"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 3, role: "default_expiration", required: false, parent_index: 2, encoded_width: 0, shape: "ExpirationDefault", path: &["policy", "defaultExpiration"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["no_expiry", "fixed_ttl"] }, OperationFieldPlan { index: 4, role: "default_ttl_milliseconds", required: false, parent_index: 2, encoded_width: 0, shape: "Long", path: &["policy", "defaultTtlMilliseconds"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 5, role: "expiration_override", required: false, parent_index: 2, encoded_width: 0, shape: "OverridePolicy", path: &["policy", "expirationOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }, OperationFieldPlan { index: 6, role: "default_eviction", required: false, parent_index: 2, encoded_width: 0, shape: "EvictionDefault", path: &["policy", "defaultEviction"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["evictable", "eviction_protected"] }, OperationFieldPlan { index: 7, role: "eviction_override", required: false, parent_index: 2, encoded_width: 0, shape: "OverridePolicy", path: &["policy", "evictionOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Opaque,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Opaque,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "descriptor", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "NamespaceDescriptor", path: &["descriptor"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "namespace_id", required: true, parent_index: 0, encoded_width: 8, shape: "Long", path: &["descriptor", "namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 2, role: "revision", required: true, parent_index: 0, encoded_width: 8, shape: "Long", path: &["descriptor", "revision"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 3, role: "policy", required: true, parent_index: 0, encoded_width: 0, shape: "NamespacePolicy", path: &["descriptor", "policy"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 4, role: "default_expiration", required: true, parent_index: 3, encoded_width: 0, shape: "ExpirationDefault", path: &["descriptor", "policy", "defaultExpiration"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["no_expiry", "fixed_ttl"] }, OperationFieldPlan { index: 5, role: "default_ttl_milliseconds", required: false, parent_index: 3, encoded_width: 0, shape: "Long", path: &["descriptor", "policy", "defaultTtlMilliseconds"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 6, role: "expiration_override", required: true, parent_index: 3, encoded_width: 0, shape: "OverridePolicy", path: &["descriptor", "policy", "expirationOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }, OperationFieldPlan { index: 7, role: "default_eviction", required: true, parent_index: 3, encoded_width: 0, shape: "EvictionDefault", path: &["descriptor", "policy", "defaultEviction"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["evictable", "eviction_protected"] }, OperationFieldPlan { index: 8, role: "eviction_override", required: true, parent_index: 3, encoded_width: 0, shape: "OverridePolicy", path: &["descriptor", "policy", "evictionOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }, OperationFieldPlan { index: 9, role: "created", required: true, parent_index: usize::MAX, encoded_width: 1, shape: "Boolean", path: &["created"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: true,
        },
        response_payload_bound: 67_108_864,
        experimental: false,
        experimental_revision: None,
        out_of_band: true,
        success_statuses: &[Status::Ok, Status::Created],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Sequence,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "expected_revision", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["expectedRevision"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 2, role: "policy", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "NamespacePolicy", path: &["policy"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 3, role: "default_expiration", required: true, parent_index: 2, encoded_width: 0, shape: "ExpirationDefault", path: &["policy", "defaultExpiration"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["no_expiry", "fixed_ttl"] }, OperationFieldPlan { index: 4, role: "default_ttl_milliseconds", required: false, parent_index: 2, encoded_width: 0, shape: "Long", path: &["policy", "defaultTtlMilliseconds"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 5, role: "expiration_override", required: true, parent_index: 2, encoded_width: 0, shape: "OverridePolicy", path: &["policy", "expirationOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }, OperationFieldPlan { index: 6, role: "default_eviction", required: true, parent_index: 2, encoded_width: 0, shape: "EvictionDefault", path: &["policy", "defaultEviction"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["evictable", "eviction_protected"] }, OperationFieldPlan { index: 7, role: "eviction_override", required: true, parent_index: 2, encoded_width: 0, shape: "OverridePolicy", path: &["policy", "evictionOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Opaque,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Opaque,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "descriptor", required: true, parent_index: usize::MAX, encoded_width: 0, shape: "NamespaceDescriptor", path: &["descriptor"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "namespace_id", required: true, parent_index: 0, encoded_width: 8, shape: "Long", path: &["descriptor", "namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 2, role: "revision", required: true, parent_index: 0, encoded_width: 8, shape: "Long", path: &["descriptor", "revision"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 3, role: "policy", required: true, parent_index: 0, encoded_width: 0, shape: "NamespacePolicy", path: &["descriptor", "policy"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 4, role: "default_expiration", required: true, parent_index: 3, encoded_width: 0, shape: "ExpirationDefault", path: &["descriptor", "policy", "defaultExpiration"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["no_expiry", "fixed_ttl"] }, OperationFieldPlan { index: 5, role: "default_ttl_milliseconds", required: false, parent_index: 3, encoded_width: 0, shape: "Long", path: &["descriptor", "policy", "defaultTtlMilliseconds"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 6, role: "expiration_override", required: true, parent_index: 3, encoded_width: 0, shape: "OverridePolicy", path: &["descriptor", "policy", "expirationOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }, OperationFieldPlan { index: 7, role: "default_eviction", required: true, parent_index: 3, encoded_width: 0, shape: "EvictionDefault", path: &["descriptor", "policy", "defaultEviction"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["evictable", "eviction_protected"] }, OperationFieldPlan { index: 8, role: "eviction_override", required: true, parent_index: 3, encoded_width: 0, shape: "OverridePolicy", path: &["descriptor", "policy", "evictionOverride"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &["allowed", "disallowed"] }],
            exact_width: 0,
            max_width: 67_108_864,
            opaque_aggregate: true,
        },
        response_payload_bound: 67_108_864,
        experimental: false,
        experimental_revision: None,
        out_of_band: true,
        success_statuses: &[Status::Ok],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::Conflict, Status::NamespaceNotFound],
    },
    OperationWireSpec {
        request: OperationLayoutPlan {
            framing: OperationLayoutFraming::OrderedFields,
            frame: OperationFramePolicy::FixedBody,
            layout: OperationFieldLayout::Dense,
            optional_value_codec: None,
            fields: &[OperationFieldPlan { index: 0, role: "namespace_id", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["namespaceId"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }, OperationFieldPlan { index: 1, role: "expected_revision", required: true, parent_index: usize::MAX, encoded_width: 8, shape: "Long", path: &["expectedRevision"], codecs: &[], nested_codecs: &[], nested_widths: &[], nested_enum_values: &[], nested_union_tags: &[], union_tags: &[], enum_values: &[] }],
            exact_width: 16,
            max_width: 16,
            opaque_aggregate: false,
        },
        response: OperationLayoutPlan {
            framing: OperationLayoutFraming::Empty,
            frame: OperationFramePolicy::LengthDelimited,
            layout: OperationFieldLayout::Empty,
            optional_value_codec: None,
            fields: &[],
            exact_width: 0,
            max_width: 0,
            opaque_aggregate: false,
        },
        response_payload_bound: 0,
        experimental: false,
        experimental_revision: None,
        out_of_band: true,
        success_statuses: &[Status::Deleted],
        error_statuses: &[Status::InvalidRequest, Status::TooLarge, Status::Overloaded, Status::Timeout, Status::Forbidden, Status::InternalError, Status::Conflict, Status::NamespaceNotFound, Status::NamespaceNotEmpty],
    }
];

/// Dense opcode-indexed registry used by server bind-time validation.
pub const OPERATION_REGISTRY: [OperationRegistryEntry; Opcode::COUNT] = [
    OperationRegistryEntry { opcode: Opcode::Ping, wire: OPERATION_WIRE_SPECS[0] },
    OperationRegistryEntry { opcode: Opcode::Get, wire: OPERATION_WIRE_SPECS[1] },
    OperationRegistryEntry { opcode: Opcode::Set, wire: OPERATION_WIRE_SPECS[2] },
    OperationRegistryEntry { opcode: Opcode::Delete, wire: OPERATION_WIRE_SPECS[3] },
    OperationRegistryEntry { opcode: Opcode::ExperimentalStats, wire: OPERATION_WIRE_SPECS[4] },
    OperationRegistryEntry { opcode: Opcode::ExperimentalSync, wire: OPERATION_WIRE_SPECS[5] },
    OperationRegistryEntry { opcode: Opcode::NamespaceOpen, wire: OPERATION_WIRE_SPECS[6] },
    OperationRegistryEntry { opcode: Opcode::NamespaceUpdatePolicy, wire: OPERATION_WIRE_SPECS[7] },
    OperationRegistryEntry { opcode: Opcode::NamespaceDelete, wire: OPERATION_WIRE_SPECS[8] },
];

/// Stable v1 operations generated from the model.
pub const STABLE_OPCODES: &[Opcode] = &[
    Opcode::Ping,
    Opcode::Get,
    Opcode::Set,
    Opcode::Delete,
];

/// Experimental operations that require an explicit adapter gate.
pub const EXPERIMENTAL_OPCODES: &[Opcode] = &[
    Opcode::ExperimentalStats,
    Opcode::ExperimentalSync,
];

/// Codec identifiers supported by the protocol adapters.
///
/// This is emitted from the same registry that language generators consume;
/// it is deliberately separate from the operation-local list below so a
/// malformed model cannot make an unknown codec appear supported merely by
/// mentioning it in a field plan.
pub const WIRE_CODEC_NAMES: &[&'static str] = &["bool_u8", "enum", "f64_be", "i32_be", "list", "map", "packed_f64_be", "raw_bytes", "u64_be", "union", "utf8"];

/// Canonical codec validator kinds shared by server validation and adapters.
///
/// The identifier remains an open string at the model boundary, while the
/// generated kind selects the reusable semantic adapter. Adding an operation
/// that reuses a codec therefore does not add a server branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecKind {
    BoolU8,
    Enum,
    F64Be,
    I32Be,
    List,
    Map,
    PackedF64Be,
    RawBytes,
    U64Be,
    Union,
    Utf8,
}

/// Canonical codec shape metadata shared by server validation and adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecWidth {
    Fixed(usize),
    Variable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecCardinality {
    Scalar,
    Repeated,
    Associative,
    Tagged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireCodecLengthEncoding {
    None,
    Byte,
    VarUInt,
}

/// Recursive codec proof metadata used by layout planners and adapters.
///
/// `max_width == usize::MAX` means that the codec has no finite bound at this
/// layer. A container is dense-safe only after a future shape descriptor
/// supplies fixed cardinality and child widths; the base codec registry alone
/// intentionally cannot make that claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireCodecDescriptor {
    pub name: &'static str,
    pub kind: WireCodecKind,
    pub width: WireCodecWidth,
    pub min_width: usize,
    pub max_width: usize,
    pub borrowable: bool,
    pub cardinality: WireCodecCardinality,
    pub length_encoding: WireCodecLengthEncoding,
    pub container: bool,
    pub recursive: bool,
}

pub const WIRE_CODEC_DESCRIPTORS: &[WireCodecDescriptor] = &[
    WireCodecDescriptor { name: "bool_u8", kind: WireCodecKind::BoolU8, width: WireCodecWidth::Fixed(1), min_width: 1, max_width: 1, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "enum", kind: WireCodecKind::Enum, width: WireCodecWidth::Variable, min_width: 0, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "f64_be", kind: WireCodecKind::F64Be, width: WireCodecWidth::Fixed(8), min_width: 8, max_width: 8, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "i32_be", kind: WireCodecKind::I32Be, width: WireCodecWidth::Fixed(4), min_width: 4, max_width: 4, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "list", kind: WireCodecKind::List, width: WireCodecWidth::Variable, min_width: 1, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Repeated, length_encoding: WireCodecLengthEncoding::VarUInt, container: true, recursive: true },
    WireCodecDescriptor { name: "map", kind: WireCodecKind::Map, width: WireCodecWidth::Variable, min_width: 1, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Associative, length_encoding: WireCodecLengthEncoding::VarUInt, container: true, recursive: true },
    WireCodecDescriptor { name: "packed_f64_be", kind: WireCodecKind::PackedF64Be, width: WireCodecWidth::Variable, min_width: 0, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Repeated, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "raw_bytes", kind: WireCodecKind::RawBytes, width: WireCodecWidth::Variable, min_width: 0, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "u64_be", kind: WireCodecKind::U64Be, width: WireCodecWidth::Fixed(8), min_width: 8, max_width: 8, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
    WireCodecDescriptor { name: "union", kind: WireCodecKind::Union, width: WireCodecWidth::Variable, min_width: 2, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Tagged, length_encoding: WireCodecLengthEncoding::Byte, container: true, recursive: true },
    WireCodecDescriptor { name: "utf8", kind: WireCodecKind::Utf8, width: WireCodecWidth::Variable, min_width: 0, max_width: usize::MAX, borrowable: true, cardinality: WireCodecCardinality::Scalar, length_encoding: WireCodecLengthEncoding::None, container: false, recursive: false },
];

/// Resolves a generated codec identifier to the shared protocol kind.
///
/// Server field validation calls this adapter directly; operation handlers do
/// not need a server-local forwarding wrapper for generic codec traversal.
pub fn wire_codec_kind(
    name: &str,
) -> Option<crate::codec::CodecKind> {
    match name {
        "bool_u8" => Some(crate::codec::CodecKind::BoolU8),
        "enum" => Some(crate::codec::CodecKind::Enum),
        "f64_be" => Some(crate::codec::CodecKind::F64Be),
        "i32_be" => Some(crate::codec::CodecKind::I32Be),
        "list" => Some(crate::codec::CodecKind::List),
        "map" => Some(crate::codec::CodecKind::Map),
        "packed_f64_be" => Some(crate::codec::CodecKind::PackedF64Be),
        "raw_bytes" => Some(crate::codec::CodecKind::RawBytes),
        "u64_be" => Some(crate::codec::CodecKind::U64Be),
        "union" => Some(crate::codec::CodecKind::Union),
        "utf8" => Some(crate::codec::CodecKind::Utf8),
        _ => None,
    }
}

/// Codec identifiers required by the generated operation plans.
///
/// This list is the model-owned support surface shared by server validation
/// and language-specific adapters. An adapter may implement only a subset,
/// but it must fail generation or bind-time validation explicitly instead of
/// silently treating a declared codec as opaque bytes.
pub const OPERATION_CODEC_NAMES: &[&'static str] = &[];

/// Returns the static generated operation registry.
pub const fn operation_registry() -> &'static [OperationRegistryEntry; Opcode::COUNT] {
    &OPERATION_REGISTRY
}
