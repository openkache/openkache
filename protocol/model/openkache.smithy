$version: "2"

namespace openkache.protocol

@trait(selector: "service")
structure wireContract {
    @required
    itemIdBytes: Integer

    @required
    maxValueBytes: Integer

    @required
    v2: WireV2

    @required
    v3: WireV3
}

/// Native binding ABI identifiers generated alongside the wire contract.
@trait(selector: "service")
structure ffiContract {
    @required
    abiVersion: Integer

    @required
    resultError: Integer

    @required
    resultOk: Integer

    @required
    resultValue: Integer

    @required
    resultNotFound: Integer

    @required
    resultCreated: Integer

    @required
    resultReplaced: Integer

    @required
    resultDeleted: Integer

    @required
    resultNotDeleted: Integer

    @required
    resultConnected: Integer

    @required
    resultNotStored: Integer

    @required
    setConditionNone: Integer

    @required
    setConditionIfAbsent: Integer

    @required
    setConditionIfPresent: Integer

    @required
    operationReconnect: Long

    @required
    connectionStateConnected: Integer

    @required
    connectionStateReconnecting: Integer

    @required
    connectionStateDisconnected: Integer

    @required
    connectionStateClosed: Integer

    @required
    connectionStateUnknown: Integer
}

/// Defaults shared by all client bindings and the native core.
@trait(selector: "service")
structure clientDefaults {
    @required
    serverName: String

    @required
    connectTimeoutMs: Integer

    @required
    requestTimeoutMs: Integer

    @required
    retryMaxAttempts: Integer

    @required
    maxInFlight: Integer

    @required
    compressionLevel: Integer

    @required
    compressionMinimumInputSize: Integer

    @required
    compressionMinimumSavings: Integer

    @required
    compressionLevelMin: Integer

    @required
    compressionLevelMax: Integer
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
    itemIdRootContext: String

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
    itemIdBytes: 32,
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
        alpn: "openkache/1",
        requestFixedBytes: 2,
        responseFixedBytes: 1,
        maxVaruintBytes: 9,
        setTtlFlag: 1,
        setIfAbsentFlag: 2,
        setIfPresentFlag: 4
    }
)
@ffiContract(
    abiVersion: 1,
    resultError: 0,
    resultOk: 1,
    resultValue: 2,
    resultNotFound: 3,
    resultCreated: 4,
    resultReplaced: 5,
    resultDeleted: 6,
    resultNotDeleted: 7,
    resultConnected: 8,
    resultNotStored: 9,
    setConditionNone: 0,
    setConditionIfAbsent: 1,
    setConditionIfPresent: 2,
    operationReconnect: 4294967041,
    connectionStateConnected: 0,
    connectionStateReconnecting: 1,
    connectionStateDisconnected: 2,
    connectionStateClosed: 3,
    connectionStateUnknown: 4
)
@clientDefaults(
    serverName: "localhost",
    connectTimeoutMs: 5000,
    requestTimeoutMs: 2000,
    retryMaxAttempts: 2,
    maxInFlight: 256,
    compressionLevel: 1,
    compressionMinimumInputSize: 1024,
    compressionMinimumSavings: 64,
    compressionLevelMin: 1,
    compressionLevelMax: 22
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
    itemIdRootContext: "OpenKache client item key root v1",
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
    version: "1"
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
}

structure SetOutput {
    @required
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    itemId: ItemId
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
