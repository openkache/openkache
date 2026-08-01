$version: "2"

namespace openkache.client

/// Defaults used by the shared client core and language adapters.
@trait(selector: "service")
structure clientDefaults {
    @required
    maxInFlight: Integer

    /// Maximum number of retired data-protection keys retained for rotation.
    @required
    maxPreviousDataProtectionKeys: Integer

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

    /// Stable structured-error category identifiers.
    @required
    errorConfiguration: Integer
    @required
    errorConnection: Integer
    @required
    errorTimeout: Integer
    @required
    errorRuntime: Integer
    @required
    errorTransport: Integer
    @required
    errorServer: Integer
    @required
    errorUnexpectedResponse: Integer
    @required
    errorResponseTooLarge: Integer
    @required
    errorTls: Integer
    @required
    errorProtocol: Integer
    @required
    errorIo: Integer
    @required
    errorValue: Integer
    @required
    errorClosed: Integer
    @required
    errorAmbiguous: Integer
    @required
    errorCancelled: Integer

    /// Stable operation-phase identifiers used by structured-error metadata.
    @required
    phaseUnknown: Integer
    @required
    phaseDnsResolution: Integer
    @required
    phaseConnectionSetup: Integer
    @required
    phaseConnectionRetry: Integer
    @required
    phaseStreamAcquisition: Integer
    @required
    phaseRequestWrite: Integer
    @required
    phaseResponseHeaderRead: Integer
    @required
    phaseResponseBodyRead: Integer
    @required
    phaseTlsInitialization: Integer
    @required
    phaseEndpointInitialization: Integer
    @required
    phaseConnectionInitialization: Integer
    @required
    phaseHandshake: Integer
    @required
    phaseStreamOpen: Integer
    @required
    phaseStreamWrite: Integer
    @required
    phaseStreamRead: Integer

    /// Native transport backend identifiers.
    @required
    backendNone: Integer
    @required
    backendQuinn: Integer
    @required
    backendCompio: Integer

    /// Metrics snapshot field identifiers.
    @required
    metricsRequests: Integer
    @required
    metricsHits: Integer
    @required
    metricsMisses: Integer
    @required
    metricsRetries: Integer
    @required
    metricsReconnects: Integer
    @required
    metricsCancellations: Integer
    @required
    metricsTransportErrors: Integer
    @required
    metricsProtocolErrors: Integer
    @required
    metricsBytesSent: Integer
    @required
    metricsBytesReceived: Integer
    @required
    metricsActiveLanes: Integer

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
    setConditionNone: Integer

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

/// Native C ABI structure sizes and offsets used by raw FFI adapters.
///
/// These values are part of the versioned ABI. Bindings that allocate or decode
/// raw native buffers must consume the generated contract instead of duplicating
/// platform-layout assumptions in language-specific source.
@trait(selector: "service")
structure ffiLayout {
    @required
    connectOptionsBytes: Integer
    @required
    connectAddressOffset: Integer
    @required
    connectAddressLengthOffset: Integer
    @required
    connectServerNameOffset: Integer
    @required
    connectServerNameLengthOffset: Integer
    @required
    connectCertificateOffset: Integer
    @required
    connectCertificateLengthOffset: Integer
    @required
    connectClientCertificateChainOffset: Integer
    @required
    connectClientCertificateChainLengthOffset: Integer
    @required
    connectClientPrivateKeyOffset: Integer
    @required
    connectClientPrivateKeyLengthOffset: Integer
    @required
    connectDataProtectionKeyOffset: Integer
    @required
    connectDataProtectionKeyLengthOffset: Integer
    @required
    connectPreviousDataProtectionKeysOffset: Integer
    @required
    connectPreviousDataProtectionKeysLengthOffset: Integer
    @required
    connectPreviousDataProtectionKeyCountOffset: Integer
    @required
    connectCompressionEnabledOffset: Integer
    @required
    connectCompressionLevelOffset: Integer
    @required
    connectMinimumInputSizeOffset: Integer
    @required
    connectMinimumSavingsOffset: Integer
    @required
    connectEncryptionOffset: Integer
    @required
    connectTimeoutOffset: Integer
    @required
    connectRequestTimeoutOffset: Integer
    @required
    connectRetryMaxAttemptsOffset: Integer
    @required
    connectMaxInFlightOffset: Integer

    @required
    errorMetadataBytes: Integer
    @required
    errorMetadataCodeOffset: Integer
    @required
    errorMetadataOperationOffset: Integer
    @required
    errorMetadataPhaseOffset: Integer
    @required
    errorMetadataBackendOffset: Integer
    @required
    errorMetadataRetryableOffset: Integer
    @required
    errorMetadataAmbiguousOffset: Integer
    @required
    errorMetadataMutationIdLengthOffset: Integer
    @required
    errorMetadataMutationIdOffset: Integer

    @required
    metricsSnapshotBytes: Integer
    @required
    metricsSnapshotRequestsOffset: Integer
    @required
    metricsSnapshotHitsOffset: Integer
    @required
    metricsSnapshotMissesOffset: Integer
    @required
    metricsSnapshotRetriesOffset: Integer
    @required
    metricsSnapshotReconnectsOffset: Integer
    @required
    metricsSnapshotCancellationsOffset: Integer
    @required
    metricsSnapshotTransportErrorsOffset: Integer
    @required
    metricsSnapshotProtocolErrorsOffset: Integer
    @required
    metricsSnapshotBytesSentOffset: Integer
    @required
    metricsSnapshotBytesReceivedOffset: Integer
    @required
    metricsSnapshotActiveLanesOffset: Integer
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

@clientDefaults(
    maxInFlight: 256,
    maxPreviousDataProtectionKeys: 8,
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
    abiVersion: 3,
    errorConfiguration: 1,
    errorConnection: 2,
    errorTimeout: 3,
    errorRuntime: 4,
    errorTransport: 5,
    errorServer: 6,
    errorUnexpectedResponse: 7,
    errorResponseTooLarge: 8,
    errorTls: 9,
    errorProtocol: 10,
    errorIo: 11,
    errorValue: 12,
    errorClosed: 13,
    errorAmbiguous: 14,
    errorCancelled: 15,
    phaseUnknown: 0,
    phaseDnsResolution: 1,
    phaseConnectionSetup: 2,
    phaseConnectionRetry: 3,
    phaseStreamAcquisition: 4,
    phaseRequestWrite: 5,
    phaseResponseHeaderRead: 6,
    phaseResponseBodyRead: 7,
    phaseTlsInitialization: 8,
    phaseEndpointInitialization: 9,
    phaseConnectionInitialization: 10,
    phaseHandshake: 11,
    phaseStreamOpen: 12,
    phaseStreamWrite: 13,
    phaseStreamRead: 14,
    backendNone: 0,
    backendQuinn: 1,
    backendCompio: 2,
    metricsRequests: 0,
    metricsHits: 1,
    metricsMisses: 2,
    metricsRetries: 3,
    metricsReconnects: 4,
    metricsCancellations: 5,
    metricsTransportErrors: 6,
    metricsProtocolErrors: 7,
    metricsBytesSent: 8,
    metricsBytesReceived: 9,
    metricsActiveLanes: 10,
    operationGetJson: 7,
    operationSetJson: 8,
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
    setConditionNone: 0,
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
@ffiLayout(
    connectOptionsBytes: 184,
    connectAddressOffset: 0,
    connectAddressLengthOffset: 8,
    connectServerNameOffset: 16,
    connectServerNameLengthOffset: 24,
    connectCertificateOffset: 32,
    connectCertificateLengthOffset: 40,
    connectClientCertificateChainOffset: 48,
    connectClientCertificateChainLengthOffset: 56,
    connectClientPrivateKeyOffset: 64,
    connectClientPrivateKeyLengthOffset: 72,
    connectDataProtectionKeyOffset: 80,
    connectDataProtectionKeyLengthOffset: 88,
    connectPreviousDataProtectionKeysOffset: 96,
    connectPreviousDataProtectionKeysLengthOffset: 104,
    connectPreviousDataProtectionKeyCountOffset: 112,
    connectCompressionEnabledOffset: 120,
    connectCompressionLevelOffset: 124,
    connectMinimumInputSizeOffset: 128,
    connectMinimumSavingsOffset: 136,
    connectEncryptionOffset: 144,
    connectTimeoutOffset: 152,
    connectRequestTimeoutOffset: 160,
    connectRetryMaxAttemptsOffset: 168,
    connectMaxInFlightOffset: 176,
    errorMetadataBytes: 36,
    errorMetadataCodeOffset: 0,
    errorMetadataOperationOffset: 4,
    errorMetadataPhaseOffset: 8,
    errorMetadataBackendOffset: 12,
    errorMetadataRetryableOffset: 16,
    errorMetadataAmbiguousOffset: 17,
    errorMetadataMutationIdLengthOffset: 18,
    errorMetadataMutationIdOffset: 20,
    metricsSnapshotBytes: 88,
    metricsSnapshotRequestsOffset: 0,
    metricsSnapshotHitsOffset: 8,
    metricsSnapshotMissesOffset: 16,
    metricsSnapshotRetriesOffset: 24,
    metricsSnapshotReconnectsOffset: 32,
    metricsSnapshotCancellationsOffset: 40,
    metricsSnapshotTransportErrorsOffset: 48,
    metricsSnapshotProtocolErrorsOffset: 56,
    metricsSnapshotBytesSentOffset: 64,
    metricsSnapshotBytesReceivedOffset: 72,
    metricsSnapshotActiveLanesOffset: 80
)
service OpenKacheClient {
    version: "1"
    operations: [Ping, Get, Set, Delete, Stats, Sync]
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

    /// Optional fixed-width idempotency token reused for mutation retries.
    mutationId: Blob
}

structure SetOutput {
    @required
    outcome: SetOutcome
}

structure DeleteInput {
    @required
    itemId: ItemId

    /// Optional fixed-width idempotency token reused for mutation retries.
    mutationId: Blob
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
