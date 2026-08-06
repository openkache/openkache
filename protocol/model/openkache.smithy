$version: "2"

namespace openkache.protocol

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. Client generators
/// map this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

/// Semantic role consumed by generated client adapters.
enum OperationFieldRole {
    PAYLOAD = "payload"
    NAMESPACE_ID = "namespace_id"
    ITEM_ID = "item_id"
    VALUE = "value"
    CONDITION = "condition"
    EXPIRATION_MODE = "expiration_mode"
    TTL_MILLISECONDS = "ttl_milliseconds"
    EVICTION_MODE = "eviction_mode"
    EXPECTED_REVISION = "expected_revision"
    NAME = "name"
    CREATE_IF_MISSING = "create_if_missing"
    POLICY = "policy"
    OUTCOME = "outcome"
    DELETED = "deleted"
    JSON = "json"
    DESCRIPTOR = "descriptor"
    CREATED = "created"
    REVISION = "revision"
    DEFAULT_EXPIRATION = "default_expiration"
    DEFAULT_TTL_MILLISECONDS = "default_ttl_milliseconds"
    EXPIRATION_OVERRIDE = "expiration_override"
    DEFAULT_EVICTION = "default_eviction"
    EVICTION_OVERRIDE = "eviction_override"
}

/// Gives a Smithy member its language-neutral semantic role.
///
/// Generated clients use this role instead of reproducing member names in
/// every language renderer. Renaming a member therefore changes the model
/// shape without requiring operation-specific generator edits.
@trait(selector: "member")
structure operationField {
    @required
    role: OperationFieldRole
}

/// Request scope used by a generated operation contract.
enum OperationScope {
    GLOBAL = "global"
    ITEM = "item"
    NAMESPACE = "namespace"
    NAMESPACE_MANAGEMENT = "namespace_management"
}

/// Native request shape used by a generated operation adapter.
enum OperationRequestKind {
    EMPTY = "empty"
    APPLICATION_VALUE = "application_value"
    SCOPED_ITEM = "scoped_item"
    SCOPED_NAMESPACE = "scoped_namespace"
    NAMESPACE_OPEN = "namespace_open"
    NAMESPACE_UPDATE_POLICY = "namespace_update_policy"
    NAMESPACE_DELETE = "namespace_delete"
}

/// Response payload contract used by the shared client core.
enum OperationResponseKind {
    EMPTY = "empty"
    PONG = "pong"
    APPLICATION_VALUE = "application_value"
    VALUE = "value"
    SET_OUTCOME = "set_outcome"
    DELETE_OUTCOME = "delete_outcome"
    STATS_JSON = "stats_json"
    NAMESPACE_DESCRIPTOR = "namespace_descriptor"
}

/// Retry policy used by a generated operation contract.
enum OperationRetryMode {
    ALWAYS = "always"
    NEVER = "never"
    WHEN_NOT_CREATING = "when_not_creating"
}

list OperationStatuses {
    member: String
}

/// Semantic contract consumed by the shared client core and language adapters.
@trait(selector: "operation")
structure operationContract {
    @required
    scope: OperationScope

    @required
    requestKind: OperationRequestKind

    @required
    responseKind: OperationResponseKind

    @required
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
    ECHO = "echo"
}

@operationContract(
    scope: "global",
    requestKind: "empty",
    responseKind: "pong",
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
    requestKind: "scoped_item",
    responseKind: "value",
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
    requestKind: "scoped_item",
    responseKind: "set_outcome",
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
    requestKind: "scoped_item",
    responseKind: "delete_outcome",
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
    requestKind: "scoped_namespace",
    responseKind: "stats_json",
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
    requestKind: "scoped_namespace",
    responseKind: "empty",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
)
operation Sync {
    input: SyncInput
    output: SyncOutput
}

/// Experimental API used to verify cross-language contract propagation.
@operationContract(
    scope: "global",
    requestKind: "application_value",
    responseKind: "application_value",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation Echo {
    input: EchoInput
    output: EchoOutput
}

@operationContract(
    scope: "namespace_management",
    requestKind: "namespace_open",
    responseKind: "namespace_descriptor",
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
    requestKind: "namespace_update_policy",
    responseKind: "namespace_descriptor",
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
    requestKind: "namespace_delete",
    responseKind: "empty",
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

structure PingInput {}
structure PingOutput {}

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

structure EchoInput {
    @required
    @operationField(role: "payload")
    message: String
}

structure EchoOutput {
    @required
    @operationField(role: "payload")
    message: String
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
