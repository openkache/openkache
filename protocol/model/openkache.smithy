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

    /// Optional protocol-v1 compatibility route. Generic operations omit this
    /// member and select only a reusable request framing primitive.
    compactRoute: String

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

/// Numeric operation assignments used by protocol v1 frames.
@trait(selector: "enum > member")
structure wireOpcode {
    @required
    value: Byte
}

/// Numeric response-status assignments used by protocol v1 frames.
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
    STATS = "stats"

    @wireOpcode(value: 6)
    SYNC = "sync"

    @wireOpcode(value: 7)
    NAMESPACE_OPEN = "namespace_open"

    @wireOpcode(value: 8)
    NAMESPACE_UPDATE_POLICY = "namespace_update_policy"

    @wireOpcode(value: 9)
    NAMESPACE_DELETE = "namespace_delete"

    @wireOpcode(value: 10)
    EXPERIMENTAL_ECHO = "experimental_echo"

    @wireOpcode(value: 11)
    EXPERIMENTAL_REVERSE = "experimental_reverse"

    @wireOpcode(value: 12)
    SQUARE_ARRAY = "square_array"

    @wireOpcode(value: 13)
    GET2 = "get2"

    @wireOpcode(value: 14)
    EXPERIMENTAL_ACKNOWLEDGE = "experimental_acknowledge"

    @wireOpcode(value: 15)
    EXPERIMENTAL_DENSE = "experimental_dense"

    @wireOpcode(value: 32)
    EXPERIMENTAL_STORAGE_READ = "experimental_storage_read"

    @wireOpcode(value: 33)
    EXPERIMENTAL_PAGE = "experimental_page"

    @wireOpcode(value: 34)
    EXPERIMENTAL_MULTI_RESOURCE_MUTATION = "experimental_multi_resource_mutation"

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
    scope: "global",
    requestFraming: "opaque",
    responseFraming: "opaque",
    responseSemantics: "application_value",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalEcho {
    input: ExperimentalEchoInput
    output: ExperimentalEchoOutput
}

