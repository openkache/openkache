$version: "2"

namespace openkache.protocol

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. Client generators
/// map this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

/// Gives a Smithy member its language-neutral semantic role.
///
/// Generated clients use this role instead of reproducing member names in
/// every language renderer. Roles are intentionally open strings: the shared
/// generator validates only the structural contract it owns, while a server
/// extension may introduce a new semantic role without editing a central enum.
@trait(selector: "member")
structure operationField {
    @required
    role: String
}

/// Selects a registered payload codec for a wire-visible Smithy shape.
///
/// The operation contract never names a language-specific encoder. A codec is
/// resolved once by the shared generator registry and then rendered for every
/// client language.
@trait(selector: "member")
structure wireCodec {
    @required
    name: String

    /// Optional exact encoded width in bytes. This is useful for fixed-size
    /// byte records while keeping the codec name independent of a domain type.
    width: Integer
}

/// Retry policy retained by legacy client adapters.
enum OperationRetryMode {
    ALWAYS = "always"
    NEVER = "never"
    WHEN_NOT_CREATING = "when_not_creating"
}

list OperationStatuses {
    member: String
}

/// One canonical field-to-bit mapping in a packed request byte.
structure WirePackedValue {
    @required
    value: String

    @required
    bits: Integer
}

list WirePackedValues {
    member: WirePackedValue
}

/// One modeled field projected into a packed request byte.
structure WirePackedField {
    @required
    field: String

    @required
    mask: Integer

    @required
    values: WirePackedValues
}

list WirePackedFields {
    member: WirePackedField
}

structure WireFixedField {
    @required
    field: String

    @required
    bytes: Integer
}

structure WirePacked {
    @required
    fields: WirePackedFields

    reservedMask: Integer
    constantBits: Integer
}

structure WireFieldReference {
    @required
    field: String
}

structure WireConditional {
    @required
    field: String

    @required
    equals: String

    @required
    steps: WireRequestSteps
}

structure WireConstant {
    /// An even-length lowercase hexadecimal byte string.
    @required
    hex: String
}

structure WireTrailingField {
    @required
    field: String

    /// Length prefix used immediately before the trailing application bytes.
    @required
    length: String
}

structure WireValueLengthField {
    @required
    field: String

    /// Length prefix used before a later metadata field and the final value body.
    @required
    length: String
}

/// Emits only the one-octet length prefix for a later byte field body.
structure WireByteLengthPrefixField {
    @required
    field: String
}

union WireRequestStep {
    fixedField: WireFixedField
    packed: WirePacked
    byteLengthField: WireFieldReference
    byteLengthPrefixField: WireByteLengthPrefixField
    byteField: WireFieldReference
    varuintField: WireFieldReference
    valueLengthField: WireValueLengthField
    conditional: WireConditional
    constant: WireConstant
    trailingField: WireTrailingField
}

list WireRequestSteps {
    member: WireRequestStep
}

/// Operation framing and status contract shared by wire adapters.
///
/// Client-only members (`scope`, `responseSemantics`, and `retryMode`) are
/// projected by the client extractor and remain opaque to generic server
/// infrastructure.
@trait(selector: "operation")
structure operationContract {
    /// Optional client scope label. It is not emitted in the canonical wire
    /// artifact and must not be consumed by server dispatch. Client adapters
    /// may use an open value such as tenant, partition, or transaction.
    scope: String

    /// Optional declarative request-wire plan for a compact byte contract.
    requestWire: WireRequestSteps

    /// Generic request framing shared by protocol adapters.
    @required
    requestFraming: String

    /// Generic response framing shared by protocol adapters.
    @required
    responseFraming: String

    /// Marks an adapter-owned aggregate payload whose historical response is
    /// intentionally opaque despite having several modeled fields.
    opaqueAggregate: Boolean

    /// Optional open semantic result label consumed by an API-owned client
    /// adapter. Without one, clients receive the generic raw result envelope.
    responseSemantics: String

    /// Optional client retry policy. The generic projection defaults to
    /// `always`; server execution does not consume this value.
    retryMode: OperationRetryMode

    /// Marks a modeled operation as outside the stable v1 conformance
    /// surface. Experimental operations remain in the generated descriptor so
    /// an adapter can gate them explicitly instead of treating their opcode
    /// as a stable route.
    experimental: Boolean

    /// Exact draft revision required when `experimental` is true.
    experimentalRevision: String

    /// Keeps an operation modeled for private/control-plane adapters without
    /// assigning it to the protocol data-plane registry.
    outOfBand: Boolean

    @required
    successStatuses: OperationStatuses

    @required
    errorStatuses: OperationStatuses

}

