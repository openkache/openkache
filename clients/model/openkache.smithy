$version: "2"

namespace openkache.client

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
    REQUEST_POINTER = "request_pointer"
    U8_POINTER = "u8_pointer"
    STRUCT_POINTER = "struct_pointer"
    SIZE = "size"
    UINT8 = "uint8"
    INT32 = "int32"
    UINT32 = "uint32"
    UINT64 = "uint64"
}

/// Ownership of memory crossing the native ABI boundary.
enum FfiNativeOwnership {
    NONE = "none"
    BORROWED = "borrowed"
    COPIED = "copied"
    OWNED = "owned"
}

/// Lifetime during which a borrowed native pointer remains valid.
enum FfiNativeLifetime {
    CALL = "call"
    REQUEST = "request"
    RESULT = "result"
    CLIENT = "client"
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

    /// Pointer ownership at the ABI boundary. Omitted legacy declarations
    /// default to borrowed for inputs and owned for returned handles.
    ownership: FfiNativeOwnership

    /// Lifetime required for borrowed pointers.
    lifetime: FfiNativeLifetime
}

list FfiNativeParameters {
    member: FfiNativeParameter
}

structure FfiNativeFunction {
    @required
    name: String

    @required
    returnType: FfiNativeType

    /// Ownership transferred by the function's return value.
    returnOwnership: FfiNativeOwnership

    /// Lifetime of a borrowed return value.
    returnLifetime: FfiNativeLifetime

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

    /// Ownership of pointer fields in an ABI structure.
    ownership: FfiNativeOwnership

    /// Lifetime required for borrowed pointer fields.
    lifetime: FfiNativeLifetime
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
    // Maintained formatted-value defaults. Automatic compression uses one
    // level-1 Zstandard frame and keeps it only when the complete frame is
    // smaller; zero means no input-size or minimum-savings threshold.
    maxInFlight: 256,
    connectTimeoutMilliseconds: 5000,
    requestTimeoutMilliseconds: 2000,
    retryMaxAttempts: 2,
    zstandardLevel: 1,
    zstandardMinimumInputBytes: 0,
    zstandardMinimumSavingsBytes: 0,
    serverName: "localhost",
    certificatePemType: "CERTIFICATE",
    minimumPositiveValue: 1,
    zstandardLevelMin: 1,
    zstandardLevelMax: 22
)
@ffiContract(
    abiVersion: 1,
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
            name: "openkache_client_connect_transport",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                {
                    name: "options",
                    type: "struct_pointer",
                    structureName: "FfiConnectOptions",
                    mutable: false
                },
                { name: "transport", type: "uint32", mutable: false }
            ]
        },
        {
            name: "openkache_client_connect_with_keyring_options",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                {
                    name: "options",
                    type: "struct_pointer",
                    structureName: "FfiConnectOptionsWithKeyring",
                    mutable: false
                }
            ]
        },
        {
            name: "openkache_client_execute_async",
            returnType: "request_pointer",
            returnOwnership: "owned",
            returnLifetime: "request",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "keySpec", type: "uint32", mutable: false },
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
            name: "openkache_client_execute_with_options_async",
            returnType: "request_pointer",
            returnOwnership: "owned",
            returnLifetime: "request",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "keySpec", type: "uint32", mutable: false },
                { name: "applicationKey", type: "u8_pointer", mutable: false },
                { name: "applicationKeyLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_raw_async",
            returnType: "request_pointer",
            returnOwnership: "owned",
            returnLifetime: "request",
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
            name: "openkache_client_request_poll",
            returnType: "uint32",
            parameters: [
                {
                    name: "request",
                    type: "request_pointer",
                    mutable: false,
                    ownership: "borrowed",
                    lifetime: "request"
                }
            ]
        },
        {
            name: "openkache_client_request_wait",
            returnType: "result_pointer",
            parameters: [
                {
                    name: "request",
                    type: "request_pointer",
                    mutable: true,
                    ownership: "borrowed",
                    lifetime: "request"
                },
                { name: "timeoutMilliseconds", type: "uint64", mutable: false }
            ]
        },
        {
            name: "openkache_client_request_cancel",
            returnType: "uint32",
            parameters: [
                {
                    name: "request",
                    type: "request_pointer",
                    mutable: false,
                    ownership: "borrowed",
                    lifetime: "request"
                }
            ]
        },
        {
            name: "openkache_client_request_free",
            returnType: "void",
            parameters: [
                {
                    name: "request",
                    type: "request_pointer",
                    mutable: true,
                    ownership: "owned",
                    lifetime: "request"
                }
            ]
        },
        {
            name: "openkache_client_execute_typed",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "keySpec", type: "uint32", mutable: false },
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
            name: "openkache_client_execute_typed_with_options",
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "keySpec", type: "uint32", mutable: false },
                { name: "applicationKey", type: "u8_pointer", mutable: false },
                { name: "applicationKeyLength", type: "size", mutable: false },
                { name: "value", type: "u8_pointer", mutable: false },
                { name: "valueLength", type: "size", mutable: false },
                { name: "setFlags", type: "uint8", mutable: false },
                { name: "ttlMilliseconds", type: "uint64", mutable: false }
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
            name: "openkache_client_execute_unary",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                { name: "body", type: "u8_pointer", mutable: false },
                { name: "bodyLength", type: "size", mutable: false }
            ]
        },
        {
            name: "openkache_client_execute_fields",
            optional: true,
            returnType: "result_pointer",
            parameters: [
                { name: "client", type: "client_pointer", mutable: false },
                { name: "operation", type: "uint32", mutable: false },
                {
                    name: "fields",
                    type: "struct_pointer",
                    structureName: "FfiOperationField",
                    mutable: false
                },
                { name: "fieldCount", type: "size", mutable: false }
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
            name: "openkache_client_result_status",
            returnType: "uint32",
            parameters: [
                { name: "result", type: "result_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_error_category",
            returnType: "uint32",
            parameters: [
                { name: "result", type: "result_pointer", mutable: false }
            ]
        },
        {
            name: "openkache_client_result_data",
            returnType: "u8_pointer",
            returnOwnership: "borrowed",
            returnLifetime: "result",
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
            returnOwnership: "owned",
            returnLifetime: "client",
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
        },
        {
            name: "FfiValueKey",
            fields: [
                { name: "id", type: "uint64", mutable: false },
                { name: "key", type: "u8_pointer", mutable: false },
                { name: "keyLength", type: "size", mutable: false }
            ]
        },
        {
            name: "FfiConnectOptionsWithKeyring",
            fields: [
                { name: "abiVersion", type: "uint32", mutable: false },
                {
                    name: "base",
                    type: "struct_pointer",
                    structureName: "FfiConnectOptions",
                    mutable: false
                },
                { name: "itemIdRootKey", type: "u8_pointer", mutable: false },
                { name: "itemIdRootKeyLength", type: "size", mutable: false },
                {
                    name: "valueKeys",
                    type: "struct_pointer",
                    structureName: "FfiValueKey",
                    mutable: false
                },
                { name: "valueKeyCount", type: "size", mutable: false },
                { name: "activeValueKeyId", type: "uint64", mutable: false },
                { name: "valueEncryption", type: "uint32", mutable: false }
            ]
        },
        {
            name: "FfiOperationField",
            fields: [
                { name: "data", type: "u8_pointer", mutable: false },
                { name: "length", type: "size", mutable: false },
                { name: "present", type: "uint8", mutable: false }
            ]
        }
    ]
)
@valueFormat(
    version: 1,
    // The legacy JSON metadata name is retained for generated compatibility
    // constants, but JSON helpers use OpaqueBytes (selector 0). The target
    // value-format document assigns selector 1 to StructuredValue-CBOR-v1.
    // VU128 is currently used for unsigned 64-bit protocol lengths and
    // versions.  A canonical u64 varuint is at most nine bytes.
    maxVu128Bytes: 9,
    formatByteBytes: 1,
    formatCompressionMask: 15,
    formatEncryptionShift: 4,
    serializationRaw: 0,
    serializationJson: 1,
    serializationStructured: 1,
    compressionNone: 0,
    compressionZstandard: 1,
    encryptionNone: 0,
    encryptionCompact: 2,
    encryptionRobust: 1,
    compactSyntheticIvBytes: 16,
    robustNonceBytes: 12,
    robustTagBytes: 16,
    dataProtectionKeyBytes: 32,
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
}

/// Flattened C-compatible projection used by the native client ABI.
///
/// This shape deliberately mirrors the fields returned by the canonical
/// descriptor decoder. Its member types and names are the source of truth for
/// every generated native binding. Its declaration order and member types
/// determine the natural C-compatible layout.
structure FfiNamespaceDescriptor {
    @required
    @openkache.protocol#unsignedLong
    namespaceId: Long

    @required
    @openkache.protocol#unsignedLong
    revision: Long

    @required
    @openkache.protocol#unsignedLong
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
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    GET_JSON = "get_json"

    @ffiValue(value: 17)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: true,
        acceptsSetOptions: true,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    SET_JSON = "set_json"

    @ffiValue(value: 18)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: false,
        acceptsSetOptions: false,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    GET_STRUCTURED = "get_structured"

    @ffiValue(value: 19)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: true,
        acceptsSetOptions: true,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    SET_STRUCTURED = "set_structured"

    @ffiValue(value: 20)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: false,
        acceptsSetOptions: false,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    GET_V0 = "get_v0"

    @ffiValue(value: 21)
    @ffiOperationContract(
        inputKind: "application_key",
        acceptsValue: true,
        acceptsSetOptions: true,
        supportsProtected: true,
        supportsRaw: true,
        supportsScoped: true,
        dedicatedAbi: false
    )
    SET_V0 = "set_v0"

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

/// Explicit native transport and trust selection. Legacy connection symbols
/// remain verified QUIC for source and ABI compatibility.
enum FfiTransport {
    @ffiValue(value: 0)
    QUIC = "quic"

    @ffiValue(value: 1)
    TLS_TCP = "tls_tcp"

    @ffiValue(value: 2)
    QUIC_INSECURE = "quic_insecure"

    @ffiValue(value: 3)
    TLS_TCP_INSECURE = "tls_tcp_insecure"
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

    /// Generic operation result carrying the declared status and raw payload.
    ///
    /// This discriminator is shape-neutral. API-specific convenience kinds
    /// remain explicit projections instead of being inferred from framing.
    @ffiValue(value: 10)
    RAW = "raw"

    @ffiValue(value: 11)
    CANCELED = "canceled"

    @ffiValue(value: 12)
    UNKNOWN_MUTATION = "unknown_mutation"

    @ffiValue(value: 13)
    RESOURCE_EXHAUSTED = "resource_exhausted"
}

enum FfiSetCondition {
    @ffiValue(value: 0)
    ANY = "any"

    @ffiValue(value: 1)
    IF_ABSENT = "if_absent"

    @ffiValue(value: 2)
    IF_PRESENT = "if_present"
}

enum FfiKeySpec {
    @ffiValue(value: 0)
    TEXT = "text"

    @ffiValue(value: 1)
    BYTES = "bytes"

    @ffiValue(value: 2)
    INTEGER = "integer"
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

/// Wire/value ownership mode; raw and caller-owned APIs preserve exact bytes.
enum FfiValueMode {
    @ffiValue(value: 0)
    FORMATTED_V1 = "formatted_v1"

    @ffiValue(value: 1)
    RAW = "raw"

    @ffiValue(value: 2)
    CALLER_OWNED_V0 = "caller_owned_v0"
}

/// Language-level structured-value projection requested by a binding.
enum FfiValueRepresentation {
    @ffiValue(value: 0)
    LOSSLESS = "lossless"

    @ffiValue(value: 1)
    NATIVE = "native"
}

enum FfiStatusCategory {
    @ffiValue(value: 0)
    SUCCESS = "success"

    @ffiValue(value: 1)
    NOT_FOUND = "not_found"

    @ffiValue(value: 2)
    MUTATION = "mutation"

    @ffiValue(value: 3)
    ERROR = "error"

    @ffiValue(value: 4)
    CANCELED = "canceled"

    @ffiValue(value: 5)
    UNKNOWN_MUTATION = "unknown_mutation"

    @ffiValue(value: 6)
    RESOURCE_EXHAUSTED = "resource_exhausted"
}

enum FfiErrorCategory {
    @ffiValue(value: 0)
    NONE = "none"

    @ffiValue(value: 1)
    INVALID_INPUT = "invalid_input"

    @ffiValue(value: 2)
    CONFIGURATION = "configuration"

    @ffiValue(value: 3)
    TIMEOUT = "timeout"

    @ffiValue(value: 4)
    TRANSPORT = "transport"

    @ffiValue(value: 5)
    SERVER = "server"

    @ffiValue(value: 6)
    PROTOCOL = "protocol"

    @ffiValue(value: 7)
    VALUE = "value"

    @ffiValue(value: 8)
    KEY = "key"

    @ffiValue(value: 9)
    CANCELED = "canceled"

    @ffiValue(value: 10)
    UNKNOWN_MUTATION = "unknown_mutation"

    @ffiValue(value: 11)
    RESOURCE_EXHAUSTED = "resource_exhausted"

    @ffiValue(value: 12)
    CLOSED = "closed"

    @ffiValue(value: 13)
    INTERNAL = "internal"
}

enum FfiRequestState {
    @ffiValue(value: 0)
    PENDING = "pending"

    @ffiValue(value: 1)
    READY = "ready"

    @ffiValue(value: 2)
    CANCELED = "canceled"

    @ffiValue(value: 3)
    CONSUMED = "consumed"

    @ffiValue(value: 4)
    FREED = "freed"
}
