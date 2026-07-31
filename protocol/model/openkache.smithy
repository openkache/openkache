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

/// Cross-language v1 value container and protection contract.
@trait(selector: "service")
structure valueFormat {
    @required
    version: Integer

    @required
    maxVu128Bytes: Integer

    @required
    formatByteBytes: Integer

    @required
    formatCompressionMask: Byte

    @required
    formatEncryptionShift: Byte

    @required
    serializationRaw: Byte

    @required
    serializationJson: Byte

    @required
    compressionNone: Byte

    @required
    compressionZstandard: Byte

    @required
    encryptionNone: Byte

    @required
    encryptionCompact: Byte

    @required
    encryptionRobust: Byte

    @required
    compactSyntheticIvBytes: Integer

    @required
    robustNonceBytes: Integer

    @required
    robustTagBytes: Integer

    @required
    dataProtectionKeyBytes: Integer

    @required
    itemKeyRootContext: String

    @required
    aadDomain: String

    @required
    valueRootContext: String

    @required
    compactMacContext: String

    @required
    compactEncryptionContext: String

    @required
    robustContext: String
}

/// Legacy pre-v1 metadata envelope retained for the TypeScript adapter migration.
@trait(selector: "service")
structure valueEnvelope {
    @required
    magicAndVersionHex: String

    @required
    maxEncodingBytes: Integer

    @required
    maxTypeNameBytes: Integer

    @required
    jsonEncoding: String
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
@valueFormat(
    version: 1,
    maxVu128Bytes: 17,
    formatByteBytes: 1,
    formatCompressionMask: 15,
    formatEncryptionShift: 4,
    serializationRaw: 0,
    serializationJson: 1,
    compressionNone: 0,
    compressionZstandard: 1,
    encryptionNone: 0,
    encryptionCompact: 1,
    encryptionRobust: 2,
    compactSyntheticIvBytes: 16,
    robustNonceBytes: 12,
    robustTagBytes: 16,
    dataProtectionKeyBytes: 32,
    itemKeyRootContext: "OpenKache client item key root v1",
    aadDomain: "openkache/value-format/aad/v1",
    valueRootContext: "OpenKache value format v1 root key",
    compactMacContext: "OpenKache value format v1 AES-256-SIV-CMAC MAC key",
    compactEncryptionContext: "OpenKache value format v1 AES-256-SIV-CMAC encryption key",
    robustContext: "OpenKache value format v1 AES-256-GCM-SIV key"
)
@valueEnvelope(
    magicAndVersionHex: "4f4b5601",
    maxEncodingBytes: 64,
    maxTypeNameBytes: 65535,
    jsonEncoding: "json"
)
service OpenKache {
    version: "3"
    operations: [Ping, Get, Set, Delete, Stats, Sync]
}

@wireOpcode(value: 1)
operation Ping {
    input: PingInput
    output: PingOutput
}

@wireOpcode(value: 2)
operation Get {
    input: GetInput
    output: GetOutput
}

@wireOpcode(value: 3)
operation Set {
    input: SetInput
    output: SetOutput
}

@wireOpcode(value: 4)
operation Delete {
    input: DeleteInput
    output: DeleteOutput
}

@wireOpcode(value: 5)
operation Stats {
    input: StatsInput
    output: StatsOutput
}

@wireOpcode(value: 6)
operation Sync {
    input: SyncInput
    output: SyncOutput
}

blob Key

blob Value

structure PingInput {}

structure PingOutput {}

structure GetInput {
    @required
    key: Key
}

structure GetOutput {
    value: Value
}

structure SetInput {
    @required
    key: Key

    @required
    value: Value

    condition: SetCondition

    ttlMilliseconds: Long
}

structure SetOutput {
    @required
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    key: Key
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

    @wireStatus(value: 127)
    INTERNAL_ERROR = "internal_error"
}