/// Values that are visible on the client/server wire.
@trait(selector: "service")
structure wireContract {
    @required
    itemIdBytes: Integer

    @required
    maxValueBytes: Integer

    @required
    v1: WireV1
}

structure WireV1 {
    @required
    alpn: String

    @required
    opcodeBytes: Integer

    @required
    statusBytes: Integer

    @required
    requestFixedBytes: Integer

    @required
    responseFixedBytes: Integer

    @required
    maxVaruintBytes: Integer

    @required
    minVaruintBytes: Integer

    @required
    namespaceIdBytes: Integer

    @required
    namespaceRevisionBytes: Integer

    @required
    namespaceNameLengthBytes: Integer

    @required
    namespaceNameMaxBytes: Integer

    /// Width of each optional-value response length prefix.
    @required
    optionalValueLengthBytes: Integer

    /// Sentinel reserved for a missing optional value.
    @required
    optionalValueMissing: Long

    @required
    setFlagsBytes: Integer

    @required
    setConditionMask: Byte

    @required
    setConditionAnyBits: Byte

    @required
    setIfAbsentFlag: Byte

    @required
    setIfPresentFlag: Byte

    @required
    setConditionReservedBits: Byte

    @required
    setExpirationMask: Byte

    @required
    setInheritExpirationBits: Byte

    @required
    setNoExpiryBits: Byte

    @required
    setTtlFlag: Byte

    @required
    setExpirationReservedBits: Byte

    @required
    setEvictionMask: Byte

    @required
    setInheritEvictionBits: Byte

    @required
    setEvictableBits: Byte

    @required
    setEvictionProtectedBits: Byte

    @required
    setEvictionReservedBits: Byte

    @required
    setReservedMask: Integer

    @required
    openFlagsBytes: Integer

    @required
    openCreateIfMissingFlag: Byte

    @required
    openReservedMask: Integer

    @required
    deleteFlagsBytes: Integer

    @required
    deleteIfEmptyBits: Byte

    @required
    deleteModeMask: Byte

    @required
    deleteReservedMask: Integer

    @required
    policyFlagsBytes: Integer

    @required
    policyDefaultExpirationMask: Byte

    @required
    policyNoExpiryBits: Byte

    @required
    policyFixedTtlBits: Byte

    @required
    policyDefaultExpirationReservedBits: Byte

    @required
    policyExpirationOverrideFlag: Byte

    @required
    policyEvictionProtectedFlag: Byte

    @required
    policyEvictionOverrideFlag: Byte

    @required
    policyReservedMask: Integer

    @required
    errorStatusMinimum: Integer
}

/// Transitional numeric metadata for modeled operations.
///
/// A member annotated here is not necessarily a stable-v1 assignment:
/// experimental operations and out-of-band control-plane operations remain in
/// this model for adapter generation. Before the draft is finalized,
/// `protocol/SPEC.md` owns stable-v1 assignments.
@trait(selector: "enum > member")
structure wireOpcode {
    @required
    value: Byte
}

