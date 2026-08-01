import Foundation

// The Rust client owns transport, TLS, retries, key derivation, compression,
// encryption, and value validation.  Swift only supplies native values and
// owns the actor/lifecycle surface.

private typealias NativeClientPointer = OpaquePointer
private typealias NativeResultPointer = OpaquePointer
private let nativeMutationIDBytes = Smithy_Value_Format.mutationIdBytes
private let nativeConnectOptionsBytes = 184
private let nativeMetricsSnapshotBytes = 88

@_silgen_name("openkache_client_abi_version")
private func nativeAbiVersion() -> UInt32

@_silgen_name("openkache_client_connect_with_options")
private func nativeConnectWithOptions(
    _ options: UnsafeRawPointer?
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_with_request_id")
private func nativeExecuteWithRequestID(
    _ client: NativeClientPointer?,
    _ requestID: UInt64,
    _ operation: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_with_request_id_and_mutation_id")
private func nativeExecuteWithRequestIDAndMutationID(
    _ client: NativeClientPointer?,
    _ requestID: UInt64,
    _ operation: UInt32,
    _ applicationKey: UnsafePointer<UInt8>?,
    _ applicationKeyLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64,
    _ mutationID: UnsafePointer<UInt8>?,
    _ mutationIDLength: Int
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_raw_with_request_id")
private func nativeExecuteRawWithRequestID(
    _ client: NativeClientPointer?,
    _ requestID: UInt64,
    _ operation: UInt32,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64
) -> NativeResultPointer?

@_silgen_name("openkache_client_execute_raw_with_request_id_and_mutation_id")
private func nativeExecuteRawWithRequestIDAndMutationID(
    _ client: NativeClientPointer?,
    _ requestID: UInt64,
    _ operation: UInt32,
    _ itemID: UnsafePointer<UInt8>?,
    _ itemIDLength: Int,
    _ value: UnsafePointer<UInt8>?,
    _ valueLength: Int,
    _ setCondition: UInt32,
    _ ttlEnabled: UInt8,
    _ ttlMilliseconds: UInt64,
    _ mutationID: UnsafePointer<UInt8>?,
    _ mutationIDLength: Int
) -> NativeResultPointer?

@_silgen_name("openkache_client_result_kind")
private func nativeResultKind(_ result: NativeResultPointer?) -> UInt32

@_silgen_name("openkache_client_result_data")
private func nativeResultData(_ result: NativeResultPointer?) -> UnsafePointer<UInt8>?

@_silgen_name("openkache_client_result_data_length")
private func nativeResultDataLength(_ result: NativeResultPointer?) -> Int

@_silgen_name("openkache_client_cancel")
private func nativeCancel(_ client: NativeClientPointer?, _ requestID: UInt64) -> UInt8

@_silgen_name("openkache_client_metrics_snapshot")
private func nativeMetricsSnapshot(
    _ client: NativeClientPointer?,
    _ snapshot: UnsafeMutableRawPointer?
) -> UInt8

@_silgen_name("openkache_client_result_error_metadata")
private func nativeResultErrorMetadata(
    _ result: NativeResultPointer?,
    _ metadata: UnsafeMutableRawPointer?
) -> UInt8

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
    /// Structured metadata supplied by the native core, when available.
    public let metadata: ErrorMetadata?

    /// Creates an error with a caller-owned diagnostic.
    public init(_ message: String, metadata: ErrorMetadata? = nil) {
        self.message = message
        self.metadata = metadata
    }

    public var errorDescription: String? {
        message
    }
}

/// Stable structured metadata attached to a native operation failure.
public struct ErrorMetadata: Equatable, Sendable {
    public let code: UInt32
    public let operation: UInt32
    public let phase: UInt32
    public let backend: UInt32
    public let retryable: Bool
    public let ambiguous: Bool
    public let mutationID: Data?
}

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

/// Active data-protection key and a bounded read/delete rotation window.
public struct OpenKacheDataProtectionKeyRing: Sendable {
    /// Key used for new writes.
    public var active: Data
    /// Retired keys tried newest-first for reads and deletes.
    public var previous: [Data]

    /// Creates a key ring within the Smithy-defined retired-key window.
    public init(active: Data, previous: [Data] = []) {
        self.active = active
        self.previous = previous
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

/// Connection and value-layer configuration.
public struct OpenKacheClientOptions: Sendable {
    /// Hostname or numeric address followed by the UDP port.
    public var address: String
    /// Certificate identity for a numeric address. A hostname uses its own
    /// value as the TLS identity when this is nil.
    public var serverName: String?
    /// One trusted DER certificate or PEM chain. Nil selects system roots.
    public var certificate: Data?
    /// Optional mutual-TLS identity.
    public var identity: OpenKacheClientIdentity?
    /// Persistent 32-byte application data-protection key.
    ///
    /// Set this when `keyRing` is nil. A key ring may be supplied instead.
    public var dataProtectionKey: Data?
    /// Optional active key plus retired keys for key rotation.
    public var keyRing: OpenKacheDataProtectionKeyRing?
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

    /// Creates a client configuration with shared-core defaults.
    public init(
        address: String,
        dataProtectionKey: Data,
        serverName: String? = nil,
        certificate: Data? = nil,
        identity: OpenKacheClientIdentity? = nil,
        compression: OpenKacheCompression = .disabled,
        encryption: OpenKacheEncryption = .robust,
        connectTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultConnectTimeoutMilliseconds)
        ),
        requestTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultRequestTimeoutMilliseconds)
        ),
        retryMaxAttempts: Int = Smithy_Value_Format.defaultRetryMaxAttempts,
        maxInFlight: Int = Smithy_Value_Format.defaultMaxInFlight,
        keyRing: OpenKacheDataProtectionKeyRing? = nil
    ) {
        self.address = address
        self.serverName = serverName
        self.certificate = certificate
        self.identity = identity
        self.dataProtectionKey = dataProtectionKey
        self.keyRing = keyRing
        self.compression = compression
        self.encryption = encryption
        self.connectTimeout = connectTimeout
        self.requestTimeout = requestTimeout
        self.retryMaxAttempts = retryMaxAttempts
        self.maxInFlight = maxInFlight
    }

    /// Creates options using an active key and optional retired rotation keys.
    public init(
        address: String,
        keyRing: OpenKacheDataProtectionKeyRing,
        serverName: String? = nil,
        certificate: Data? = nil,
        identity: OpenKacheClientIdentity? = nil,
        compression: OpenKacheCompression = .disabled,
        encryption: OpenKacheEncryption = .robust,
        connectTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultConnectTimeoutMilliseconds)
        ),
        requestTimeout: Duration = .milliseconds(
            Int64(Smithy_Value_Format.defaultRequestTimeoutMilliseconds)
        ),
        retryMaxAttempts: Int = Smithy_Value_Format.defaultRetryMaxAttempts,
        maxInFlight: Int = Smithy_Value_Format.defaultMaxInFlight
    ) {
        self.address = address
        self.serverName = serverName
        self.certificate = certificate
        self.identity = identity
        self.dataProtectionKey = nil
        self.keyRing = keyRing
        self.compression = compression
        self.encryption = encryption
        self.connectTimeout = connectTimeout
        self.requestTimeout = requestTimeout
        self.retryMaxAttempts = retryMaxAttempts
        self.maxInFlight = maxInFlight
    }
}

