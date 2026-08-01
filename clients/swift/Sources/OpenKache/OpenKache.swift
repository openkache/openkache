import Foundation

// The Rust client owns transport, TLS, retries, key derivation, compression,
// encryption, and value validation.  Swift only supplies native values and
// owns the actor/lifecycle surface.

private typealias NativeClientPointer = OpaquePointer
private typealias NativeResultPointer = OpaquePointer

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
        maxInFlight: Int = Smithy_Value_Format.defaultMaxInFlight
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

private enum NativeBridge {
    static func connect(options: OpenKacheClientOptions) throws -> NativeHandle {
        guard nativeAbiVersion() == Smithy_Native_Contract.abiVersion else {
            throw OpenKacheError("unsupported OpenKache native ABI version")
        }
        guard !options.address.isEmpty else {
            throw OpenKacheError("address must not be empty")
        }
        guard options.dataProtectionKey.count == Smithy_Value_Format.dataProtectionKeyBytes else {
            throw OpenKacheError(
                "dataProtectionKey must contain exactly \(Smithy_Value_Format.dataProtectionKeyBytes) bytes"
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

        let result = withBytes(address) { addressPointer, addressLength in
            withBytes(serverName) { serverNamePointer, serverNameLength in
                withBytes(certificate) { certificatePointer, certificateLength in
                    withBytes(clientCertificate) { clientCertificatePointer, clientCertificateLength in
                        withBytes(clientPrivateKey) { clientPrivateKeyPointer, clientPrivateKeyLength in
                            withBytes(dataProtectionKey) { keyPointer, keyLength in
                                nativeConnect(
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

    static func execute(
        _ handle: NativeHandle,
        operation: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        condition: OpenKacheSetCondition? = nil,
        ttl: UInt64? = nil
    ) throws -> NativeResultPointer {
        let conditionValue: UInt32
        switch condition {
        case .none:
            conditionValue = Smithy_Native_Contract.setConditionNone
        case .ifAbsent:
            conditionValue = Smithy_Native_Contract.setConditionIfAbsent
        case .ifPresent:
            conditionValue = Smithy_Native_Contract.setConditionIfPresent
        }
        return try withBytes(Array(key)) { keyPointer, keyLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = nativeExecute(
                    handle.pointer,
                    operation,
                    keyPointer,
                    keyLength,
                    valuePointer,
                    valueLength,
                    conditionValue,
                    ttl == nil ? 0 : 1,
                    ttl ?? 0
                ) else {
                    throw OpenKacheError("native client returned a null operation result")
                }
                return result
            }
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
        let conditionValue: UInt32
        switch condition {
        case .none:
            conditionValue = Smithy_Native_Contract.setConditionNone
        case .ifAbsent:
            conditionValue = Smithy_Native_Contract.setConditionIfAbsent
        case .ifPresent:
            conditionValue = Smithy_Native_Contract.setConditionIfPresent
        }
        return try withBytes(Array(itemID)) { itemIDPointer, itemIDLength in
            try withBytes(Array(value)) { valuePointer, valueLength in
                guard let result = nativeExecuteRaw(
                    handle.pointer,
                    operation,
                    itemIDPointer,
                    itemIDLength,
                    valuePointer,
                    valueLength,
                    conditionValue,
                    ttl == nil ? 0 : 1,
                    ttl ?? 0
                ) else {
                    throw OpenKacheError("native client returned a null raw operation result")
                }
                return result
            }
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

private func resultPayload(_ result: NativeResultPointer) -> Data {
    let length = nativeResultDataLength(result)
    guard length > 0, let pointer = nativeResultData(result) else {
        return Data()
    }
    return Data(bytes: pointer, count: length)
}

private func resultError(_ result: NativeResultPointer) -> OpenKacheError {
    let payload = resultPayload(result)
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
    return try transform(kind, resultPayload(result))
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue)
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
        return try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                key: key
            )
            return try consumeResult(result) { kind, payload in
                switch kind {
                case Smithy_Native_Contract.resultValue:
                    return payload
                case Smithy_Native_Contract.resultNotFound:
                    return nil
                default:
                    throw OpenKacheError("unexpected GET result")
                }
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
        return try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                key: key,
                value: value,
                condition: options.condition,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                switch kind {
                case Smithy_Native_Contract.resultCreated:
                    return .created
                case Smithy_Native_Contract.resultReplaced:
                    return .replaced
                case Smithy_Native_Contract.resultNotStored:
                    return .notStored
                default:
                    throw OpenKacheError("unexpected SET result")
                }
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
    public func delete(_ key: Data) async throws -> OpenKacheDeleteOutcome {
        try validateKey(key)
        return try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                key: key
            )
            return try consumeResult(result) { kind, _ in
                switch kind {
                case Smithy_Native_Contract.resultDeleted:
                    return .deleted
                case Smithy_Native_Contract.resultNotDeleted:
                    return .notFound
                default:
                    throw OpenKacheError("unexpected DELETE result")
                }
            }
        }
    }

    /// Deletes a UTF-8 string key.
    public func delete(_ key: String) async throws -> OpenKacheDeleteOutcome {
        try await delete(Data(key.utf8))
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue)
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue)
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: Smithy_Native_Contract.operationReconnect
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.ping.rawValue)
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
        return try await perform { handle in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.get.rawValue),
                itemID: itemID
            )
            return try consumeResult(result) { kind, payload in
                switch kind {
                case Smithy_Native_Contract.resultValue:
                    return payload
                case Smithy_Native_Contract.resultNotFound:
                    return nil
                default:
                    throw OpenKacheError("unexpected raw GET result")
                }
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
        return try await perform { handle in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
                itemID: itemID,
                value: value,
                condition: options.condition,
                ttl: ttl
            )
            return try consumeResult(result) { kind, _ in
                switch kind {
                case Smithy_Native_Contract.resultCreated:
                    return .created
                case Smithy_Native_Contract.resultReplaced:
                    return .replaced
                case Smithy_Native_Contract.resultNotStored:
                    return .notStored
                default:
                    throw OpenKacheError("unexpected raw SET result")
                }
            }
        }
    }

    /// Deletes a 32-byte protocol item ID.
    public func delete(_ itemID: Data) async throws -> OpenKacheDeleteOutcome {
        try validateItemID(itemID)
        return try await perform { handle in
            let result = try NativeBridge.executeRaw(
                handle,
                operation: UInt32(Smithy_Opcode.delete.rawValue),
                itemID: itemID
            )
            return try consumeResult(result) { kind, _ in
                switch kind {
                case Smithy_Native_Contract.resultDeleted:
                    return .deleted
                case Smithy_Native_Contract.resultNotDeleted:
                    return .notFound
                default:
                    throw OpenKacheError("unexpected raw DELETE result")
                }
            }
        }
    }

    /// Returns the server's JSON statistics payload.
    public func stats() async throws -> String {
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.stats.rawValue)
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: UInt32(Smithy_Opcode.sync.rawValue)
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
        try await perform { handle in
            let result = try NativeBridge.execute(
                handle,
                operation: Smithy_Native_Contract.operationReconnect
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
            expiresAfter: ttl
        )
        let outcome = try await set(input.itemId, value: input.value, options: options)
        return Smithy_Set_Output(outcome: outcome)
    }

    public func delete(_ input: Smithy_Delete_Input) async throws -> Smithy_Delete_Output {
        let outcome = try await delete(input.itemId)
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

    /// Creates unconditional persistent options.
    public init(
        condition: OpenKacheSetCondition? = nil,
        expiresAfter: Duration? = nil
    ) {
        self.condition = condition
        self.expiresAfter = expiresAfter
    }

    fileprivate func ttlMilliseconds() throws -> UInt64? {
        guard let expiresAfter else {
            return nil
        }
        return try milliseconds(expiresAfter, named: "expiresAfter")
    }
}