/// Transitional numeric response-status metadata.
///
/// Legacy and experimental statuses may remain in the generated enum. Before
/// the draft is finalized, `protocol/SPEC.md` and `EXPERIMENTAL.md` own stable
/// status applicability and assignment.
@trait(selector: "enum > member")
structure wireStatus {
    @required
    value: Integer
}

@wireContract(
    itemIdBytes: 32,
    maxValueBytes: 67108864,
    v1: {
        alpn: "openkache/1",
        opcodeBytes: 1,
        statusBytes: 1,
        requestFixedBytes: 1,
        responseFixedBytes: 1,
        maxVaruintBytes: 9,
        minVaruintBytes: 1,
        namespaceIdBytes: 8,
        namespaceRevisionBytes: 8,
        namespaceNameLengthBytes: 1,
        namespaceNameMaxBytes: 255,
        optionalValueLengthBytes: 4,
        optionalValueMissing: 4294967295,
        setFlagsBytes: 1,
        setConditionMask: 3,
        setConditionAnyBits: 0,
        setIfAbsentFlag: 1,
        setIfPresentFlag: 2,
        setConditionReservedBits: 3,
        setExpirationMask: 12,
        setInheritExpirationBits: 0,
        setNoExpiryBits: 4,
        setTtlFlag: 8,
        setExpirationReservedBits: 12,
        setEvictionMask: 48,
        setInheritEvictionBits: 0,
        setEvictableBits: 16,
        setEvictionProtectedBits: 32,
        setEvictionReservedBits: 48,
        setReservedMask: 192,
        openFlagsBytes: 1,
        openCreateIfMissingFlag: 1,
        openReservedMask: 254,
        deleteFlagsBytes: 1,
        deleteIfEmptyBits: 0,
        deleteModeMask: 3,
        deleteReservedMask: 252,
        policyFlagsBytes: 1,
        policyDefaultExpirationMask: 3,
        policyNoExpiryBits: 0,
        policyFixedTtlBits: 1,
        policyDefaultExpirationReservedBits: 3,
        policyExpirationOverrideFlag: 4,
        policyEvictionProtectedFlag: 8,
        policyEvictionOverrideFlag: 16,
        policyReservedMask: 224,
        errorStatusMinimum: 128
    }
)
service OpenKache {
    version: "1"
}

enum Opcode {
    @wireOpcode(value: 1)
    PING = "ping"

    @wireOpcode(value: 2)
    GET = "get"

    @wireOpcode(value: 3)
    SET = "set"

    @wireOpcode(value: 4)
    DELETE = "delete"

    @wireOpcode(value: 5)
    EXPERIMENTAL_STATS = "experimental_stats"

    @wireOpcode(value: 6)
    EXPERIMENTAL_SYNC = "experimental_sync"

    @wireOpcode(value: 7)
    NAMESPACE_OPEN = "namespace_open"

    @wireOpcode(value: 8)
    NAMESPACE_UPDATE_POLICY = "namespace_update_policy"

    @wireOpcode(value: 9)
    NAMESPACE_DELETE = "namespace_delete"

}