@operationContract(
    scope: "global",
    requestFraming: "opaque",
    responseFraming: "empty",
    responseSemantics: "accepted",
    retryMode: "always",
    successStatuses: ["accepted"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalAcknowledge {
    input: ExperimentalAcknowledgeInput
    output: ExperimentalAcknowledgeOutput
}

@operationContract(
    scope: "global",
    requestFraming: "ordered_fields",
    responseFraming: "field_sequence",
    responseSemantics: "values",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalDense {
    input: ExperimentalDenseInput
    output: ExperimentalDenseOutput
}

@operationContract(
    scope: "global",
    requestFraming: "opaque",
    responseFraming: "opaque",
    retryMode: "always",
    successStatuses: ["ok", "not_found"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalStorageRead {
    input: ExperimentalStorageReadInput
    output: ExperimentalStorageReadOutput
}

@operationContract(
    scope: "global",
    requestFraming: "opaque",
    responseFraming: "opaque",
    responseSemantics: "application_value",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalReverse {
    input: ExperimentalReverseInput
    output: ExperimentalReverseOutput
}

@operationContract(
    scope: "global",
    requestFraming: "opaque",
    responseFraming: "opaque",
    responseSemantics: "application_value",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation SquareArray {
    input: SquareArrayInput
    output: SquareArrayOutput
}

@operationContract(
    scope: "global",
    requestFraming: "ordered_fields",
    responseFraming: "field_sequence",
    responseSemantics: "page",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation ExperimentalPage {
    input: ExperimentalPageInput
    output: ExperimentalPageOutput
}

@operationContract(
    scope: "global",
    requestFraming: "ordered_fields",
    responseFraming: "field_sequence",
    responseSemantics: "receipt",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "conflict"]
)
operation ExperimentalMultiResourceMutation {
    input: ExperimentalMultiResourceMutationInput
    output: ExperimentalMultiResourceMutationOutput
}

@operationContract(
    scope: "item",
    compactRoute: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { byteLengthField: { field: "itemId" } }
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
    compactRoute: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { byteLengthField: { field: "itemIdA" } },
        { byteLengthField: { field: "itemIdB" } }
    ],
    requestFraming: "ordered_fields",
    responseFraming: "optional_values",
    responseSemantics: "values",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation Get2 {
    input: Get2Input
    output: Get2Output
}

@operationContract(
    scope: "item",
    compactRoute: "set",
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
    compactRoute: "item",
    requestWire: [
        { fixedField: { field: "namespaceId", bytes: 8 } },
        { byteLengthField: { field: "itemId" } }
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
    compactRoute: "namespace",
    requestFraming: "ordered_fields",
    responseFraming: "opaque",
    responseSemantics: "stats_json",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation Stats {
    input: StatsInput
    output: StatsOutput
}

@operationContract(
    scope: "namespace",
    compactRoute: "namespace",
    requestFraming: "ordered_fields",
    responseFraming: "empty",
    responseSemantics: "empty",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation Sync {
    input: SyncInput
    output: SyncOutput
}

@operationContract(
    scope: "namespace_management",
    compactRoute: "namespace_open",
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
    compactRoute: "namespace_update_policy",
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
    compactRoute: "namespace_delete",
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

/// Dense finite IEEE-754 binary64 values. Application-value operations encode
/// each value as one big-endian eight-octet payload with no count prefix.
list FloatingPointArray {
    member: Double
}

list ExperimentalPageItems {
    member: Value
}

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

structure Get2Input {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long

    @required
    @operationField(role: "item_id")
    itemIdA: ItemId

    @required
    @operationField(role: "item_id")
    itemIdB: ItemId
}

structure Get2Output {
    @operationField(role: "value")
    valueA: Value

    @operationField(role: "value")
    valueB: Value
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

structure StatsOutput {
    @required
    @operationField(role: "json")
    json: String
}

structure StatsInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long
}

structure SyncInput {
    @required
    @unsignedLong
    @operationField(role: "namespace_id")
    namespaceId: Long
}

structure SyncOutput {}

structure ExperimentalEchoInput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "utf8")
    message: String
}

structure ExperimentalEchoOutput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "utf8")
    message: String
}

structure ExperimentalAcknowledgeInput {
    @required
    @operationField(role: "token")
    @wireCodec(name: "utf8")
    token: String
}

structure ExperimentalAcknowledgeOutput {}

structure ExperimentalDenseInput {
    @required
    @unsignedLong
    @operationField(role: "counter")
    @wireCodec(name: "u64_be")
    counter: Long

    @required
    @operationField(role: "enabled")
    @wireCodec(name: "bool_u8")
    enabled: Boolean
}

structure ExperimentalDenseOutput {
    @required
    @unsignedLong
    @operationField(role: "counter")
    @wireCodec(name: "u64_be")
    counter: Long

    @required
    @operationField(role: "enabled")
    @wireCodec(name: "bool_u8")
    enabled: Boolean
}

structure ExperimentalStorageReadInput {
    @required
    @wireCodec(name: "raw_bytes")
    @operationField(role: "key")
    key: Value
}

structure ExperimentalStorageReadOutput {
    @required
    @wireCodec(name: "raw_bytes")
    @operationField(role: "value")
    value: Value
}

structure ExperimentalReverseInput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "utf8")
    message: String
}

structure ExperimentalReverseOutput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "utf8")
    message: String
}

structure SquareArrayInput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "packed_f64_be")
    values: FloatingPointArray
}

structure SquareArrayOutput {
    @required
    @operationField(role: "payload")
    @wireCodec(name: "packed_f64_be")
    values: FloatingPointArray
}

structure ExperimentalPageInput {
    @operationField(role: "cursor")
    cursor: Value
}

structure ExperimentalPageOutput {
    @required
    @operationField(role: "items")
    @wireCodec(name: "list")
    items: ExperimentalPageItems

    @operationField(role: "next_cursor")
    nextCursor: Value
}

structure ExperimentalMultiResourceMutationInput {
    @required
    @operationField(role: "source_resource")
    sourceResource: Value

    @required
    @operationField(role: "target_resource")
    targetResource: Value

    @required
    @operationField(role: "payload")
    payload: Value
}

structure ExperimentalMultiResourceMutationOutput {
    @required
    @operationField(role: "receipt")
    receipt: Value
}

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
