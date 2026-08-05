$version: "2"

namespace openkache.client

/// Marks a Smithy Long member whose domain is the complete unsigned 64-bit range.
///
/// Smithy's built-in Long is signed, while OpenKache uses fixed-width unsigned
/// integers for namespace identities, revisions, and TTLs. The custom
/// generator maps this trait to each language's unsigned 64-bit type.
@trait(selector: "member")
structure unsignedLong {}

/// Request scope used by a generated client operation contract.
enum OperationScope {
    GLOBAL = "global"
    ITEM = "item"
    NAMESPACE = "namespace"
    NAMESPACE_MANAGEMENT = "namespace_management"
}

/// Response payload contract used by the shared client core.
enum OperationResponseKind {
    EMPTY = "empty"
    PONG = "pong"
    ECHO = "echo"
    VALUE = "value"
    SET_OUTCOME = "set_outcome"
    DELETE_OUTCOME = "delete_outcome"
    STATS_JSON = "stats_json"
    NAMESPACE_DESCRIPTOR = "namespace_descriptor"
}

/// Retry policy used by a generated client operation contract.
enum OperationRetryMode {
    ALWAYS = "always"
    NEVER = "never"
    WHEN_NOT_CREATING = "when_not_creating"
}

list OperationStatuses {
    member: String
}

/// Semantic contract consumed by the shared client core.
@trait(selector: "operation")
structure operationContract {
    @required
    scope: OperationScope

    @required
    responseKind: OperationResponseKind

    @required
    retryMode: OperationRetryMode

    @required
    successStatuses: OperationStatuses

    @required
    errorStatuses: OperationStatuses
}

/// Input buffer shape exposed by one native FFI operation.
enum FfiInputKind {
    NONE = "none"
    APPLICATION_KEY = "application_key"
    ITEM_ID = "item_id"
}

/// Dispatch and buffer contract for an operation that exists only in the native ABI.
@trait(selector: "enum > member")
structure ffiOperationContract {
    @required
    inputKind: FfiInputKind

    @required
    acceptsValue: Boolean

    @required
    acceptsSetOptions: Boolean

    @required
    supportsProtected: Boolean

    @required
    supportsRaw: Boolean

    @required
    supportsScoped: Boolean

    @required
    dedicatedAbi: Boolean
}

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

/// Native scalar and pointer kinds used by the stable C ABI.
enum FfiNativeType {
    VOID = "void"
    CLIENT_POINTER = "client_pointer"
    RESULT_POINTER = "result_pointer"
    U8_POINTER = "u8_pointer"
    STRUCT_POINTER = "struct_pointer"
    SIZE = "size"
    UINT8 = "uint8"
    INT32 = "int32"
    UINT32 = "uint32"
    UINT64 = "uint64"
}

structure FfiNativeParameter {
    @required
    name: String

    @required
    type: FfiNativeType

    /// The pointed-to value is writable when this flag is true.
    @required
    mutable: Boolean

    /// Required for STRUCT_POINTER parameters.
    structureName: String
}

list FfiNativeParameters {
    member: FfiNativeParameter
}

structure FfiNativeFunction {
    @required
    name: String

    @required
    returnType: FfiNativeType

    /// Optional extension symbols may be absent from older native libraries.
    optional: Boolean

    parameters: FfiNativeParameters
}

list FfiNativeFunctions {
    member: FfiNativeFunction
}

structure FfiNativeField {
    @required
    name: String

    @required
    type: FfiNativeType

    /// The pointed-to value is writable when this flag is true.
    @required
    mutable: Boolean

    /// Required for STRUCT_POINTER fields.
    structureName: String
}

list FfiNativeFields {
    member: FfiNativeField
}

structure FfiNativeStructure {
    @required
    name: String

    @required
    fields: FfiNativeFields
}

list FfiNativeStructures {
    member: FfiNativeStructure
}