/// Point-in-time counters collected by one native client connection.
public struct OpenKacheMetricsSnapshot: Sendable, Equatable {
    public let requests: UInt64
    public let hits: UInt64
    public let misses: UInt64
    public let retries: UInt64
    public let reconnects: UInt64
    public let cancellations: UInt64
    public let transportErrors: UInt64
    public let protocolErrors: UInt64
    public let bytesSent: UInt64
    public let bytesReceived: UInt64
    public let activeLanes: UInt64
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
    private let lock = NSLock()
    private var nextRequestID: UInt64 = 1

    init(pointer: NativeClientPointer) {
        self.pointer = pointer
    }

    func reserveRequestID() -> UInt64 {
        lock.lock()
        defer { lock.unlock() }
        let requestID = nextRequestID
        nextRequestID = nextRequestID == UInt64.max ? 1 : nextRequestID + 1
        return requestID == 0 ? reserveRequestID() : requestID
    }

    func cancel(requestID: UInt64) {
        _ = nativeCancel(pointer, requestID)
    }

    func metricsSnapshot() throws -> OpenKacheMetricsSnapshot {
        let pointer = UnsafeMutableRawPointer.allocate(
            byteCount: nativeMetricsSnapshotBytes,
            alignment: MemoryLayout<UInt64>.alignment
        )
        defer { pointer.deallocate() }
        guard nativeMetricsSnapshot(self.pointer, pointer) != 0 else {
            throw OpenKacheError("native client did not return metrics")
        }
        return OpenKacheMetricsSnapshot(
            requests: pointer.load(fromByteOffset: 0, as: UInt64.self),
            hits: pointer.load(fromByteOffset: 8, as: UInt64.self),
            misses: pointer.load(fromByteOffset: 16, as: UInt64.self),
            retries: pointer.load(fromByteOffset: 24, as: UInt64.self),
            reconnects: pointer.load(fromByteOffset: 32, as: UInt64.self),
            cancellations: pointer.load(fromByteOffset: 40, as: UInt64.self),
            transportErrors: pointer.load(fromByteOffset: 48, as: UInt64.self),
            protocolErrors: pointer.load(fromByteOffset: 56, as: UInt64.self),
            bytesSent: pointer.load(fromByteOffset: 64, as: UInt64.self),
            bytesReceived: pointer.load(fromByteOffset: 72, as: UInt64.self),
            activeLanes: pointer.load(fromByteOffset: 80, as: UInt64.self)
        )
    }

