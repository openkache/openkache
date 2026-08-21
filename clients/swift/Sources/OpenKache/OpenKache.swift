import Foundation
#if canImport(Darwin)
import Darwin
#elseif canImport(Glibc)
import Glibc
#endif

// The Rust client owns transport, TLS, retries, key derivation, compression,
// encryption, and value validation.  Swift only supplies native values and
// owns the actor/lifecycle surface.

private typealias NativeClientPointer = OpaquePointer
private typealias NativeResultPointer = OpaquePointer
private typealias NativeConnectTransportFunction = @convention(c) (
    UnsafeRawPointer?,
    UInt32
) -> UnsafeMutableRawPointer?
private typealias NativeRequestPointer = OpaquePointer

private typealias NativeNamespaceDescriptor = Smithy_Native_Namespace_Descriptor

private func optionalNativeConnectTransport() -> NativeConnectTransportFunction? {
    guard let handle = dlopen(nil, RTLD_LAZY) else {
        return nil
    }
    defer { _ = dlclose(handle) }
    guard let symbol = dlsym(handle, "openkache_client_connect_transport") else {
        return nil
    }
    return unsafeBitCast(symbol, to: NativeConnectTransportFunction.self)
}

private let nativeNamespaceDescriptorLayoutIsValid: Void = {
    let layout = MemoryLayout<NativeNamespaceDescriptor>.self
    precondition(
        layout.size == Smithy_Native_Contract.namespaceDescriptorSizeBytes,
        "native namespace descriptor size does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.namespaceId)
            == Smithy_Native_Contract.namespaceDescriptorNamespaceIdOffset,
        "native namespace descriptor namespaceId offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.revision)
            == Smithy_Native_Contract.namespaceDescriptorRevisionOffset,
        "native namespace descriptor revision offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.defaultTtlMs)
            == Smithy_Native_Contract.namespaceDescriptorDefaultTtlMsOffset,
        "native namespace descriptor defaultTtlMs offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.defaultExpiration)
            == Smithy_Native_Contract.namespaceDescriptorDefaultExpirationOffset,
        "native namespace descriptor defaultExpiration offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.expirationOverride)
            == Smithy_Native_Contract.namespaceDescriptorExpirationOverrideOffset,
        "native namespace descriptor expirationOverride offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.defaultEviction)
            == Smithy_Native_Contract.namespaceDescriptorDefaultEvictionOffset,
        "native namespace descriptor defaultEviction offset does not match the Smithy contract"
    )
    precondition(
        layout.offset(of: \NativeNamespaceDescriptor.evictionOverride)
            == Smithy_Native_Contract.namespaceDescriptorEvictionOverrideOffset,
        "native namespace descriptor evictionOverride offset does not match the Smithy contract"
    )
}()

@_silgen_name("openkache_client_abi_version")
private func nativeAbiVersion() -> UInt32

