import Foundation

// The Rust client owns transport, TLS, retries, key derivation, compression,
// encryption, and value validation.  Swift only supplies native values and
// owns the actor/lifecycle surface.

private typealias NativeNamespaceDescriptor = Smithy_Native_Namespace_Descriptor

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
    let pointer: Smithy_Native_Client_Pointer

    init(pointer: Smithy_Native_Client_Pointer) {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
    ) throws -> Smithy_Native_Result_Pointer {
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
        ) -> Smithy_Native_Result_Pointer?
    ) throws -> Smithy_Native_Result_Pointer {
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

private func resultPayload(_ result: Smithy_Native_Result_Pointer) throws -> Data {
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

private func resultError(_ result: Smithy_Native_Result_Pointer) -> OpenKacheError {
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
    _ result: Smithy_Native_Result_Pointer,
    _ transform: (UInt32, Data) throws -> T
) throws -> T {
    defer { nativeFreeResult(result) }
    let kind = nativeResultKind(result)
    guard kind != Smithy_Native_Contract.resultError else {
        throw resultError(result)
    }
    return try transform(kind, resultPayload(result))
}

internal struct Smithy_Invocation_Result: Sendable {
    let kind: UInt32
    let payload: Data
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
        let (setFlags, ttl) = try options.wireOptions()
        return try await perform { handle in
            let result = try NativeBridge.executeWithOptions(
                handle,
                operation: UInt32(Smithy_Opcode.set.rawValue),
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
                try deleteOutcome(kind, operation: "DELETE")
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
        let (setFlags, ttl) = try options.wireOptions()
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
                try deleteOutcome(kind, operation: "raw DELETE")
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

    internal func smithyInvoke(
        _ operation: UInt32,
        key: Data = Data(),
        value: Data = Data(),
        condition: Smithy_Set_Condition? = nil,
        expirationMode: Smithy_Expiration_Mode? = nil,
        evictionMode: Smithy_Eviction_Mode? = nil,
        ttlMilliseconds: UInt64? = nil
    ) async throws -> Smithy_Invocation_Result {
        let (setFlags, ttl) = try smithySetFlags(
            condition: condition,
            expirationMode: expirationMode,
            evictionMode: evictionMode,
            ttlMilliseconds: ttlMilliseconds
        )
        return try await perform { handle in
            let result = try NativeBridge.executeRawWithOptions(
                handle,
                operation: operation,
                itemID: key,
                value: value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                Smithy_Invocation_Result(kind: kind, payload: payload)
            }
        }
    }

    internal func smithyInvokeScoped(
        _ operation: UInt32,
        namespaceID: UInt64,
        itemID: Data = Data(),
        value: Data = Data(),
        condition: Smithy_Set_Condition? = nil,
        expirationMode: Smithy_Expiration_Mode? = nil,
        evictionMode: Smithy_Eviction_Mode? = nil,
        ttlMilliseconds: UInt64? = nil
    ) async throws -> Smithy_Invocation_Result {
        if !itemID.isEmpty {
            try validateItemID(itemID)
        }
        let (setFlags, ttl) = try smithySetFlags(
            condition: condition,
            expirationMode: expirationMode,
            evictionMode: evictionMode,
            ttlMilliseconds: ttlMilliseconds
        )
        return try await perform { handle in
            let result = try NativeBridge.executeScoped(
                handle,
                operation: operation,
                namespaceID: namespaceID,
                itemID: itemID,
                value: value,
                setFlags: setFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                Smithy_Invocation_Result(kind: kind, payload: payload)
            }
        }
    }

    internal func smithyNamespaceOpen(
        name: String,
        createIfMissing: Bool,
        policyFlags: UInt8,
        ttl: UInt64
    ) async throws -> (descriptor: Smithy_Namespace_Descriptor, created: Bool) {
        return try await perform { handle in
            let result = try NativeBridge.namespaceOpen(
                handle,
                name: name,
                createIfMissing: createIfMissing,
                policyFlags: policyFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultOk
                    || kind == Smithy_Native_Contract.resultCreated
                else {
                    throw OpenKacheError("unexpected NAMESPACE_OPEN result")
                }
                return (
                    descriptor: smithyNamespaceDescriptor(
                        try NativeBridge.decodeNamespaceDescriptor(payload)
                    ),
                    created: kind == Smithy_Native_Contract.resultCreated
                )
            }
        }
    }

    internal func smithyNamespaceUpdatePolicy(
        namespaceID: UInt64,
        expectedRevision: UInt64,
        policyFlags: UInt8,
        ttl: UInt64
    ) async throws -> Smithy_Namespace_Descriptor {
        return try await perform { handle in
            let result = try NativeBridge.namespaceUpdatePolicy(
                handle,
                namespaceID: namespaceID,
                expectedRevision: expectedRevision,
                policyFlags: policyFlags,
                ttl: ttl
            )
            return try consumeResult(result) { kind, payload in
                guard kind == Smithy_Native_Contract.resultValue else {
                    throw OpenKacheError("unexpected NAMESPACE_UPDATE_POLICY result")
                }
                return smithyNamespaceDescriptor(
                    try NativeBridge.decodeNamespaceDescriptor(payload)
                )
            }
        }
    }

    internal func smithyNamespaceDelete(
        _ namespaceID: UInt64,
        expectedRevision: UInt64
    ) async throws {
        try await perform { handle in
            let result = try NativeBridge.namespaceDelete(
                handle,
                namespaceID: namespaceID,
                expectedRevision: expectedRevision
            )
            try consumeResult(result) { kind, _ in
                guard kind == Smithy_Native_Contract.resultOk else {
                    throw OpenKacheError("unexpected NAMESPACE_DELETE result")
                }
            }
        }
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

private func smithySetFlags(
    condition: Smithy_Set_Condition?,
    expirationMode: Smithy_Expiration_Mode?,
    evictionMode: Smithy_Eviction_Mode?,
    ttlMilliseconds: UInt64?
) throws -> (flags: UInt8, ttl: UInt64) {
    var flags: UInt8
    switch condition {
    case nil, .any:
        flags = Smithy_Value_Format.setConditionAnyBits
    case .ifAbsent:
        flags = Smithy_Value_Format.setIfAbsentBits
    case .ifPresent:
        flags = Smithy_Value_Format.setIfPresentBits
    }
    switch expirationMode {
    case nil, .inherit:
        guard ttlMilliseconds == nil else {
            throw OpenKacheError("ttlMilliseconds requires explicitTtl expiration mode")
        }
        flags |= Smithy_Value_Format.setInheritExpirationBits
    case .noExpiry:
        guard ttlMilliseconds == nil else {
            throw OpenKacheError("ttlMilliseconds is not valid with noExpiry")
        }
        flags |= Smithy_Value_Format.setNoExpiryBits
    case .explicitTtl:
        guard let ttl = ttlMilliseconds, ttl > 0 else {
            throw OpenKacheError("ttlMilliseconds must be positive with explicitTtl")
        }
        flags |= Smithy_Value_Format.setExplicitTtlBits
    }
    switch evictionMode {
    case nil, .inherit:
        flags |= Smithy_Value_Format.setInheritEvictionBits
    case .evictable:
        flags |= Smithy_Value_Format.setEvictableBits
    case .evictionProtected:
        flags |= Smithy_Value_Format.setEvictionProtectedBits
    }
    return (flags, ttlMilliseconds ?? 0)
}

internal func smithyPolicyFlags(
    defaultExpiration: Smithy_Expiration_Default,
    defaultTtlMilliseconds: UInt64?,
    expirationOverride: Smithy_Override_Policy,
    defaultEviction: Smithy_Eviction_Default,
    evictionOverride: Smithy_Override_Policy
) throws -> (flags: UInt8, ttl: UInt64) {
    var flags: UInt8
    switch defaultExpiration {
    case .noExpiry:
        guard defaultTtlMilliseconds == nil else {
            throw OpenKacheError("defaultTtlMilliseconds is invalid with noExpiry")
        }
        flags = Smithy_Value_Format.policyNoExpiry
    case .fixedTtl:
        guard let ttl = defaultTtlMilliseconds, ttl > 0 else {
            throw OpenKacheError("defaultTtlMilliseconds must be positive with fixedTtl")
        }
        flags = Smithy_Value_Format.policyFixedTtl
    }
    switch expirationOverride {
    case .allowed:
        flags |= Smithy_Value_Format.policyExpirationOverride
    case .disallowed:
        break
    }
    switch defaultEviction {
    case .evictable:
        break
    case .evictionProtected:
        flags |= Smithy_Value_Format.policyEvictionProtected
    }
    switch evictionOverride {
    case .allowed:
        flags |= Smithy_Value_Format.policyEvictionOverride
    case .disallowed:
        break
    }
    return (flags, defaultTtlMilliseconds ?? 0)
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
}
