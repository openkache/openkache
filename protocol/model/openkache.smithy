$version: "2"

namespace openkache.protocol

/// Values that are visible on the client/server wire.
@trait(selector: "service")
structure wireContract {
    @required
    itemIdBytes: Integer

    @required
    mutationIdBytes: Integer

    @required
    maxValueBytes: Integer

    @required
    v1: WireV1
}

structure WireV1 {
    @required
    alpn: String

    @required
    requestFixedBytes: Integer

    @required
    responseFixedBytes: Integer

    @required
    maxVaruintBytes: Integer

    @required
    setTtlFlag: Byte

    @required
    setIfAbsentFlag: Byte

    @required
    setIfPresentFlag: Byte

    @required
    setMutationIdFlag: Byte
}

/// Operation-level framing, validation, and idempotency constraints.
@trait(selector: "operation")
structure operationSpec {
    @required
    itemIdBytes: Integer

    @required
    valueMinBytes: Integer

    @required
    valueMaxBytes: Integer

    @required
    allowedFlags: Byte

    @required
    ttlAllowed: Boolean

    @required
    mutation: Boolean

    @required
    successStatus: Byte
}

/// Numeric operation assignments used by protocol v1 frames.
@trait(selector: "operation")
structure wireOpcode {
    @required
    value: Byte
}

/// Numeric operation assignments used by protocol v1 enum members.
@trait(selector: "enum > member")
structure wireEnumOpcode {
    @required
    value: Byte
}

/// Numeric response-status assignments used by protocol v1 frames.
@trait(selector: "enum > member")
structure wireStatus {
    @required
    value: Byte
}

@wireContract(
    itemIdBytes: 32,
    mutationIdBytes: 16,
    maxValueBytes: 67108864,
    v1: {
        alpn: "openkache/1",
        requestFixedBytes: 2,
        responseFixedBytes: 1,
        maxVaruintBytes: 9,
        setTtlFlag: 1,
        setIfAbsentFlag: 2,
        setIfPresentFlag: 4,
        setMutationIdFlag: 8
    }
)
service OpenKache {
    version: "1"
    operations: [Ping, Get, Set, Delete, Stats, Sync]
}

enum Opcode {
    @wireEnumOpcode(value: 1)
    PING = "ping"

    @wireEnumOpcode(value: 2)
    GET = "get"

    @wireEnumOpcode(value: 3)
    SET = "set"

    @wireEnumOpcode(value: 4)
    DELETE = "delete"

    @wireEnumOpcode(value: 5)
    STATS = "stats"

    @wireEnumOpcode(value: 6)
    SYNC = "sync"
}

@wireOpcode(value: 1)
@operationSpec(
    itemIdBytes: 0,
    valueMinBytes: 0,
    valueMaxBytes: 0,
    allowedFlags: 0,
    ttlAllowed: false,
    mutation: false,
    successStatus: 0
)
operation Ping {
    input: PingInput
    output: PingOutput
}

@wireOpcode(value: 2)
@operationSpec(
    itemIdBytes: 32,
    valueMinBytes: 0,
    valueMaxBytes: 0,
    allowedFlags: 0,
    ttlAllowed: false,
    mutation: false,
    successStatus: 0
)
operation Get {
    input: GetInput
    output: GetOutput
}

@wireOpcode(value: 3)
@operationSpec(
    itemIdBytes: 32,
    valueMinBytes: 0,
    valueMaxBytes: 67108864,
    allowedFlags: 15,
    ttlAllowed: true,
    mutation: true,
    successStatus: 2
)
operation Set {
    input: SetInput
    output: SetOutput
}

@wireOpcode(value: 4)
@operationSpec(
    itemIdBytes: 32,
    valueMinBytes: 0,
    valueMaxBytes: 0,
    allowedFlags: 8,
    ttlAllowed: false,
    mutation: true,
    successStatus: 4
)
operation Delete {
    input: DeleteInput
    output: DeleteOutput
}

@wireOpcode(value: 5)
@operationSpec(
    itemIdBytes: 0,
    valueMinBytes: 0,
    valueMaxBytes: 0,
    allowedFlags: 0,
    ttlAllowed: false,
    mutation: false,
    successStatus: 0
)
operation Stats {
    input: StatsInput
    output: StatsOutput
}

@wireOpcode(value: 6)
@operationSpec(
    itemIdBytes: 0,
    valueMinBytes: 0,
    valueMaxBytes: 0,
    allowedFlags: 0,
    ttlAllowed: false,
    mutation: false,
    successStatus: 0
)
operation Sync {
    input: SyncInput
    output: SyncOutput
}

blob ItemId

blob Value

structure PingInput {}

structure PingOutput {}

structure GetInput {
    @required
    itemId: ItemId
}

structure GetOutput {
    value: Value
}

structure SetInput {
    @required
    itemId: ItemId

    @required
    value: Value

    condition: SetCondition

    ttlMilliseconds: Long

    /// Optional fixed-width idempotency token reused for mutation retries.
    mutationId: Blob
}

structure SetOutput {
    @required
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    itemId: ItemId

    /// Optional fixed-width idempotency token reused for mutation retries.
    mutationId: Blob
}

structure DeleteOutput {
    @required
    deleted: Boolean
}

structure StatsInput {}

structure StatsOutput {
    @required
    json: String
}

structure SyncInput {}

structure SyncOutput {}

enum SetCondition {
    IF_ABSENT = "if_absent"
    IF_PRESENT = "if_present"
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

    @wireStatus(value: 64)
    INVALID_REQUEST = "invalid_request"

    @wireStatus(value: 65)
    UNSUPPORTED_OPCODE = "unsupported_opcode"

    @wireStatus(value: 66)
    TOO_LARGE = "too_large"

    @wireStatus(value: 67)
    OVERLOADED = "overloaded"

    @wireStatus(value: 68)
    TIMEOUT = "timeout"

    @wireStatus(value: 69)
    FORBIDDEN = "forbidden"

    @wireStatus(value: 70)
    MUTATION_CONFLICT = "mutation_conflict"

    @wireStatus(value: 127)
    INTERNAL_ERROR = "internal_error"
}
