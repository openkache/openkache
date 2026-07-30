$version: "2"

namespace openkache.protocol

@trait(selector: "service")
structure wireContract {
    @required
    itemKeyBytes: Integer

    @required
    maxValueBytes: Integer

    @required
    v2: WireV2

    @required
    v3: WireV3
}

structure WireV2 {
    @required
    alpn: String

    @required
    requestHeaderBytes: Integer

    @required
    responseHeaderBytes: Integer

    @required
    setTtlBytes: Integer

    @required
    responseValueLengthMask: Long

    @required
    valueCompressedBit: Long

    @required
    valueEncryptedBit: Long

    @required
    setTtlBit: Long

    @required
    setIfAbsentBit: Long

    @required
    setIfPresentBit: Long
}

structure WireV3 {
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
}

@trait(selector: "operation")
structure wireOpcode {
    @required
    value: Byte
}

@trait(selector: "enum > member")
structure wireStatus {
    @required
    value: Byte
}

@wireContract(
    itemKeyBytes: 32,
    maxValueBytes: 67108864,
    v2: {
        alpn: "openkache/2",
        requestHeaderBytes: 9,
        responseHeaderBytes: 5,
        setTtlBytes: 8,
        responseValueLengthMask: 1073741823,
        valueCompressedBit: 2147483648,
        valueEncryptedBit: 1073741824,
        setTtlBit: 536870912,
        setIfAbsentBit: 268435456,
        setIfPresentBit: 134217728
    },
    v3: {
        alpn: "openkache/3",
        requestFixedBytes: 2,
        responseFixedBytes: 1,
        maxVaruintBytes: 9,
        setTtlFlag: 1,
        setIfAbsentFlag: 2,
        setIfPresentFlag: 4
    }
)
service OpenKache {
    version: "3"
    operations: [Ping, Get, Set, Delete, Stats, Sync]
}

@wireOpcode(value: 1)
operation Ping {}

@wireOpcode(value: 2)
operation Get {}

@wireOpcode(value: 3)
operation Set {}

@wireOpcode(value: 4)
operation Delete {}

@wireOpcode(value: 5)
operation Stats {}

@wireOpcode(value: 6)
operation Sync {}

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

    @wireStatus(value: 127)
    INTERNAL_ERROR = "internal_error"
}
