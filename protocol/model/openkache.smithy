$version: "2"

namespace openkache.protocol

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. Client generators
/// map this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

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

}

/// Numeric operation assignments reserved by the evolving protocol-v1 draft
/// profile. The transport generator emits only these opaque identifiers; API
/// modules still own every request/response codec and registration.
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

/// API shape declarations. The transport generator intentionally ignores these
/// operations; each API module owns its request/response codecs and registration.
operation Ping {
    input: PingInput
    output: PingOutput
}

operation Get {
    input: GetInput
    output: GetOutput
}

operation Set {
    input: SetInput
    output: SetOutput
}

operation Delete {
    input: DeleteInput
    output: DeleteOutput
}

operation Stats {
    input: StatsInput
    output: StatsOutput
}

operation Sync {
    input: SyncInput
    output: SyncOutput
}

operation NamespaceOpen {
    input: NamespaceOpenInput
    output: NamespaceOpenOutput
}

operation NamespaceUpdatePolicy {
    input: NamespaceUpdatePolicyInput
    output: NamespaceUpdatePolicyOutput
}

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
    payload: PongPayload
}

structure GetInput {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    itemId: ItemId
}

structure GetOutput {
    value: Value
}

structure SetInput {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    itemId: ItemId

    @required
    value: Value
    condition: SetCondition
    expirationMode: ExpirationMode
    evictionMode: EvictionMode

    @unsignedLong
    ttlMilliseconds: Long
}

structure SetOutput {
    @required
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    itemId: ItemId
}

structure DeleteOutput {
    @required
    deleted: Boolean
}

structure StatsOutput {
    @required
    json: String
}

structure StatsInput {
    @required
    @unsignedLong
    namespaceId: Long
}

structure SyncInput {
    @required
    @unsignedLong
    namespaceId: Long
}

structure SyncOutput {}

structure NamespaceOpenInput {
    @required
    name: String

    @required
    createIfMissing: Boolean
    policy: NamespacePolicy
}

structure NamespaceOpenOutput {
    @required
    descriptor: NamespaceDescriptor

    @required
    created: Boolean
}

structure NamespaceUpdatePolicyInput {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    @unsignedLong
    expectedRevision: Long

    @required
    policy: NamespacePolicy
}

structure NamespaceUpdatePolicyOutput {
    @required
    descriptor: NamespaceDescriptor
}

structure NamespaceDeleteInput {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    @unsignedLong
    expectedRevision: Long
}

structure NamespaceDeleteOutput {}

structure NamespaceDescriptor {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    @unsignedLong
    revision: Long

    @required
    policy: NamespacePolicy
}

structure NamespacePolicy {
    @required
    defaultExpiration: ExpirationDefault

    @unsignedLong
    defaultTtlMilliseconds: Long

    @required
    expirationOverride: OverridePolicy

    @required
    defaultEviction: EvictionDefault

    @required
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