@_silgen_name("openkache_client_connect_ex")
private func nativeConnect(
    _ address: UnsafePointer<UInt8>?,
    _ addressLength: Int,
    _ serverName: UnsafePointer<UInt8>?,
    _ serverNameLength: Int,
    _ certificate: UnsafePointer<UInt8>?,
    _ certificateLength: Int,
    _ clientCertificate: UnsafePointer<UInt8>?,
    _ clientCertificateLength: Int,
    _ clientPrivateKey: UnsafePointer<UInt8>?,
    _ clientPrivateKeyLength: Int,
    _ dataProtectionKey: UnsafePointer<UInt8>?,
    _ dataProtectionKeyLength: Int,
    _ compressionEnabled: UInt8,
    _ compressionLevel: Int32,
    _ minimumInputSize: Int,
    _ minimumSavings: Int,
    _ encryption: UInt32,
    _ retryMaxAttempts: Int,
    _ maxInFlight: Int,
    _ connectTimeoutMilliseconds: UInt64,
    _ requestTimeoutMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute")
private func nativeExecute(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_async")
private func nativeExecuteAsync(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ keySpec: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeRequestPointer?

@_silgen_name("openkache_client_execute_with_options_async")
private func nativeExecuteWithOptionsAsync(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ keySpec: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeRequestPointer?

@_silgen_name("openkache_client_execute_raw_async")
private func nativeExecuteRawAsync(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeRequestPointer?

@_silgen_name("openkache_client_request_poll")
private func nativeRequestPoll(_ request: NativeRequestPointer?) -> UInt32

@_silgen_name("openkache_client_request_wait")
private func nativeRequestWait(
    _ request: NativeRequestPointer?,
    _ timeoutMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_request_cancel")
private func nativeRequestCancel(_ request: NativeRequestPointer?) -> UInt32

@_silgen_name("openkache_client_request_free")
private func nativeRequestFree(_ request: NativeRequestPointer?)

@_silgen_name("openkache_client_execute_raw")
private func nativeExecuteRaw(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_with_options")
private func nativeExecuteWithOptions(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_raw_with_options")
private func nativeExecuteRawWithOptions(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_scoped")
private func nativeExecuteScoped(
    _ client: NativeClientPointer?,
    _ operation: UInt32,
    _ namespaceID: UInt64,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_namespace_open")
private func nativeNamespaceOpen(
    _ client: NativeClientPointer?,
    _ name: UnsafePointer<UInt8>?,
    _ nameLength: Int,
    _ createIfMissing: UInt8,
    _ policyFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_namespace_update_policy")
private func nativeNamespaceUpdatePolicy(
    _ client: NativeClientPointer?,
    _ namespaceID: UInt64,
    _ expectedRevision: UInt64,
    _ policyFlags: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_namespace_delete")
private func nativeNamespaceDelete(
    _ client: NativeClientPointer?,
    _ namespaceID: UInt64,
    _ expectedRevision: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_namespace_descriptor_decode")
private func nativeNamespaceDescriptorDecode(
    _ payload: UnsafePointer<UInt8>?,
    _ payloadLength: Int,
    _ output: UnsafeMutablePointer<NativeNamespaceDescriptor>?
) -> UInt32

@_silgen_name("openkache_client_result_kind")
private func nativeResultKind(_ result: NativeResultPointer?) -> UInt32

@_silgen_name("openkache_client_result_data")
private func nativeResultData(_ result: NativeResultPointer?) -> UnsafePointer<UInt8>?

@_silgen_name("openkache_client_result_data_length")
private func nativeResultDataLength(_ result: NativeResultPointer?) -> Int

@_silgen_name("openkache_client_result_take_client")
private func nativeTakeClient(_ result: NativeResultPointer?) -> NativeClientPointer?

@_silgen_name("openkache_client_result_free")
private func nativeFreeResult(_ result: NativeResultPointer?)

@_silgen_name("openkache_client_free")
private func nativeFreeClient(_ client: NativeClientPointer?)

/// A native client failure returned by the shared Rust core.
public struct OpenKacheError: Error, LocalizedError, Equatable, Sendable {
    /// Human-readable diagnostic supplied by the core.
    public let message: String

    /// Creates an error with a caller-owned diagnostic.
    public init(_ message: String) {
        self.message = message
    }

    public var errorDescription: String? {
        message
    }
}

/// A mutating request crossed the native cancellation/admission boundary.
///
/// The server may have applied the mutation, so callers must not
/// automatically replay the operation.
public struct OpenKacheUnknownMutationError: Error, LocalizedError, Equatable, Sendable {
    /// Human-readable diagnostic supplied by the core.
    public let message: String

    /// Creates an unknown-mutation error with a caller-owned diagnostic.
    public init(_ message: String) {
        self.message = message
    }

    public var errorDescription: String? {
        message
    }
}

/// Compatibility spelling for callers that use the shorter category name.
public typealias UnknownMutationError = OpenKacheUnknownMutationError

/// Optional mutual-TLS identity.
public struct OpenKacheClientIdentity: Sendable {
    /// Leaf certificate and any intermediates as one DER certificate or PEM chain.
    public var certificate: Data
    /// PKCS#1, PKCS#8, or SEC1 private key in DER or PEM form.
    public var privateKey: Data

    /// Creates a client identity. Both buffers must be non-empty.
    public init(certificate: Data, privateKey: Data) {
        self.certificate = certificate
        self.privateKey = privateKey
    }
}

/// Zstandard settings applied before value protection.
public struct OpenKacheCompression: Sendable {
    /// Whether compression is enabled.
    public let enabled: Bool
    /// Zstandard compression level.
    public let level: Int32
    /// Values below this size bypass compression.
    public let minimumInputSize: Int
    /// A compressed value must save at least this many bytes.
    public let minimumSavings: Int

    /// Automatic level-1 Zstandard compression with no size thresholds.
    public static let automatic = OpenKacheCompression(
        enabled: true,
        level: Smithy_Value_Format.defaultZstandardLevel,
        minimumInputSize: Smithy_Value_Format.defaultZstandardMinimumInputBytes,
        minimumSavings: Smithy_Value_Format.defaultZstandardMinimumSavingsBytes
    )

    /// Disables compression.
    public static let disabled = OpenKacheCompression(
        enabled: false,
        level: Smithy_Value_Format.defaultZstandardLevel,
        minimumInputSize: Smithy_Value_Format.defaultZstandardMinimumInputBytes,
        minimumSavings: Smithy_Value_Format.defaultZstandardMinimumSavingsBytes
    )

    /// Creates Zstandard compression settings.
    public static func zstandard(
        level: Int32 = Smithy_Value_Format.defaultZstandardLevel,
        minimumInputSize: Int = Smithy_Value_Format.defaultZstandardMinimumInputBytes,
        minimumSavings: Int = Smithy_Value_Format.defaultZstandardMinimumSavingsBytes
    ) -> OpenKacheCompression {
        OpenKacheCompression(
            enabled: true,
            level: level,
            minimumInputSize: minimumInputSize,
            minimumSavings: minimumSavings
        )
    }

    private init(
        enabled: Bool,
        level: Int32,
        minimumInputSize: Int,
        minimumSavings: Int
    ) {
        self.enabled = enabled
        self.level = level
        self.minimumInputSize = minimumInputSize
        self.minimumSavings = minimumSavings
    }
}

/// Authenticated-encryption profile for protected values.
public enum OpenKacheEncryption: Sendable {
    /// Deterministic AES-256-SIV-CMAC.
    case compact
    /// Randomized AES-256-GCM-SIV (the recommended profile).
    case robust

    fileprivate var nativeValue: UInt32 {
        switch self {
        case .compact:
            return UInt32(Smithy_Value_Format.encryptionCompact)
        case .robust:
            return UInt32(Smithy_Value_Format.encryptionRobust)
        }
    }
}

/// Native transport and server-trust selector.
public enum OpenKacheTransport: Sendable {
    case quic
    case tlsTcp
    case quicInsecure
    case tlsTcpInsecure

    fileprivate var rawValue: UInt32 {
        switch self {
        case .quic:
            return Smithy_Native_Contract.transportQuic
        case .tlsTcp:
            return Smithy_Native_Contract.transportTlsTcp
        case .quicInsecure:
            return Smithy_Native_Contract.transportQuicInsecure
        case .tlsTcpInsecure:
            return Smithy_Native_Contract.transportTlsTcpInsecure
        }
    }
}

/// Connection and value-layer configuration.
public struct OpenKacheClientOptions: Sendable {
    /// Hostname or numeric address followed by the transport port.
    public var address: String
    /// Certificate identity for a numeric address. A hostname uses its own
    /// value as the TLS identity when this is nil.
    public var serverName: String?
    /// One trusted DER certificate or PEM chain. Nil selects system roots.
    public var certificate: Data?
    /// Optional mutual-TLS identity.
    public var identity: OpenKacheClientIdentity?
    /// Optional persistent 32-byte application data-protection key.
    public var dataProtectionKey: Data
    /// Value compression policy.
    public var compression: OpenKacheCompression
    /// Value authenticated-encryption profile.
    public var encryption: OpenKacheEncryption
    /// Maximum connection-establishment duration.
    public var connectTimeout: Duration
    /// Maximum complete request duration.
    public var requestTimeout: Duration
    /// Total attempts for response-safe operations.
    public var retryMaxAttempts: Int
    /// Maximum reusable request lanes on one connection.
    public var maxInFlight: Int
    /// Native transport; verified QUIC is the compatibility default.
    public var transport: OpenKacheTransport

    /// Creates a client configuration with shared-core defaults.
    public init(
        address: String,
        dataProtectionKey: Data = Data(),
        serverName: String? = nil,
        certificate: Data? = nil,
        identity: OpenKacheClientIdentity? = nil,
        compression: OpenKacheCompression = .automatic,
        encryption: OpenKacheEncryption = .robust,
        connectTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultConnectTimeoutMilliseconds)
        ),
        requestTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultRequestTimeoutMilliseconds)
        ),
        retryMaxAttempts: Int = Smithy_Value_Format.defaultRetryMaxAttempts,
        maxInFlight: Int = Smithy_Value_Format.defaultMaxInFlight,
        transport: OpenKacheTransport = .quic
    ) {
        self.address = address
        self.serverName = serverName
        self.certificate = certificate
        self.identity = identity
        self.dataProtectionKey = dataProtectionKey
        self.compression = compression
        self.encryption = encryption
        self.connectTimeout = connectTimeout
        self.requestTimeout = requestTimeout
        self.retryMaxAttempts = retryMaxAttempts
        self.maxInFlight = maxInFlight
        self.transport = transport
    }
}

/// Conditional behavior for one SET operation.
public typealias OpenKacheSetCondition = Smithy_Set_Condition

/// Result of a successful SET operation.
public typealias OpenKacheSetOutcome = Smithy_Set_Outcome

/// Result of a successful DELETE operation.
public enum OpenKacheDeleteOutcome: Sendable, Equatable {
    /// The key existed and was deleted.
    case deleted
    /// The key was absent.
    case notFound
}

/// Best-effort connection state generated from the native Smithy contract.
public typealias OpenKacheConnectionState = Smithy_Connection_State

private final class NativeHandle: @unchecked Sendable {
    let pointer: NativeClientPointer

    init(pointer: NativeClientPointer) {
        self.pointer = pointer
    }

    deinit {
        nativeFreeClient(pointer)
    }
}

/// Owns one asynchronous native request through its complete lifecycle.
///
/// The result returned by `request_wait` is independently owned; this handle
/// only owns the request object and frees it exactly once.
private final class NativeRequestHandle: @unchecked Sendable {
    let pointer: NativeRequestPointer
    private let freeLock = NSLock()
    private var isFreed = false

    init(pointer: NativeRequestPointer) {
        self.pointer = pointer
    }

    deinit {
        free()
    }

    func poll() -> UInt32 {
        nativeRequestPoll(pointer)
    }

    func wait(timeoutMilliseconds: UInt64) -> NativeResultPointer? {
        nativeRequestWait(pointer, timeoutMilliseconds)
    }

    func cancel() {
        _ = nativeRequestCancel(pointer)
    }

    private func free() {
        freeLock.lock()
        defer { freeLock.unlock() }
        guard !isFreed else {
            return
        }
        isFreed = true
        nativeRequestFree(pointer)
    }
}

private enum NativeBridge {
    static func connect(options: OpenKacheClientOptions) throws -> NativeHandle {
        guard nativeAbiVersion() == Smithy_Native_Contract.abiVersion else {
            throw OpenKacheError("unsupported OpenKache native ABI version")
        }
        guard !options.address.isEmpty else {
            throw OpenKacheError("address must not be empty")
        }
        guard options.dataProtectionKey.isEmpty
            || options.dataProtectionKey.count == Smithy_Value_Format.dataProtectionKeyBytes
        else {
            throw OpenKacheError(
                "dataProtectionKey must contain exactly \(Smithy_Value_Format.dataProtectionKeyBytes) bytes when supplied"
            )
        }
        let connectTimeout = try milliseconds(options.connectTimeout, named: "connectTimeout")
        let requestTimeout = try milliseconds(options.requestTimeout, named: "requestTimeout")
        guard options.retryMaxAttempts > 0 else {
            throw OpenKacheError("retryMaxAttempts must be greater than zero")
        }
        guard options.maxInFlight > 0 else {
            throw OpenKacheError("maxInFlight must be greater than zero")
        }
        if options.compression.enabled
            && !(Smithy_Value_Format.defaultZstandardLevelMin...Smithy_Value_Format.defaultZstandardLevelMax)
                .contains(options.compression.level)
        {
            throw OpenKacheError(
                "compression.level must be from "
                    + "\(Smithy_Value_Format.defaultZstandardLevelMin) through "
                    + "\(Smithy_Value_Format.defaultZstandardLevelMax)"
            )
        }
        guard options.compression.minimumInputSize >= 0 else {
            throw OpenKacheError("compression.minimumInputSize must not be negative")
        }
        guard options.compression.minimumSavings >= 0 else {
            throw OpenKacheError("compression.minimumSavings must not be negative")
        }
        if let identity = options.identity {
            guard !identity.certificate.isEmpty else {
                throw OpenKacheError("identity.certificate must not be empty")
            }
            guard !identity.privateKey.isEmpty else {
                throw OpenKacheError("identity.privateKey must not be empty")
            }
        }

        let address = Array(options.address.utf8)
        let serverName = Array((options.serverName ?? "").utf8)
        let certificate = Array(options.certificate ?? Data())
        let clientCertificate = Array(options.identity?.certificate ?? Data())
        let clientPrivateKey = Array(options.identity?.privateKey ?? Data())
        let dataProtectionKey = Array(options.dataProtectionKey)

        let result = try withBytes(address) { addressPointer, addressLength in
            try withBytes(serverName) { serverNamePointer, serverNameLength in
                try withBytes(certificate) { certificatePointer, certificateLength in
                    try withBytes(clientCertificate) { clientCertificatePointer, clientCertificateLength in
                        try withBytes(clientPrivateKey) { clientPrivateKeyPointer, clientPrivateKeyLength in
                            try withBytes(dataProtectionKey) { keyPointer, keyLength in
                                if options.transport == .quic {
                                    return nativeConnect(
                                        addressPointer,
                                        addressLength,
                                        serverNamePointer,
                                        serverNameLength,
                                        certificatePointer,
                                        certificateLength,
                                        clientCertificatePointer,
                                        clientCertificateLength,
                                        clientPrivateKeyPointer,
                                        clientPrivateKeyLength,
                                        keyPointer,
                                        keyLength,
                                        options.compression.enabled ? 1 : 0,
                                        options.compression.level,
                                        options.compression.minimumInputSize,
                                        options.compression.minimumSavings,
                                        options.encryption.nativeValue,
                                        options.retryMaxAttempts,
                                        options.maxInFlight,
                                        connectTimeout,
                                        requestTimeout
                                    )
                                } else {
                                    var nativeOptions = Smithy_Native_Connect_Options(
                                        address: addressPointer,
                                        addressLength: addressLength,
                                        serverName: serverNamePointer,
                                        serverNameLength: serverNameLength,
                                        certificate: certificatePointer,
                                        certificateLength: certificateLength,
                                        clientCertificateChain: clientCertificatePointer,
                                        clientCertificateChainLength: clientCertificateLength,
                                        clientPrivateKey: clientPrivateKeyPointer,
                                        clientPrivateKeyLength: clientPrivateKeyLength,
                                        dataProtectionKey: keyPointer,
                                        dataProtectionKeyLength: keyLength,
                                        compressionEnabled: options.compression.enabled ? 1 : 0,
                                        compressionLevel: options.compression.level,
                                        minimumInputSize: options.compression.minimumInputSize,
                                        minimumSavings: options.compression.minimumSavings,
                                        encryption: options.encryption.nativeValue,
                                        connectTimeoutMilliseconds: connectTimeout,
                                        requestTimeoutMilliseconds: requestTimeout,
                                        retryMaxAttempts: options.retryMaxAttempts,
                                        maxInFlight: options.maxInFlight
                                    )
                                    guard let connectTransport = optionalNativeConnectTransport()
                                    else {
                                        throw OpenKacheError(
                                            "native OpenKache client does not export the optional transport selector"
                                        )
                                    }
                                    return connectTransport(
                                        withUnsafePointer(to: &nativeOptions) {
                                            UnsafeRawPointer($0)
                                        },
                                        options.transport.rawValue
                                    ).map(OpaquePointer.init)
                                }
                            }
                        }
                    }
                }
            }
        }
        guard let result else {
            throw OpenKacheError("native client returned a null connection result")
        }
        defer { nativeFreeResult(result) }
        guard nativeResultKind(result) == Smithy_Native_Contract.resultConnected else {
            throw resultError(result)
        }
        guard let client = nativeTakeClient(result) else {
            throw OpenKacheError("native client returned no client handle")
        }
        return NativeHandle(pointer: client)
    }

    /// Starts a typed logical-key request and drains it through the native
    /// request-handle boundary.
    static func executeTypedAsync(
        _ handle: NativeHandle,
        operation: UInt32,
        keySpec: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil
    ) async throws -> NativeResultPointer {
        let conditionValue = nativeCondition(condition)
        let ttlEnabled: UInt8 = ttl == nil ? 0 : 1
        let ttlMilliseconds = ttl ?? 0
        let request = try withBytes(Array(key)) { keyPointer, keyLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let pointer = nativeExecuteAsync(
                    handle.pointer,
                    operation,
                    keySpec,
                    keyPointer,
                    keyLength,
                    valuePointer,
                    valueLength,
                    conditionValue,
                    ttlEnabled,
                    ttlMilliseconds
                ) else {
                    throw OpenKacheError("native client returned a null request")
                }
                return NativeRequestHandle(pointer: pointer)
            }
        }
        return try await awaitRequest(request)
    }

    /// Starts a typed request with complete SET policy flags.
    static func executeTypedWithOptionsAsync(
        _ handle: NativeHandle,
        operation: UInt32,
        keySpec: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        setFlags: UInt8 = 0,
        ttl: UInt64 = 0
    ) async throws -> NativeResultPointer {
        let request = try withBytes(Array(key)) { keyPointer, keyLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let pointer = nativeExecuteWithOptionsAsync(
                    handle.pointer,
                    operation,
                    keySpec,
                    keyPointer,
                    keyLength,
                    valuePointer,
                    valueLength,
                    setFlags,
                    ttl
                ) else {
                    throw OpenKacheError("native client returned a null request")
                }
                return NativeRequestHandle(pointer: pointer)
            }
        }
        return try await awaitRequest(request)
    }

    /// Starts an exact Item ID request through the native request-handle
    /// boundary.
    static func executeRawAsync(
        _ handle: NativeHandle,
        operation: UInt32,
        itemID: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil
    ) async throws -> NativeResultPointer {
        let conditionValue = nativeCondition(condition)
        let ttlEnabled: UInt8 = ttl == nil ? 0 : 1
        let ttlMilliseconds = ttl ?? 0
        let request = try withBytes(Array(itemID)) { itemIDPointer, itemIDLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let pointer = nativeExecuteRawAsync(
                    handle.pointer,
                    operation,
                    itemIDPointer,
                    itemIDLength,
                    valuePointer,
                    valueLength,
                    conditionValue,
                    ttlEnabled,
                    ttlMilliseconds
                ) else {
                    throw OpenKacheError("native client returned a null raw request")
                }
                return NativeRequestHandle(pointer: pointer)
            }
        }
        return try await awaitRequest(request)
    }

    private static func awaitRequest(
        _ request: NativeRequestHandle
    ) async throws -> NativeResultPointer {
        try await withTaskCancellationHandler(operation: {
            var cancellationPublished = false
            while true {
                let state = request.poll()
                if state != 0 {
                    guard let result = request.wait(timeoutMilliseconds: 0) else {
                        throw OpenKacheError("native request returned no result")
                    }
                    return result
                }

                do {
                    // Polling keeps the actor cooperative while allowing the
                    // cancellation handler to publish native cancellation.
                    try await Task.sleep(nanoseconds: 1_000_000)
                } catch is CancellationError {
                    if !cancellationPublished {
                        request.cancel()
                        cancellationPublished = true
                    }
                }
            }
        }, onCancel: {
            request.cancel()
        })
    }

    static func execute(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil
    ) throws -> NativeResultPointer {
        try executeNative(
            handle,
            operation: operation,
            key: key,
            value: value,
            condition: condition,
            ttl: ttl
        ) { keyPointer, keyLength, valuePointer, valueLength, conditionValue, ttlEnabled, ttlMilliseconds in
            nativeExecute(
                handle.pointer,
                operation,
                keyPointer,
                keyLength,
                valuePointer,
                valueLength,
                conditionValue,
                ttlEnabled,
                ttlMilliseconds
            )
        }
    }

    static func executeRaw(
        _ handle: NativeHandle,
        operation: UInt32,
        itemID: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil
    ) throws -> NativeResultPointer {
        try executeNative(
            handle,
            operation: operation,
            key: itemID,
            value: value,
            condition: condition,
            ttl: ttl
        ) { itemIDPointer, itemIDLength, valuePointer, valueLength, conditionValue, ttlEnabled, ttlMilliseconds in
            nativeExecuteRaw(
                handle.pointer,
                operation,
                itemIDPointer,
                itemIDLength,
                valuePointer,
                valueLength,
                conditionValue,
                ttlEnabled,
                ttlMilliseconds
            )
        }
    }

    static func executeWithOptions(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        setFlags: UInt8 = 0,
        ttl: UInt64 = 0
    ) throws -> NativeResultPointer {
        try withBytes(Array(key)) { keyPointer, keyLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = nativeExecuteWithOptions(
                    handle.pointer,
                    operation,
                    keyPointer,
                    keyLength,
                    valuePointer,
                    valueLength,
                    setFlags,
                    ttl
                ) else {
                    throw OpenKacheError("native client returned a null operation result")
                }
                return result
            }
        }
    }

    static func executeRawWithOptions(
        _ handle: NativeHandle,
        operation: UInt32,
        itemID: Data,
        value: Data = Data(),
        setFlags: UInt8 = 0,
        ttl: UInt64 = 0
    ) throws -> NativeResultPointer {
        try withBytes(Array(itemID)) { itemIDPointer, itemIDLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = nativeExecuteRawWithOptions(
                    handle.pointer,
                    operation,
                    itemIDPointer,
                    itemIDLength,
                    valuePointer,
                    valueLength,
                    setFlags,
                    ttl
                ) else {
                    throw OpenKacheError("native client returned a null raw operation result")
                }
                return result
            }
        }
    }

    static func executeScoped(
        _ handle: NativeHandle,
        operation: UInt32,
        namespaceID: UInt64,
        itemID: Data = Data(),
        value: Data = Data(),
        setFlags: UInt8 = 0,
        ttl: UInt64 = 0
    ) throws -> NativeResultPointer {
        try withBytes(Array(itemID)) { itemIDPointer, itemIDLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = nativeExecuteScoped(
                    handle.pointer,
                    operation,
                    namespaceID,
                    itemIDPointer,
                    itemIDLength,
                    valuePointer,
                    valueLength,
                    setFlags,
                    ttl
                ) else {
                    throw OpenKacheError("native client returned a null scoped operation result")
                }
                return result
            }
        }
    }

    static func namespaceOpen(
        _ handle: NativeHandle,
        name: String,
        createIfMissing: Bool,
        policyFlags: UInt8,
        ttl: UInt64
    ) throws -> NativeResultPointer {
        let bytes = Array(name.utf8)
        guard bytes.count <= Smithy_Value_Format.namespaceNameMaxBytes else {
            throw OpenKacheError(
                "namespace name exceeds \(Smithy_Value_Format.namespaceNameMaxBytes) UTF-8 octets"
            )
        }
        return try withBytes(bytes) { pointer, length in
            guard let result = nativeNamespaceOpen(
                handle.pointer,
                pointer,
                length,
                createIfMissing ? 1 : 0,
                policyFlags,
                ttl
            ) else {
                throw OpenKacheError("native client returned a null namespace-open result")
            }
            return result
        }
    }

    static func namespaceUpdatePolicy(
        _ handle: NativeHandle,
        namespaceID: UInt64,
        expectedRevision: UInt64,
        policyFlags: UInt8,
        ttl: UInt64
    ) throws -> NativeResultPointer {
        guard let result = nativeNamespaceUpdatePolicy(
            handle.pointer,
            namespaceID,
            expectedRevision,
            policyFlags,
            ttl
        ) else {
            throw OpenKacheError("native client returned a null namespace-policy result")
        }
        return result
    }

    static func namespaceDelete(
        _ handle: NativeHandle,
        namespaceID: UInt64,
        expectedRevision: UInt64
    ) throws -> NativeResultPointer {
        guard let result = nativeNamespaceDelete(
            handle.pointer,
            namespaceID,
            expectedRevision
        ) else {
            throw OpenKacheError("native client returned a null namespace-delete result")
        }
        return result
    }

    static func decodeNamespaceDescriptor(
        _ payload: Data
    ) throws -> NativeNamespaceDescriptor {
        _ = nativeNamespaceDescriptorLayoutIsValid
        var decoded = NativeNamespaceDescriptor()
        let status = withBytes(Array(payload)) { pointer, length in
            nativeNamespaceDescriptorDecode(pointer, length, &decoded)
        }
        guard status == Smithy_Native_Contract.namespaceDescriptorDecodeOk else {
            throw OpenKacheError("native ABI returned an invalid namespace descriptor")
        }
        return decoded
    }

    private static func executeNative(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data,
        value: Data,
        condition: OpenKacheSetCondition?,
        ttl: UInt64?,
        call: (
            UnsafePointer<UInt8>?,
            Int,
            UnsafePointer<UInt8>?,
            Int,
            UInt32,
            UInt8,
            UInt64
        ) -> NativeResultPointer?
    ) throws -> NativeResultPointer {
        let conditionValue = nativeCondition(condition)
        let ttlEnabled: UInt8 = ttl == nil ? 0 : 1
        let ttlMilliseconds = ttl ?? 0
        return try withBytes(Array(key)) { keyPointer, keyLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = call(
                    keyPointer,
                    keyLength,
                    valuePointer,
                    valueLength,
                    conditionValue,
                    ttlEnabled,
                    ttlMilliseconds
                ) else {
                    throw OpenKacheError("native client returned a null operation result")
                }
                return result
            }
        }
    }

    private static func nativeCondition(_ condition: OpenKacheSetCondition?) -> UInt32 {
        switch condition {
        case .none:
            return Smithy_Native_Contract.setConditionAny
        case .any:
            return Smithy_Native_Contract.setConditionAny
        case .ifAbsent:
            return Smithy_Native_Contract.setConditionIfAbsent
        case .ifPresent:
            return Smithy_Native_Contract.setConditionIfPresent
        }
    }
}

/// Rust `Duration` conversion shared by all native calls.
private func milliseconds(_ duration: Duration, named name: String) throws -> UInt64 {
    guard duration > .zero else {
        throw OpenKacheError("\(name) must be greater than zero")
    }
    let components = duration.components
    guard components.seconds >= 0, components.attoseconds >= 0 else {
        throw OpenKacheError("\(name) must be finite and non-negative")
    }
    let seconds = UInt64(components.seconds)
    let fractionalMilliseconds = UInt64(components.attoseconds / 1_000_000_000_000_000)
    let (wholeMilliseconds, overflow) = seconds.multipliedReportingOverflow(by: 1_000)
    guard !overflow else {
        throw OpenKacheError("\(name) exceeds the native duration range")
    }
    let (total, additionOverflow) = wholeMilliseconds.addingReportingOverflow(
        fractionalMilliseconds
    )
    guard !additionOverflow, total > 0 else {
        throw OpenKacheError("\(name) must be at least one millisecond")
    }
    return total
}

private func withBytes<T>(
    _ bytes: [UInt8],
    _ body: (UnsafePointer<UInt8>?, Int) throws -> T
) rethrows -> T {
    try bytes.withUnsafeBufferPointer { buffer in
        try body(buffer.baseAddress, buffer.count)
    }
}

private func resultPayload(_ result: NativeResultPointer) throws -> Data {
    let length = nativeResultDataLength(result)
    guard length >= 0 else {
        throw OpenKacheError("native client returned a negative payload length")
    }
    if length == 0 {
        return Data()
    }
    guard let pointer = nativeResultData(result) else {
        throw OpenKacheError("native client returned a null payload pointer")
    }
    return Data(bytes: pointer, count: length)
}

private func resultError(_ result: NativeResultPointer) -> OpenKacheError {
    let payload: Data
    do {
        payload = try resultPayload(result)
    } catch let error as OpenKacheError {
        return error
    } catch {
        return OpenKacheError("native client returned a malformed result payload")
    }
    let message = String(decoding: payload, as: UTF8.self)
    return OpenKacheError(message.isEmpty ? "OpenKache native operation failed" : message)
}

private func consumeResult<T>(
    _ result: NativeResultPointer,
    _ transform: (UInt32, Data) throws -> T
) throws -> T {
    defer { nativeFreeResult(result) }
    let kind = nativeResultKind(result)
    guard kind != Smithy_Native_Contract.resultError else {
        throw resultError(result)
    }
    if kind == Smithy_Native_Contract.resultUnknownMutation {
        let payload = try resultPayload(result)
        let message = String(decoding: payload, as: UTF8.self)
        throw OpenKacheUnknownMutationError(
            message.isEmpty
                ? "OpenKache mutation outcome is unknown after cancellation"
                : message
        )
    }
    if kind == Smithy_Native_Contract.resultCanceled {
        // The native completion has been consumed before exposing Swift's
        // normal cancellation error to the caller.
        throw CancellationError()
    }
    return try transform(kind, resultPayload(result))
}

private func getOutcome(
    _ kind: UInt32,
    payload: Data,
    operation: String
) throws -> Data? {
    switch kind {
    case Smithy_Native_Contract.resultValue:
        return payload
    case Smithy_Native_Contract.resultNotFound:
        return nil
    default:
        throw OpenKacheError("unexpected \(operation) result")
    }
}

private func setOutcome(
    _ kind: UInt32,
    operation: String
) throws -> OpenKacheSetOutcome {
    switch kind {
    case Smithy_Native_Contract.resultCreated:
        return .created
    case Smithy_Native_Contract.resultReplaced:
        return .replaced
    case Smithy_Native_Contract.resultNotStored:
        return .notStored
    default:
        throw OpenKacheError("unexpected \(operation) result")
    }
}

private func deleteOutcome(
    _ kind: UInt32,
    operation: String
) throws -> OpenKacheDeleteOutcome {
    switch kind {
    case Smithy_Native_Contract.resultDeleted:
        return .deleted
    case Smithy_Native_Contract.resultNotDeleted:
        return .notFound
    default:
        throw OpenKacheError("unexpected \(operation) result")
    }
}

/// Actor-isolated Swift client over the shared Rust core.
public actor OpenKacheClient {
    private var native: NativeHandle?

    private init(native: NativeHandle) {
        self.native = native
    }

    /// Connects without blocking the caller's cooperative executor.
    public static func connect(options: OpenKacheClientOptions) async throws -> OpenKacheClient {
        let handle = try await Task.detached(priority: nil) {
            try NativeBridge.connect(options: options)
        }.value
        return OpenKacheClient(native: handle)
    }

    /// Verifies the connection.
    public func ping() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected PING result")
                }
            }
        }
    }

    /// Retrieves protected bytes, or nil when the key does not exist.
    public func get(_ key: Data) async throws -> Data? {
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes,
                key: key
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "GET")
            }
        }
    }

    /// Retrieves protected bytes for a UTF-8 string key.
    public func get(_ key: String) async throws -> Data? {
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                keySpec: Smithy_Native_Contract.keySpecText,
                key: Data(key.utf8)
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "GET")
            }
        }
    }

    /// Stores protected bytes with optional Smithy condition and TTL.
    public func set(
        _ key: Data,
        value: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheSetOutcome {
        let (setFlags, ttl) = try options.wireOptions()
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedWithOptionsAsync(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes,
                key: key,
                value: value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                try setOutcome(kind, operation: "SET")
            }
        }
    }

    /// Stores protected bytes for a UTF-8 string key.
    public func set(
        _ key: String,
        value: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheSetOutcome {
        let (setFlags, ttl) = try options.wireOptions()
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedWithOptionsAsync(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                keySpec: Smithy_Native_Contract.keySpecText,
                key: Data(key.utf8),
                value: value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                try setOutcome(kind, operation: "SET")
            }
        }
    }

    /// Deletes a key and reports whether it existed.
    public func delete(_ key: Data) async throws -> OpenKacheDeleteOutcome {
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes,
                key: key
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "DELETE")
            }
        }
    }

    /// Deletes a UTF-8 string key.
    public func delete(_ key: String) async throws -> OpenKacheDeleteOutcome {
        return try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                keySpec: Smithy_Native_Contract.keySpecText,
                key: Data(key.utf8)
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "DELETE")
            }
        }
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultValue else {
                    throw OpenKacheError("unexpected STATS result")
                }
                guard let text = String(data: payload, encoding: .utf8) else {
                    throw OpenKacheError("STATS response is not valid UTF-8")
                }
                return text
            }
        }
    }

    /// Waits for the server durability barrier.
    public func sync() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected SYNC result")
                }
            }
        }
    }

    /// Replaces a failed connection without replaying an operation.
    public func reconnect() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: Smithy_Native_Contract.operationReconnect,
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected RECONNECT result")
                }
            }
        }
    }

    /// Returns a best-effort connection-state snapshot.
    public func connectionState() async -> OpenKacheConnectionState {
        guard let native else {
            return .closed
        }
        let value = await Task.detached(priority: nil) {
            nativeConnectionState(native.pointer)
        }.value
        return OpenKacheConnectionState(rawValue: value) ?? .unknown
    }

    /// Permanently closes this client. Repeated calls are safe.
    public func close() {
        native = nil
    }

    /// Returns an exact-item-ID client sharing this native connection.
    ///
    /// The returned client bypasses application-key derivation and formatted
    /// value protection. Its operations are the generated Smithy service
    /// operations.
    public func raw() throws -> OpenKacheRawClient {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return OpenKacheRawClient(native: native)
    }

    private func perform<T: Sendable>(
        _ operation: @escaping @Sendable (NativeHandle) throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await Task.detached(priority: nil) {
            try operation(native)
        }.value
    }

    private func performAsync<T: Sendable>(
        _ operation: @escaping @Sendable (NativeHandle) async throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await operation(native)
    }

}

