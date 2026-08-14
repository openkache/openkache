$version: "2"

namespace openkache.client

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. The custom
/// generator maps this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

/// Assigns a numeric discriminator to a native FFI enum member.
@trait(selector: "enum > member")
structure ffiValue {
    @required
    value: Long
}

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

    /// Connection encryption value selecting the configured default.
    ///
    /// The value is deliberately outside the value-format selector range so
    /// `encryptionNone` remains available for an explicit Unprotected
    /// connection profile.
    @required
    defaultEncryption: Long
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

    /// Selector protection field mask (bits 0..1).
    @required
    formatProtectionMask: Byte

    /// Selector compression field mask (bits 2..3).
    @required
    formatCompressionMask: Byte

    /// Selector compression field shift.
    @required
    formatCompressionShift: Byte

    /// Selector payload-format field mask (bits 4..5).
    @required
    formatPayloadMask: Byte

    /// Selector payload-format field shift.
    @required
    formatPayloadShift: Byte

    /// Selector reserved-bit mask (bits 6..7).
    @required
    formatReservedMask: Integer

    @required
    serializationOpaqueBytes: Byte

    @required
    serializationCbor: Byte

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
    clientRootKeyBytes: Integer

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
    abiVersion: 1,
    defaultEncryption: 4294967295
)
@valueFormat(
    version: 1,
    // A value-envelope version is an unsigned 64-bit vu128.  The largest
    // canonical encoding is nine bytes (the protocol's vu128 limit).
    maxVu128Bytes: 9,
    formatByteBytes: 1,
    formatProtectionMask: 3,
    formatCompressionMask: 12,
    formatCompressionShift: 2,
    formatPayloadMask: 48,
    formatPayloadShift: 4,
    formatReservedMask: 192,
    serializationOpaqueBytes: 0,
    serializationCbor: 1,
    compressionNone: 0,
    compressionZstandard: 1,
    encryptionNone: 0,
    encryptionCompact: 2,
    encryptionRobust: 1,
    compactSyntheticIvBytes: 16,
    robustNonceBytes: 12,
    robustTagBytes: 16,
    clientRootKeyBytes: 32,
    itemIdRootContext: "OpenKache item ID derivation root v1",
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

/// Flattened C-compatible projection used by the native client ABI.
///
/// This shape deliberately mirrors the fields returned by the canonical
/// descriptor decoder. Its member types and names are the source of truth for
/// every generated native binding. Its declaration order and member types
/// determine the natural C-compatible layout.
structure FfiNamespaceDescriptor {
    @required
    @unsignedLong
    namespaceId: Long

    @required
    @unsignedLong
    revision: Long

    @required
    @unsignedLong
    defaultTtlMs: Long

    @required
    defaultExpiration: Integer

    @required
    expirationOverride: Integer

    @required
    defaultEviction: Integer

    @required
    evictionOverride: Integer
}

enum FfiOperation {
    @ffiValue(value: 16)
    GET_JSON = "get_json"

    @ffiValue(value: 17)
    SET_JSON = "set_json"

    @ffiValue(value: 4294967041)
    RECONNECT = "reconnect"
}

enum FfiResultKind {
    @ffiValue(value: 0)
    ERROR = "error"

    @ffiValue(value: 1)
    OK = "ok"

    @ffiValue(value: 2)
    VALUE = "value"

    @ffiValue(value: 3)
    NOT_FOUND = "not_found"

    @ffiValue(value: 4)
    CREATED = "created"

    @ffiValue(value: 5)
    REPLACED = "replaced"

    @ffiValue(value: 6)
    DELETED = "deleted"

    @ffiValue(value: 7)
    NOT_DELETED = "not_deleted"

    @ffiValue(value: 8)
    CONNECTED = "connected"

    @ffiValue(value: 9)
    NOT_STORED = "not_stored"
}

enum FfiSetCondition {
    @ffiValue(value: 0)
    ANY = "any"

    @ffiValue(value: 1)
    IF_ABSENT = "if_absent"

    @ffiValue(value: 2)
    IF_PRESENT = "if_present"
}

/// Native discriminator for the typed key representation supplied by the FFI.
enum FfiKeySpec {
    @ffiValue(value: 0)
    TEXT = "text"

    @ffiValue(value: 1)
    BYTES = "bytes"

    @ffiValue(value: 2)
    INTEGER = "integer"
}

/// Native discriminator for the client-local Item ID mapping profile.
enum FfiKeyFormat {
    @ffiValue(value: 0)
    HASH = "hash"

    @ffiValue(value: 1)
    BYTE_KEY_OR_HASH = "byte_key_or_hash"
}

enum FfiConnectionState {
    @ffiValue(value: 0)
    CONNECTED = "connected"

    @ffiValue(value: 1)
    RECONNECTING = "reconnecting"

    @ffiValue(value: 2)
    DISCONNECTED = "disconnected"

    @ffiValue(value: 3)
    CLOSED = "closed"

    @ffiValue(value: 4)
    UNKNOWN = "unknown"
}

enum FfiNamespaceDescriptorDecodeStatus {
    @ffiValue(value: 0)
    OK = "ok"

    @ffiValue(value: 1)
    INVALID = "invalid"
}

enum FfiNamespaceDefaultExpiration {
    @ffiValue(value: 0)
    NO_EXPIRY = "no_expiry"

    @ffiValue(value: 1)
    FIXED_TTL = "fixed_ttl"
}

enum FfiNamespaceDefaultEviction {
    @ffiValue(value: 0)
    EVICTABLE = "evictable"

    @ffiValue(value: 1)
    PROTECTED = "eviction_protected"
}

enum FfiNamespaceOverridePolicy {
    @ffiValue(value: 0)
    DISALLOWED = "disallowed"

    @ffiValue(value: 1)
    ALLOWED = "allowed"
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