/// Native binding ABI identifiers and declarations shared by language adapters.
@trait(selector: "service")
structure ffiContract {
    @required
    abiVersion: Integer

    @required
    nativeFunctions: FfiNativeFunctions

    @required
    nativeStructures: FfiNativeStructures
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
    nativeFunctions: [
        {
            name: "openkache_client_abi_version",
            returnType: "uint32",
            parameters: []
        },
        {
            name: "openkache_client_connect",
            returnType: "result_pointer",
            parameters: [
                { name: "address", type: "u8_pointer", mutable: false },
                { name: "addressLength", type: "size", mutable: false },
                { name: "serverName", type: "u8_pointer", mutable: false },
                { name: "serverNameLength", type: "size", mutable: false },
                { name: "certificate", type: "u8_pointer", mutable: false },
                { name: "certificateLength", type: "size", mutable: false },
                { name: "dataProtectionKey", type: "u8_pointer", mutable: false },
                { name: "dataProtectionKeyLength", type: "size", mutable: false },
                { name: "compressionEnabled", type: "uint8", mutable: false },
                { name: "compressionLevel", type: "int32", mutable: false },
                { name: "minimumInputSize", type: "size", mutable: false },
                { name: "minimumSavings", type: "size", mutable: false },
                { name: "connectTimeoutMilliseconds", type: "uint64", mutable: false },
                { name: "requestTimeoutMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_connect_ex",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                { name: "address", type: "u8_pointer", mutable: false },
                { name: "addressLength", type: "size", mutable: false },
                { name: "serverName", type: "u8_pointer", mutable: false },
                { name: "serverNameLength", type: "size", mutable: false },
                { name: "certificate", type: "u8_pointer", mutable: false },
                { name: "certificateLength", type: "size", mutable: false },
                { name: "clientCertificateChain", type: "u8_pointer", mutable: false },
                { name: "clientCertificateChainLength", type: "size", mutable: false },
                { name: "clientPrivateKey", type: "u8_pointer", mutable: false },
                { name: "clientPrivateKeyLength", type: "size", mutable: false },
                { name: "dataProtectionKey", type: "u8_pointer", mutable: false },
                { name: "dataProtectionKeyLength", type: "size", mutable: false },
                { name: "compressionEnabled", type: "uint8", mutable: false },
                { name: "compressionLevel", type: "int32", mutable: false },
                { name: "minimumInputSize", type: "size", mutable: false },
                { name: "minimumSavings", type: "size", mutable: false },
                { name: "encryption", type: "uint32", mutable: false },
                { name: "retryMaxAttempts", type: "size", mutable: false },
                { name: "maxInFlight", type: "size", mutable: false },
                { name: "connectTimeoutMilliseconds", type: "uint64", mutable: false },
                { name: "requestTimeoutMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_connect_with_options",
            returnType: "result_pointer",
            parameters: [
                {
                    name: "options",
                    type: "struct_pointer",
                    structureName: "FfiConnectOptions",
                    mutable: false
                }
            ]
        },
        {
            name: "openkache_client_execute",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "applicationKey", type: "u8_pointer", mutable: false },
                { name: "applicationKeyLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setCondition", type: "uint32", mutable: false },
                { name: "ttlEnabled", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_raw",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "itemId", type: "u8_pointer", mutable: false },
                { name: "itemIdLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setCondition", type: "uint32", mutable: false },
                { name: "ttlEnabled", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_with_options",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "applicationKey", type: "u8_pointer", mutable: false },
                { name: "applicationKeyLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_raw_with_options",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "itemId", type: "u8_pointer", mutable: false },
                { name: "itemIdLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_scoped",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "namespaceId", type: "uint64", mutable: false },
                { name: "itemId", type: "u8_pointer", mutable: false },
                { name: "itemIdLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_namespace_open",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "name", type: "u8_pointer", mutable: false },
                { name: "nameLength", type: "size", mutable: false },
                { name: "createIfMissing", type: "uint8", mutable: false },
                { name: "policyFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_namespace_update_policy",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "namespaceId", type: "uint64", mutable: false },
                { name: "expectedRevision", type: "uint64", mutable: false },
                { name: "policyFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_namespace_delete",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "namespaceId", type: "uint64", mutable: false },
                { name: "expectedRevision", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_namespace_descriptor_decode",
            returnType: "uint32",
            parameters: [
                { name: "payload", type: "u8_pointer", mutable: false },
                { name: "payloadLength", type: "size", mutable: false },
                {
                    name: "output",
                    type: "struct_pointer",
                    structureName: "FfiNamespaceDescriptor",
                    mutable: true
                }
            ]
        },
        {
            name: "openkache_client_connection_state",
            optional: true,
            returnType: "uint32",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_kind",
            returnType: "uint32",
            parameters: [
                { name: "result", type: "result_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_data",
            returnType: "u8_pointer",
            parameters: [
                { name: "result", type: "result_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_data_length",
            returnType: "size",
            parameters: [
                { name: "result", type: "result_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_take_client",
            returnType: "client_pointer",
            parameters: [
                { name: "result", type: "result_pointer", mutable: true }
            ]
        },
        {
            name: "openkache_client_result_free",
            returnType: "void",
            parameters: [
                { name: "result", type: "result_pointer", mutable: true }
            ]
        },
        {
            name: "openkache_client_free",
            returnType: "void",
            parameters: [
                { name: "client", type: "client_pointer", mutable: true }
            ]
        }
    ],
    nativeStructures: [
        {
            name: "FfiConnectOptions",
            fields: [
                { name: "address", type: "u8_pointer", mutable: false },
                { name: "addressLength", type: "size", mutable: false },
                { name: "serverName", type: "u8_pointer", mutable: false },
                { name: "serverNameLength", type: "size", mutable: false },
                { name: "certificate", type: "u8_pointer", mutable: false },
                { name: "certificateLength", type: "size", mutable: false },
                { name: "clientCertificateChain", type: "u8_pointer", mutable: false },
                { name: "clientCertificateChainLength", type: "size", mutable: false },
                { name: "clientPrivateKey", type: "u8_pointer", mutable: false },
                { name: "clientPrivateKeyLength", type: "size", mutable: false },
                { name: "dataProtectionKey", type: "u8_pointer", mutable: false },
                { name: "dataProtectionKeyLength", type: "size", mutable: false },
                { name: "compressionEnabled", type: "uint8", mutable: false },
                { name: "compressionLevel", type: "int32", mutable: false },
                { name: "minimumInputSize", type: "size", mutable: false },
                { name: "minimumSavings", type: "size", mutable: false },
                { name: "encryption", type: "uint32", mutable: false },
                { name: "connectTimeoutMilliseconds", type: "uint64", mutable: false },
                { name: "requestTimeoutMilliseconds", type: "uint64", mutable: false },
                { name: "retryMaxAttempts", type: "size", mutable: false },
                { name: "maxInFlight", type: "size", mutable: false }
            ]
        }
    ]
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
}

@operationContract(
    scope: "global",
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
    responseKind: "empty",
    retryMode: "never",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "namespace_not_found"]
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

@operationContract(
    scope: "namespace_management",
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
    responseKind: "empty",
    retryMode: "never",
    successStatuses: ["deleted"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error", "conflict", "namespace_not_found", "namespace_not_empty"]
)
operation NamespaceDelete {
    input: NamespaceDeleteInput
    output: NamespaceDeleteOutput
}

/// Experimental API used to verify cross-language contract propagation.
@operationContract(
    scope: "global",
    responseKind: "echo",
    retryMode: "always",
    successStatuses: ["ok"],
    errorStatuses: ["invalid_request", "too_large", "overloaded", "timeout", "forbidden", "internal_error"]
)
operation Echo {
    input: EchoInput
    output: EchoOutput
}

structure EchoInput {
    @required
    message: String
}

structure EchoOutput {
    @required
    message: String
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
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: false,
        acceptsSetOptions: false,
        supportsProtected: true,
        supportsRaw: false,
        supportsScoped: false,
        dedicatedAbi: false
    )
    GET_JSON = "get_json"

    @ffiValue(value: 17)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: true,
        acceptsSetOptions: true,
        supportsProtected: true,
        supportsRaw: false,
        supportsScoped: false,
        dedicatedAbi: false
    )
    SET_JSON = "set_json"

    @ffiValue(value: 4294967041)
    @ffiOperationContract(
        inputKind: "none",
        acceptsValue: false,
        acceptsSetOptions: false,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: false,
        dedicatedAbi: false
    )
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