/// Actor-isolated exact-item-ID client implementing the generated Smithy API.
///
/// Raw values are sent to the server exactly as supplied. They are not
/// application-key-derived and do not receive the protected value format.
public actor OpenKacheRawClient {
    private var native: NativeHandle?

    fileprivate init(native: NativeHandle) {
        self.native = native
    }

    /// Connects an exact-item-ID client over the shared native core.
    public static func connect(options: OpenKacheClientOptions) async throws -> OpenKacheRawClient {
        let handle = try await Task.detached(priority: nil) {
            try NativeBridge.connect(options: options)
        }.value
        return OpenKacheRawClient(native: handle)
    }

    /// Verifies the connection.
    public func ping() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected raw PING result")
                }
            }
        }
    }

    /// Retrieves exact bytes for a `0...32`-byte protocol item ID.
    public func get(_ itemID: Data) async throws -> Data? {
        try validateItemID(itemID)
        return try await performAsync { handle in
            let result = try await NativeBridge.executeRawAsync(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                itemID: itemID
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "raw GET")
            }
        }
    }

    /// Stores exact bytes for a `0...32`-byte protocol item ID.
    public func set(
        _ itemID: Data,
        value: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheSetOutcome {
        try validateItemID(itemID)
        let (setFlags, ttl) = try options.wireOptions()
        if let legacy = try options.legacyRequestOptions() {
            return try await performAsync { handle in
                let result = try await NativeBridge.executeRawAsync(
                    handle,
                    operation: UInt32(Smithy_Opcode.set.rawValue),
                    itemID: itemID,
                    value: value,
                    condition: legacy.condition,
                    ttl: legacy.ttl
                )
                return try consumeResult(result) { kind, _ in
                    try setOutcome(kind, operation: "raw SET")
                }
            }
        }
        // ABI v6 has no raw request handle for complete SET policy flags.
        // A detached synchronous call is the documented safe completion boundary
        // for this operation shape.
        return try await perform { handle in
            let result = try NativeBridge.executeRawWithOptions(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                itemID: itemID,
                value: value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                try setOutcome(kind, operation: "raw SET")
            }
        }
    }

    /// Deletes a `0...32`-byte protocol item ID.
    public func delete(_ itemID: Data) async throws -> OpenKacheDeleteOutcome {
        try validateItemID(itemID)
        return try await performAsync { handle in
            let result = try await NativeBridge.executeRawAsync(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                itemID: itemID
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "raw DELETE")
            }
        }
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultValue else {
                    throw OpenKacheError("unexpected raw STATS result")
                }
                guard let text = String(data: payload, encoding: .utf8) else {
                    throw OpenKacheError("raw STATS response is not valid UTF-8")
                }
                return text
            }
        }
    }

    /// Waits for the server durability barrier.
    public func sync() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue),
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected raw SYNC result")
                }
            }
        }
    }

    /// Replaces a failed connection without replaying an operation.
    public func reconnect() async throws {
        try await performAsync { handle in
            let result = try await NativeBridge.executeTypedAsync(
                handle,
                operation: Smithy_Native_Contract.operationReconnect,
                keySpec: Smithy_Native_Contract.keySpecBytes
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected raw RECONNECT result")
                }
            }
        }
    }

    /// Returns a best-effort connection-state snapshot.
    public func connectionState() async -> OpenKacheConnectionState {
        guard let native else {
            return .closed
        }
        let value = await Task.detached(priority: nil) {
            nativeConnectionState(native.pointer)
        }.value
        return OpenKacheConnectionState(rawValue: value) ?? .unknown
    }

    /// Permanently closes this client. Repeated calls are safe.
    public func close() {
        native = nil
    }

    private func perform<T: Sendable>(
        _ operation: @escaping @Sendable (NativeHandle) throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await Task.detached(priority: nil) {
            try operation(native)
        }.value
    }

    private func performAsync<T: Sendable>(
        _ operation: @escaping @Sendable (NativeHandle) async throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await operation(native)
    }

    private func validateItemID(_ itemID: Data) throws {
        guard itemID.count <= Smithy_Value_Format.itemIdBytes else {
            throw OpenKacheError(
                "itemID must contain at most \(Smithy_Value_Format.itemIdBytes) bytes"
            )
        }
    }
}

