$version: "2"

namespace openkache.protocol

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