@operationContract(
    scope: "global",
    requestFraming: "empty",
    responseFraming: "opaque",
    responseSemantics: "pong",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation Ping {
    input: PingInput
    output: PingOutput
}

@operationContract(
    scope: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { byteLengthPrefixField: { field: "itemId" } },
        { byteField: { field: "itemId" } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "opaque",
    responseSemantics: "value",
    retryMode: "always",
    successStatuses: ["ok", "not_found"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation Get {
    input: GetInput
    output: GetOutput
}

@operationContract(
    scope: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        {
            packed: {
                fields: [
                    {
                        field: "condition",
                        mask: 3,
                        values: [
                            { value: "any", bits: 0 },
                            { value: "if_absent", bits: 1 },
                            { value: "if_present", bits: 2 }
                        ]
                    },
                    {
                        field: "expirationMode",
                        mask: 12,
                        values: [
                            { value: "inherit", bits: 0 },
                            { value: "no_expiry", bits: 4 },
                            { value: "explicit_ttl", bits: 8 }
                        ]
                    },
                    {
                        field: "evictionMode",
                        mask: 48,
                        values: [
                            { value: "inherit", bits: 0 },
                            { value: "evictable", bits: 16 },
                            { value: "eviction_protected", bits: 32 }
                        ]
                    }
                ],
                reservedMask: 192
            }
        },
        { byteLengthPrefixField: { field: "itemId" } },
        { valueLengthField: { field: "value", length: "varuint" } },
        {
            conditional: {
                field: "expirationMode",
                equals: "explicit_ttl",
                steps: [
                    { varuintField: { field: "ttlMilliseconds" } }
                ]
            }
        },
        { byteField: { field: "itemId" } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "empty",
    responseSemantics: "set_outcome",
    retryMode: "never",
    successStatuses: ["created", "replaced", "not_stored"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "no_capacity", "policy_conflict", "namespace_not_found"]
)
operation Set {
    input: SetInput
    output: SetOutput
}

@operationContract(
    scope: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { byteLengthPrefixField: { field: "itemId" } },
        { byteField: { field: "itemId" } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "empty",
    responseSemantics: "delete_outcome",
    retryMode: "never",
    successStatuses: ["deleted", "not_found"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "conflict", "namespace_not_found", "namespace_not_empty"]
)
operation Delete {
    input: DeleteInput
    output: DeleteOutput
}

@operationContract(
    scope: "namespace",
    experimental: true,
    experimentalRevision: "draft-2026-08-19.4",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "opaque",
    responseSemantics: "stats_json",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation ExperimentalStats {
    input: ExperimentalStatsInput
    output: ExperimentalStatsOutput
}

@operationContract(
    scope: "namespace",
    experimental: true,
    experimentalRevision: "draft-2026-08-19.4",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "empty",
    responseSemantics: "empty",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation ExperimentalSync {
    input: ExperimentalSyncInput
    output: ExperimentalSyncOutput
}

@operationContract(
    scope: "namespace_management",
    outOfBand: true,
    requestWire: [
        {
            packed: {
                fields: [
                    {
                        field: "createIfMissing",
                        mask: 1,
                        values: [
                            { value: "false", bits: 0 },
                            { value: "true", bits: 1 }
                        ]
                    }
                ],
                reservedMask: 254
            }
        },
        { byteLengthField: { field: "name" } },
        {
            conditional: {
                field: "createIfMissing",
                equals: "true",
                steps: [
                    {
                        packed: {
                            fields: [
                                {
                                    field: "policy.defaultExpiration",
                                    mask: 3,
                                    values: [
                                        { value: "no_expiry", bits: 0 },
                                        { value: "fixed_ttl", bits: 1 }
                                    ]
                                },
                                {
                                    field: "policy.expirationOverride",
                                    mask: 4,
                                    values: [
                                        { value: "disallowed", bits: 0 },
                                        { value: "allowed", bits: 4 }
                                    ]
                                },
                                {
                                    field: "policy.defaultEviction",
                                    mask: 8,
                                    values: [
                                        { value: "evictable", bits: 0 },
                                        { value: "eviction_protected", bits: 8 }
                                    ]
                                },
                                {
                                    field: "policy.evictionOverride",
                                    mask: 16,
                                    values: [
                                        { value: "disallowed", bits: 0 },
                                        { value: "allowed", bits: 16 }
                                    ]
                                }
                            ],
                            reservedMask: 224
                        }
                    },
                    {
                        conditional: {
                            field: "policy.defaultExpiration",
                            equals: "fixed_ttl",
                            steps: [
                                {
                                    varuintField: {
                                        field: "policy.defaultTtlMilliseconds"
                                    }
                                }
                            ]
                        }
                    }
                ]
            }
        }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "opaque",
    opaqueAggregate: true,
    responseSemantics: "namespace_descriptor",
    retryMode: "when_not_creating",
    successStatuses: ["ok", "created"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation NamespaceOpen {
    input: NamespaceOpenInput
    output: NamespaceOpenOutput
}

@operationContract(
    scope: "namespace_management",
    outOfBand: true,
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { fixedField: { field: "expectedRevision", bytes: 8 } },
        {
            packed: {
                fields: [
                    {
                        field: "policy.defaultExpiration",
                        mask: 3,
                        values: [
                            { value: "no_expiry", bits: 0 },
                            { value: "fixed_ttl", bits: 1 }
                        ]
                    },
                    {
                        field: "policy.expirationOverride",
                        mask: 4,
                        values: [
                            { value: "disallowed", bits: 0 },
                            { value: "allowed", bits: 4 }
                        ]
                    },
                    {
                        field: "policy.defaultEviction",
                        mask: 8,
                        values: [
                            { value: "evictable", bits: 0 },
                            { value: "eviction_protected", bits: 8 }
                        ]
                    },
                    {
                        field: "policy.evictionOverride",
                        mask: 16,
                        values: [
                            { value: "disallowed", bits: 0 },
                            { value: "allowed", bits: 16 }
                        ]
                    }
                ],
                reservedMask: 224
            }
        },
        {
            conditional: {
                field: "policy.defaultExpiration",
                equals: "fixed_ttl",
                steps: [
                    {
                        varuintField: {
                            field: "policy.defaultTtlMilliseconds"
                        }
                    }
                ]
            }
        }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "opaque",
    opaqueAggregate: true,
    responseSemantics: "namespace_descriptor",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "conflict", "namespace_not_found"]
)
operation NamespaceUpdatePolicy {
    input: NamespaceUpdatePolicyInput
    output: NamespaceUpdatePolicyOutput
}

@operationContract(
    scope: "namespace_management",
    outOfBand: true,
    requestWire: [
        { constant: { hex: "00" } },
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { fixedField: { field: "expectedRevision", bytes: 8 } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "empty",
    responseSemantics: "delete_outcome",
    retryMode: "never",
    successStatuses: ["deleted"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "conflict", "namespace_not_found", "namespace_not_empty"]
)
operation NamespaceDelete {
    input: NamespaceDeleteInput
    output: NamespaceDeleteOutput
}

blob ItemId
blob Value
blob PongPayload

structure PingInput {}
structure PingOutput {
    @required
    @operationField(role: "payload")
    payload: PongPayload
}

structure GetInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @operationField(role: "item_id")
    itemId: ItemId
}

structure GetOutput {
    @operationField(role: "value")
    value: Value
}

structure SetInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @operationField(role: "item_id")
    itemId: ItemId

    @required
    @operationField(role: "value")
    value: Value

    @operationField(role: "condition")
    condition: SetCondition

    @operationField(role: "expiration_mode")
    expirationMode: ExpirationMode

    @operationField(role: "eviction_mode")
    evictionMode: EvictionMode

    @unsignedLong
    @operationField(role: "ttl_milliseconds")
    ttlMilliseconds: Long
}

structure SetOutput {
    @required
    @operationField(role: "outcome")
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @operationField(role: "item_id")
    itemId: ItemId
}

structure DeleteOutput {
    @required
    @operationField(role: "deleted")
    deleted: Boolean
}

structure ExperimentalStatsOutput {
    @required
    @operationField(role: "json")
    json: String
}

structure ExperimentalStatsInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long
}

structure ExperimentalSyncInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long
}

structure ExperimentalSyncOutput {}

structure NamespaceOpenInput {
    @required
    @operationField(role: "name")
    name: String

    @required
    @operationField(role: "create_if_missing")
    createIfMissing: Boolean

    @operationField(role: "policy")
    policy: NamespacePolicy
}

structure NamespaceOpenOutput {
    @required
    @operationField(role: "descriptor")
    descriptor: NamespaceDescriptor

    @required
    @operationField(role: "created")
    created: Boolean
}

structure NamespaceUpdatePolicyInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @unsignedLong
    @operationField(role: "expected_revision")
    expectedRevision: Long

    @required
    @operationField(role: "policy")
    policy: NamespacePolicy
}

structure NamespaceUpdatePolicyOutput {
    @required
    @operationField(role: "descriptor")
    descriptor: NamespaceDescriptor
}

structure NamespaceDeleteInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @unsignedLong
    @operationField(role: "expected_revision")
    expectedRevision: Long
}

structure NamespaceDeleteOutput {}

structure NamespaceDescriptor {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @unsignedLong
    @operationField(role: "revision")
    revision: Long

    @required
    @operationField(role: "policy")
    policy: NamespacePolicy
}

structure NamespacePolicy {
    @required
    @operationField(role: "default_expiration")
    defaultExpiration: ExpirationDefault

    @unsignedLong
    @operationField(role: "default_ttl_milliseconds")
    defaultTtlMilliseconds: Long

    @required
    @operationField(role: "expiration_override")
    expirationOverride: OverridePolicy

    @required
    @operationField(role: "default_eviction")
    defaultEviction: EvictionDefault

    @required
    @operationField(role: "eviction_override")
    evictionOverride: OverridePolicy
}

enum SetCondition {
    ANY = "any"
    IF_ABSENT = "if_absent"
    IF_PRESENT = "if_present"
}

enum ExpirationMode {
    INHERIT = "inherit"
    NO_EXPIRY = "no_expiry"
    EXPLICIT_TTL = "explicit_ttl"
}

enum EvictionMode {
    INHERIT = "inherit"
    EVICTABLE = "evictable"
    EVICTION_PROTECTED = "eviction_protected"
}

enum OverridePolicy {
    ALLOWED = "allowed"
    DISALLOWED = "disallowed"
}

enum ExpirationDefault {
    NO_EXPIRY = "no_expiry"
    FIXED_TTL = "fixed_ttl"
}

enum EvictionDefault {
    EVICTABLE = "evictable"
    EVICTION_PROTECTED = "eviction_protected"
}

enum SetOutcome {
    CREATED = "created"
    REPLACED = "replaced"
    NOT_STORED = "not_stored"
}

enum Status {
    @wireStatus(value: 0)
    OK = "ok"

    @wireStatus(value: 1)
    NOT_FOUND = "not_found"

    @wireStatus(value: 2)
    CREATED = "created"

    @wireStatus(value: 3)
    REPLACED = "replaced"

    @wireStatus(value: 4)
    DELETED = "deleted"

    @wireStatus(value: 5)
    NOT_STORED = "not_stored"

    @wireStatus(value: 6)
    ACCEPTED = "accepted"

    @wireStatus(value: 128)
    INVALID_REQUEST = "invalid_request"

    @wireStatus(value: 129)
    UNSUPPORTED_OPCODE = "unsupported_opcode"

    @wireStatus(value: 130)
    TOO_LARGE = "too_large"

    @wireStatus(value: 131)
    OVERLOADED = "overloaded"

    @wireStatus(value: 132)
    TIMEOUT = "timeout"

    @wireStatus(value: 133)
    FORBIDDEN = "forbidden"

    @wireStatus(value: 134)
    INTERNAL_ERROR = "internal_error"

    @wireStatus(value: 135)
    NO_CAPACITY = "no_capacity"

    @wireStatus(value: 136)
    POLICY_CONFLICT = "policy_conflict"

    @wireStatus(value: 137)
    CONFLICT = "conflict"

    @wireStatus(value: 138)
    NAMESPACE_NOT_FOUND = "namespace_not_found"

    @wireStatus(value: 139)
    NAMESPACE_NOT_EMPTY = "namespace_not_empty"
}