extension OpenKacheRawClient: Smithy_OpenKache_Api {
    public func ping(_ input: Smithy_Ping_Input) async throws -> Smithy_Ping_Output {
        _ = input
        try await ping()
        return Smithy_Ping_Output(payload: Data())
    }

    public func get(_ input: Smithy_Get_Input) async throws -> Smithy_Get_Output {
        try validateItemID(input.itemId)
        let value = try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                namespaceID: input.namespaceId,
                itemID: input.itemId
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "GET")
            }
        }
        return Smithy_Get_Output(value: value)
    }

    public func set(_ input: Smithy_Set_Input) async throws -> Smithy_Set_Output {
        try validateItemID(input.itemId)
        let (setFlags, ttl) = try smithySetFlags(input)
        let outcome = try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                namespaceID: input.namespaceId,
                itemID: input.itemId,
                value: input.value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                try setOutcome(kind, operation: "SET")
            }
        }
        return Smithy_Set_Output(outcome: outcome)
    }

    public func delete(_ input: Smithy_Delete_Input) async throws -> Smithy_Delete_Output {
        try validateItemID(input.itemId)
        let deleted = try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                namespaceID: input.namespaceId,
                itemID: input.itemId
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "DELETE") == .deleted
            }
        }
        return Smithy_Delete_Output(deleted: deleted)
    }

    public func stats(_ input: Smithy_Stats_Input) async throws -> Smithy_Stats_Output {
        let json = try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue),
                namespaceID: input.namespaceId
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultValue else {
                    throw OpenKacheError("unexpected STATS result")
                }
                guard let text = String(data: payload, encoding: .utf8) else {
                    throw OpenKacheError("STATS response is not valid UTF-8")
                }
                return text
            }
        }
        return Smithy_Stats_Output(json: json)
    }

    public func sync(_ input: Smithy_Sync_Input) async throws -> Smithy_Sync_Output {
        try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue),
                namespaceID: input.namespaceId
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected SYNC result")
                }
            }
        }
        return Smithy_Sync_Output()
    }

    public func namespaceOpen(
        _ input: Smithy_Namespace_Open_Input
    ) async throws -> Smithy_Namespace_Open_Output {
        if input.createIfMissing && input.policy == nil {
            throw OpenKacheError("namespace policy is required when createIfMissing is true")
        }
        if !input.createIfMissing && input.policy != nil {
            throw OpenKacheError("namespace policy is only valid when createIfMissing is true")
        }
        let (flags, ttl) = try smithyPolicyFlags(input.policy)
        return try await perform { handle in
            let result = try NativeBridge.namespaceOpen(
                handle,
                name: input.name,
                createIfMissing: input.createIfMissing,
                policyFlags: flags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultOk
                    || kind == Smithy_Native_Contract.resultCreated
                else {
                    throw OpenKacheError("unexpected NAMESPACE_OPEN result")
                }
                return Smithy_Namespace_Open_Output(
                    descriptor: smithyNamespaceDescriptor(
                        try NativeBridge.decodeNamespaceDescriptor(payload)
                    ),
                    created: kind == Smithy_Native_Contract.resultCreated
                )
            }
        }
    }

    public func namespaceUpdatePolicy(
        _ input: Smithy_Namespace_Update_Policy_Input
    ) async throws -> Smithy_Namespace_Update_Policy_Output {
        let (flags, ttl) = try smithyPolicyFlags(input.policy)
        return try await perform { handle in
            let result = try NativeBridge.namespaceUpdatePolicy(
                handle,
                namespaceID: input.namespaceId,
                expectedRevision: input.expectedRevision,
                policyFlags: flags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultValue else {
                    throw OpenKacheError("unexpected NAMESPACE_UPDATE_POLICY result")
                }
                return Smithy_Namespace_Update_Policy_Output(
                    descriptor: smithyNamespaceDescriptor(
                        try NativeBridge.decodeNamespaceDescriptor(payload)
                    )
                )
            }
        }
    }

    public func namespaceDelete(
        _ input: Smithy_Namespace_Delete_Input
    ) async throws -> Smithy_Namespace_Delete_Output {
        try await perform { handle in
            let result = try NativeBridge.namespaceDelete(
                handle,
                namespaceID: input.namespaceId,
                expectedRevision: input.expectedRevision
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected NAMESPACE_DELETE result")
                }
            }
        }
        return Smithy_Namespace_Delete_Output()
    }
}