    deinit {
        nativeFreeClient(pointer)
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
        let activeKey: Data
        let previousKeys: [Data]
        if let keyRing = options.keyRing {
            guard options.dataProtectionKey == nil else {
                throw OpenKacheError("provide either dataProtectionKey or keyRing, not both")
            }
            activeKey = keyRing.active
            previousKeys = keyRing.previous
        } else if let dataProtectionKey = options.dataProtectionKey {
            activeKey = dataProtectionKey
            previousKeys = []
        } else {
            throw OpenKacheError("dataProtectionKey or keyRing must be supplied")
        }
        guard activeKey.count == Smithy_Value_Format.dataProtectionKeyBytes else {
            throw OpenKacheError(
                "dataProtectionKey must contain exactly \(Smithy_Value_Format.dataProtectionKeyBytes) bytes"
            )
        }
        guard previousKeys.count <= Smithy_Value_Format.maxPreviousDataProtectionKeys else {
            throw OpenKacheError(
                "keyRing.previous may contain at most "
                    + "\(Smithy_Value_Format.maxPreviousDataProtectionKeys) keys"
            )
        }
        for (index, key) in previousKeys.enumerated() {
            guard key.count == Smithy_Value_Format.dataProtectionKeyBytes else {
                throw OpenKacheError(
                    "keyRing.previous[\(index)] must contain exactly "
                        + "\(Smithy_Value_Format.dataProtectionKeyBytes) bytes"
                )
            }
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
        let dataProtectionKey = Array(activeKey)
        let previousDataProtectionKeys = previousKeys.flatMap(Array.init)
        let result = withBytes(address) { addressPointer, addressLength in
            withBytes(serverName) { serverNamePointer, serverNameLength in
                withBytes(certificate) { certificatePointer, certificateLength in
                    withBytes(clientCertificate) { clientCertificatePointer, clientCertificateLength in
                        withBytes(clientPrivateKey) { clientPrivateKeyPointer, clientPrivateKeyLength in
                            withBytes(dataProtectionKey) { keyPointer, keyLength in
                                withBytes(previousDataProtectionKeys) {
                                    previousKeysPointer,
                                    previousKeysLength
                                in
                                    connectWithOptions(
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
                                        previousKeysPointer,
                                        previousKeysLength,
                                        previousKeys.count,
                                        options,
                                        connectTimeout,
                                        requestTimeout
                                    )
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

    private static func connectWithOptions(
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
        _ previousKeys: UnsafePointer<UInt8>?,
        _ previousKeysLength: Int,
        _ previousKeyCount: Int,
        _ options: OpenKacheClientOptions,
        _ connectTimeout: UInt64,
        _ requestTimeout: UInt64
    ) -> NativeResultPointer? {
        var bytes = [UInt8](repeating: 0, count: nativeConnectOptionsBytes)
        return bytes.withUnsafeMutableBytes { buffer in
            func storePointer(_ value: UnsafePointer<UInt8>?, at offset: Int) {
                buffer.storeBytes(
                    of: value.map { UInt(bitPattern: $0) } ?? 0,
                    toByteOffset: offset,
                    as: UInt.self
                )
            }

            func storeSize(_ value: Int, at offset: Int) {
                buffer.storeBytes(
                    of: UInt(value),
                    toByteOffset: offset,
                    as: UInt.self
                )
            }

            storePointer(address, at: 0)
            storeSize(addressLength, at: 8)
            storePointer(serverName, at: 16)
            storeSize(serverNameLength, at: 24)
            storePointer(certificate, at: 32)
            storeSize(certificateLength, at: 40)
            storePointer(clientCertificate, at: 48)
            storeSize(clientCertificateLength, at: 56)
            storePointer(clientPrivateKey, at: 64)
            storeSize(clientPrivateKeyLength, at: 72)
            storePointer(dataProtectionKey, at: 80)
            storeSize(dataProtectionKeyLength, at: 88)
            storePointer(previousKeys, at: 96)
            storeSize(previousKeysLength, at: 104)
            storeSize(previousKeyCount, at: 112)
            buffer.storeBytes(
                of: options.compression.enabled ? UInt8(1) : UInt8(0),
                toByteOffset: 120,
                as: UInt8.self
            )
            buffer.storeBytes(
                of: options.compression.level,
                toByteOffset: 124,
                as: Int32.self
            )
            storeSize(options.compression.minimumInputSize, at: 128)
            storeSize(options.compression.minimumSavings, at: 136)
            buffer.storeBytes(
                of: options.encryption.nativeValue,
                toByteOffset: 144,
                as: UInt32.self
            )
            buffer.storeBytes(of: connectTimeout, toByteOffset: 152, as: UInt64.self)
            buffer.storeBytes(of: requestTimeout, toByteOffset: 160, as: UInt64.self)
            storeSize(options.retryMaxAttempts, at: 168)
            storeSize(options.maxInFlight, at: 176)
            return nativeConnectWithOptions(buffer.baseAddress)
        }
    }

    static func execute(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil,
        mutationID: Data? = nil,
        requestID: UInt64? = nil
    ) throws -> NativeResultPointer {
        try executeNative(
            handle,
            operation: operation,
            key: key,
            value: value,
            condition: condition,
            ttl: ttl,
            mutationID: mutationID
        ) { keyPointer, keyLength, valuePointer, valueLength, conditionValue, ttlEnabled, ttlMilliseconds in
            if let mutationID {
                return withBytes(Array(mutationID)) { mutationPointer, mutationLength in
                    nativeExecuteWithRequestIDAndMutationID(
                        handle.pointer,
                        requestID ?? handle.reserveRequestID(),
                        operation,
                        keyPointer,
                        keyLength,
                        valuePointer,
                        valueLength,
                        conditionValue,
                        ttlEnabled,
                        ttlMilliseconds,
                        mutationPointer,
                        mutationLength
                    )
                }
            }
            return nativeExecuteWithRequestID(
                handle.pointer,
                requestID ?? handle.reserveRequestID(),
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
        ttl: UInt64? = nil,
        mutationID: Data? = nil,
        requestID: UInt64? = nil
    ) throws -> NativeResultPointer {
        try executeNative(
            handle,
            operation: operation,
            key: itemID,
            value: value,
            condition: condition,
            ttl: ttl,
            mutationID: mutationID
        ) { itemIDPointer, itemIDLength, valuePointer, valueLength, conditionValue, ttlEnabled, ttlMilliseconds in
            if let mutationID {
                return withBytes(Array(mutationID)) { mutationPointer, mutationLength in
                    nativeExecuteRawWithRequestIDAndMutationID(
                        handle.pointer,
                        requestID ?? handle.reserveRequestID(),
                        operation,
                        itemIDPointer,
                        itemIDLength,
                        valuePointer,
                        valueLength,
                        conditionValue,
                        ttlEnabled,
                        ttlMilliseconds,
                        mutationPointer,
                        mutationLength
                    )
                }
            }
            return nativeExecuteRawWithRequestID(
                handle.pointer,
                requestID ?? handle.reserveRequestID(),
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

    private static func executeNative(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data,
        value: Data,
        condition: OpenKacheSetCondition?,
        ttl: UInt64?,
        mutationID: Data?,
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
            return Smithy_Native_Contract.setConditionNone
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
    return OpenKacheError(
        message.isEmpty ? "OpenKache native operation failed" : message,
        metadata: resultMetadata(result))
}

private func resultMetadata(_ result: NativeResultPointer) -> ErrorMetadata? {
    let bytes = 36
    let pointer = UnsafeMutableRawPointer.allocate(byteCount: bytes, alignment: 4)
    defer { pointer.deallocate() }
    guard nativeResultErrorMetadata(result, pointer) != 0 else {
        return nil
    }
    let mutationLength = Int(pointer.load(fromByteOffset: 18, as: UInt8.self))
    let boundedLength = min(mutationLength, nativeMutationIDBytes)
    let mutationID = boundedLength == 0
        ? nil
        : Data(
            bytes: pointer.advanced(by: 20),
            count: boundedLength)
    return ErrorMetadata(
        code: pointer.load(fromByteOffset: 0, as: UInt32.self),
        operation: pointer.load(fromByteOffset: 4, as: UInt32.self),
        phase: pointer.load(fromByteOffset: 8, as: UInt32.self),
        backend: pointer.load(fromByteOffset: 12, as: UInt32.self),
        retryable: pointer.load(fromByteOffset: 16, as: UInt8.self) != 0,
        ambiguous: pointer.load(fromByteOffset: 17, as: UInt8.self) != 0,
        mutationID: mutationID)
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue),
                requestID: requestID
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
        try validateKey(key)
        return try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                key: key,
                requestID: requestID
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "GET")
            }
        }
    }

    /// Retrieves protected bytes for a UTF-8 string key.
    public func get(_ key: String) async throws -> Data? {
        try await get(Data(key.utf8))
    }

    /// Stores protected bytes with optional Smithy condition and TTL.
    public func set(
        _ key: Data,
        value: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheSetOutcome {
        try validateKey(key)
        let ttl = try options.ttlMilliseconds()
        let mutationID = try options.validatedMutationID()
        return try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                key: key,
                value: value,
                condition: options.condition,
                ttl: ttl,
                mutationID: mutationID,
                requestID: requestID
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
        try await set(Data(key.utf8), value: value, options: options)
    }

    /// Deletes a key and reports whether it existed.
    public func delete(
        _ key: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheDeleteOutcome {
        try validateKey(key)
        let mutationID = try options.validatedMutationID()
        return try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                key: key,
                mutationID: mutationID,
                requestID: requestID
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "DELETE")
            }
        }
    }

    /// Deletes a UTF-8 string key.
    public func delete(
        _ key: String,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheDeleteOutcome {
        try await delete(Data(key.utf8), options: options)
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue),
                requestID: requestID
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue),
                requestID: requestID
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: Smithy_Native_Contract.operationReconnect,
                requestID: requestID
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

    /// Returns a point-in-time request, retry, cancellation, and lane snapshot.
    public func metricsSnapshot() async throws -> OpenKacheMetricsSnapshot {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await Task.detached(priority: nil) {
            try native.metricsSnapshot()
        }.value
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
        _ operation: @escaping @Sendable (NativeHandle, UInt64) throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        let requestID = native.reserveRequestID()
        return try await withTaskCancellationHandler(
            operation: {
                try await Task.detached(priority: nil) {
                    try operation(native, requestID)
                }.value
            },
            onCancel: {
                native.cancel(requestID: requestID)
            }
        )
    }

    private func validateKey(_ key: Data) throws {
        guard !key.isEmpty else {
            throw OpenKacheError("key must not be empty")
        }
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue),
                requestID: requestID
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected raw PING result")
                }
            }
        }
    }

    /// Retrieves exact bytes for a 32-byte protocol item ID.
    public func get(_ itemID: Data) async throws -> Data? {
        try validateItemID(itemID)
        return try await perform { handle, requestID in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                itemID: itemID,
                requestID: requestID
            )
            return try consumeResult(result) { kind, payload in
                try getOutcome(kind, payload: payload, operation: "raw GET")
            }
        }
    }

    /// Stores exact bytes for a 32-byte protocol item ID.
    public func set(
        _ itemID: Data,
        value: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheSetOutcome {
        try validateItemID(itemID)
        let ttl = try options.ttlMilliseconds()
        let mutationID = try options.validatedMutationID()
        return try await perform { handle, requestID in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                itemID: itemID,
                value: value,
                condition: options.condition,
                ttl: ttl,
                mutationID: mutationID,
                requestID: requestID
            )
            return try consumeResult(result) { kind, _ in
                try setOutcome(kind, operation: "raw SET")
            }
        }
    }

    /// Deletes a 32-byte protocol item ID.
    public func delete(
        _ itemID: Data,
        options: OpenKacheSetOptions = .init()
    ) async throws -> OpenKacheDeleteOutcome {
        try validateItemID(itemID)
        let mutationID = try options.validatedMutationID()
        return try await perform { handle, requestID in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                itemID: itemID,
                mutationID: mutationID,
                requestID: requestID
            )
            return try consumeResult(result) { kind, _ in
                try deleteOutcome(kind, operation: "raw DELETE")
            }
        }
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue),
                requestID: requestID
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue),
                requestID: requestID
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
        try await perform { handle, requestID in
            let result = try NativeBridge.execute(
                handle,
                operation: Smithy_Native_Contract.operationReconnect,
                requestID: requestID
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

    /// Returns a point-in-time request, retry, cancellation, and lane snapshot.
    public func metricsSnapshot() async throws -> OpenKacheMetricsSnapshot {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        return try await Task.detached(priority: nil) {
            try native.metricsSnapshot()
        }.value
    }

    /// Permanently closes this client. Repeated calls are safe.
    public func close() {
        native = nil
    }

    private func perform<T: Sendable>(
        _ operation: @escaping @Sendable (NativeHandle, UInt64) throws -> T
    ) async throws -> T {
        guard let native else {
            throw OpenKacheError("client is closed")
        }
        let requestID = native.reserveRequestID()
        return try await withTaskCancellationHandler(
            operation: {
                try await Task.detached(priority: nil) {
                    try operation(native, requestID)
                }.value
            },
            onCancel: {
                native.cancel(requestID: requestID)
            }
        )
    }

    private func validateItemID(_ itemID: Data) throws {
        guard itemID.count == Smithy_Value_Format.itemIdBytes else {
            throw OpenKacheError(
                "itemID must contain exactly \(Smithy_Value_Format.itemIdBytes) bytes"
            )
        }
    }
}

extension OpenKacheRawClient: Smithy_OpenKache_Api {
    public func ping(_ input: Smithy_Ping_Input) async throws -> Smithy_Ping_Output {
        _ = input
        try await ping()
        return Smithy_Ping_Output()
    }

    public func get(_ input: Smithy_Get_Input) async throws -> Smithy_Get_Output {
        Smithy_Get_Output(value: try await get(input.itemId))
    }

    public func set(_ input: Smithy_Set_Input) async throws -> Smithy_Set_Output {
        let ttl: Duration?
        if let ttlMilliseconds = input.ttlMilliseconds {
            guard ttlMilliseconds > 0 else {
                throw OpenKacheError("set.ttlMilliseconds must be greater than zero")
            }
            ttl = .milliseconds(ttlMilliseconds)
        } else {
            ttl = nil
        }
        let options = OpenKacheSetOptions(
            condition: input.condition,
            expiresAfter: ttl,
            mutationID: input.mutationId
        )
        let outcome = try await set(input.itemId, value: input.value, options: options)
        return Smithy_Set_Output(outcome: outcome)
    }

    public func delete(_ input: Smithy_Delete_Input) async throws -> Smithy_Delete_Output {
        let outcome = try await delete(
            input.itemId,
            options: OpenKacheSetOptions(mutationID: input.mutationId)
        )
        return Smithy_Delete_Output(deleted: outcome == .deleted)
    }

    public func stats(_ input: Smithy_Stats_Input) async throws -> Smithy_Stats_Output {
        _ = input
        return Smithy_Stats_Output(json: try await stats())
    }

    public func sync(_ input: Smithy_Sync_Input) async throws -> Smithy_Sync_Output {
        _ = input
        try await sync()
        return Smithy_Sync_Output()
    }
}

@_silgen_name("openkache_client_connection_state")
private func nativeConnectionState(_ client: NativeClientPointer?) -> UInt32

/// Optional SET condition and expiration.
public struct OpenKacheSetOptions: Sendable {
    /// Conditional write behavior, or nil for unconditional SET.
    public var condition: OpenKacheSetCondition?
    /// Positive relative lifetime.
    public var expiresAfter: Duration?
    /// Optional fixed-width idempotency token reused across retries.
    public var mutationID: Data?

    /// Creates unconditional persistent options.
    public init(
        condition: OpenKacheSetCondition? = nil,
        expiresAfter: Duration? = nil,
        mutationID: Data? = nil
    ) {
        self.condition = condition
        self.expiresAfter = expiresAfter
        self.mutationID = mutationID
    }

    fileprivate func ttlMilliseconds() throws -> UInt64? {
        guard let expiresAfter else {
            return nil
        }
        return try milliseconds(expiresAfter, named: "expiresAfter")
    }

    fileprivate func validatedMutationID() throws -> Data? {
        guard let mutationID else {
            return nil
        }
        guard mutationID.count == Smithy_Value_Format.mutationIdBytes else {
            throw OpenKacheError(
                "mutationID must contain exactly \(Smithy_Value_Format.mutationIdBytes) bytes"
            )
        }
        return mutationID
    }
}
