$version: "2"

namespace openkache.client

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. The custom
/// generator maps this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

/// Defaults used by the shared client core and language adapters.
@trait(selector: "service")
structure clientDefaults {
    @required
    maxInFlight: Integer

    @required
    connectTimeoutMilliseconds: Long

    @required
    requestTimeoutMilliseconds: Long

    @required
    retryMaxAttempts: Integer

    @required
    zstandardLevel: Integer

    @required
    zstandardMinimumInputBytes: Integer

    @required
    zstandardMinimumSavingsBytes: Integer

    /// Stable TLS server-name default used by adapters without a platform-specific default.
    @required
    serverName: String

    /// PEM label for certificate chains assembled by an adapter.
    @required
    certificatePemType: String

    /// Minimum positive value for settings where zero selects a default.
    @required
    minimumPositiveValue: Integer

    /// Inclusive lower bound accepted by Zstandard adapters.
    @required
    zstandardLevelMin: Integer

    /// Inclusive upper bound accepted by Zstandard adapters.
    @required
    zstandardLevelMax: Integer
}

/// Native binding ABI identifiers shared by language adapters.
@trait(selector: "service")
structure ffiContract {
    @required
    abiVersion: Integer

    @required
    operationGetJson: Integer

    @required
    operationSetJson: Integer

    @required
    operationReconnect: Long

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
    setConditionAny: Integer

    @required
    setConditionIfAbsent: Integer

    @required
    setConditionIfPresent: Integer

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

/// Client-owned v1 value container and protection contract.
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

/// Legacy metadata envelope retained for the TypeScript adapter migration.
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

@clientDefaults(
    maxInFlight: 256,
    connectTimeoutMilliseconds: 5000,
    requestTimeoutMilliseconds: 2000,
    retryMaxAttempts: 2,
    zstandardLevel: 1,
    zstandardMinimumInputBytes: 1024,
    zstandardMinimumSavingsBytes: 64,
    serverName: "localhost",
    certificatePemType: "CERTIFICATE",
    minimumPositiveValue: 1,
    zstandardLevelMin: 1,
    zstandardLevelMax: 22
)
@ffiContract(
    abiVersion: 4,
    operationGetJson: 16,
    operationSetJson: 17,
    operationReconnect: 4294967041,
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
    setConditionAny: 0,
    setConditionIfAbsent: 1,
    setConditionIfPresent: 2,
    connectionStateConnected: 0,
    connectionStateReconnecting: 1,
    connectionStateDisconnected: 2,
    connectionStateClosed: 3,
    connectionStateUnknown: 4
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
service OpenKacheClient {
    version: "1"
    operations: [
        Ping,
        Get,
        Set,
        Delete,
        Stats,
        Sync,
        NamespaceOpen,
        NamespaceUpdatePolicy,
        NamespaceDelete
    ]
}

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

blob ItemId
blob Value

structure PingInput {}
structure PingOutput {}

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