private func smithySetFlags(
    _ input: Smithy_Set_Input
) throws -> (flags: UInt8, ttl: UInt64) {
    var flags: UInt8
    switch input.condition {
    case nil, .any:
        flags = Smithy_Value_Format.setConditionAnyBits
    case .ifAbsent:
        flags = Smithy_Value_Format.setIfAbsentBits
    case .ifPresent:
        flags = Smithy_Value_Format.setIfPresentBits
    }
    switch input.expirationMode {
    case nil, .inherit:
        guard input.ttlMilliseconds == nil else {
            throw OpenKacheError("ttlMilliseconds requires explicitTtl expiration mode")
        }
        flags |= Smithy_Value_Format.setInheritExpirationBits
    case .noExpiry:
        guard input.ttlMilliseconds == nil else {
            throw OpenKacheError("ttlMilliseconds is not valid with noExpiry")
        }
        flags |= Smithy_Value_Format.setNoExpiryBits
    case .explicitTtl:
        guard let ttl = input.ttlMilliseconds, ttl > 0 else {
            throw OpenKacheError("ttlMilliseconds must be positive with explicitTtl")
        }
        flags |= Smithy_Value_Format.setExplicitTtlBits
    }
    switch input.evictionMode {
    case nil, .inherit:
        flags |= Smithy_Value_Format.setInheritEvictionBits
    case .evictable:
        flags |= Smithy_Value_Format.setEvictableBits
    case .evictionProtected:
        flags |= Smithy_Value_Format.setEvictionProtectedBits
    }
    return (flags, input.ttlMilliseconds ?? 0)
}

private func smithyPolicyFlags(
    _ policy: Smithy_Namespace_Policy?
) throws -> (flags: UInt8, ttl: UInt64) {
    guard let policy else {
        return (0, 0)
    }
    var flags: UInt8
    switch policy.defaultExpiration {
    case .noExpiry:
        guard policy.defaultTtlMilliseconds == nil else {
            throw OpenKacheError("defaultTtlMilliseconds is invalid with noExpiry")
        }
        flags = Smithy_Value_Format.policyNoExpiry
    case .fixedTtl:
        guard let ttl = policy.defaultTtlMilliseconds, ttl > 0 else {
            throw OpenKacheError("defaultTtlMilliseconds must be positive with fixedTtl")
        }
        flags = Smithy_Value_Format.policyFixedTtl
    }
    switch policy.expirationOverride {
    case .allowed:
        flags |= Smithy_Value_Format.policyExpirationOverride
    case .disallowed:
        break
    }
    switch policy.defaultEviction {
    case .evictable:
        break
    case .evictionProtected:
        flags |= Smithy_Value_Format.policyEvictionProtected
    }
    switch policy.evictionOverride {
    case .allowed:
        flags |= Smithy_Value_Format.policyEvictionOverride
    case .disallowed:
        break
    }
    return (flags, policy.defaultTtlMilliseconds ?? 0)
}

private func smithyNamespaceDescriptor(
    _ decoded: NativeNamespaceDescriptor
) -> Smithy_Namespace_Descriptor {
    return Smithy_Namespace_Descriptor(
        namespaceId: decoded.namespaceId,
        revision: decoded.revision,
        policy: Smithy_Namespace_Policy(
            defaultExpiration: decoded.defaultExpiration
                == Smithy_Native_Contract.namespaceDefaultExpirationFixedTtl
                ? .fixedTtl
                : .noExpiry,
            defaultTtlMilliseconds: decoded.defaultExpiration
                == Smithy_Native_Contract.namespaceDefaultExpirationFixedTtl
                ? decoded.defaultTtlMs
                : nil,
            expirationOverride: decoded.expirationOverride
                == Smithy_Native_Contract.namespaceOverrideAllowed
                ? .allowed
                : .disallowed,
            defaultEviction: decoded.defaultEviction
                == Smithy_Native_Contract.namespaceDefaultEvictionProtected
                ? .evictionProtected
                : .evictable,
            evictionOverride: decoded.evictionOverride
                == Smithy_Native_Contract.namespaceOverrideAllowed
                ? .allowed
                : .disallowed
        )
    )
}

/// Optional SET condition and expiration.
public struct OpenKacheSetOptions: Sendable {
    /// Conditional write behavior, or nil for unconditional SET.
    public var condition: OpenKacheSetCondition?
    /// Item expiration selection. Nil inherits the namespace policy.
    public var expirationMode: Smithy_Expiration_Mode?
    /// Item capacity-eviction selection. Nil inherits the namespace policy.
    public var evictionMode: Smithy_Eviction_Mode?
    /// Positive relative lifetime.
    public var expiresAfter: Duration?

    /// Creates unconditional persistent options.
    public init(
        condition: OpenKacheSetCondition? = nil,
        expirationMode: Smithy_Expiration_Mode? = nil,
        evictionMode: Smithy_Eviction_Mode? = nil,
        expiresAfter: Duration? = nil
    ) {
        self.condition = condition
        self.expirationMode = expirationMode
        self.evictionMode = evictionMode
        self.expiresAfter = expiresAfter
    }

    fileprivate func ttlMilliseconds() throws -> UInt64? {
        guard let expiresAfter else {
            return nil
        }
        return try milliseconds(expiresAfter, named: "expiresAfter")
    }

    fileprivate func wireOptions() throws -> (Flags: UInt8, Ttl: UInt64) {
        var flags: UInt8
        switch condition {
        case nil, .any:
            flags = Smithy_Value_Format.setConditionAnyBits
        case .ifAbsent:
            flags = Smithy_Value_Format.setIfAbsentBits
        case .ifPresent:
            flags = Smithy_Value_Format.setIfPresentBits
        }
        let ttl = try ttlMilliseconds()
        if let expirationMode {
            switch expirationMode {
            case .inherit where ttl == nil:
                flags |= Smithy_Value_Format.setInheritExpirationBits
            case .noExpiry where ttl == nil:
                flags |= Smithy_Value_Format.setNoExpiryBits
            case .explicitTtl where ttl != nil:
                flags |= Smithy_Value_Format.setExplicitTtlBits
            case .inherit, .noExpiry:
                throw OpenKacheError("expiresAfter is only valid with explicitTtl expiration mode")
            case .explicitTtl:
                throw OpenKacheError("expiresAfter must be positive with explicitTtl expiration mode")
            }
        } else if ttl != nil {
            flags |= Smithy_Value_Format.setExplicitTtlBits
        } else {
            flags |= Smithy_Value_Format.setInheritExpirationBits
        }
        switch evictionMode {
        case nil, .inherit:
            flags |= Smithy_Value_Format.setInheritEvictionBits
        case .evictable:
            flags |= Smithy_Value_Format.setEvictableBits
        case .evictionProtected:
            flags |= Smithy_Value_Format.setEvictionProtectedBits
        }
        return (flags, ttl ?? 0)
    }

    /// Returns the legacy condition/TTL projection when complete policy flags
    /// are not needed by a raw SET request.
    fileprivate func legacyRequestOptions() throws -> (
        condition: OpenKacheSetCondition,
        ttl: UInt64?
    )? {
        switch evictionMode {
        case nil, .inherit:
            break
        case .evictable, .evictionProtected:
            return nil
        }
        let condition = self.condition ?? .any
        switch expirationMode {
        case nil:
            return (condition, try ttlMilliseconds())
        case .inherit:
            guard expiresAfter == nil else {
                return nil
            }
            return (condition, nil)
        case .explicitTtl:
            guard let ttl = try ttlMilliseconds() else {
                return nil
            }
            return (condition, ttl)
        case .noExpiry:
            return nil
        }
    }
}
