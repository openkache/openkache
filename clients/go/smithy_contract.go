// Code generated from the OpenKache Smithy contract. DO NOT EDIT.

package openkache

const (
	// SmithyProtocolALPN is the negotiated protocol identifier.
	SmithyProtocolALPN = "openkache/1"
	// SmithyItemIDBytes is the exact protocol item-ID width.
	SmithyItemIDBytes = 32
	// SmithyMaxValueBytes is the protocol value and payload ceiling.
	SmithyMaxValueBytes = 67108864
	// SmithyDataProtectionKeyBytes is the shared key width.
	SmithyDataProtectionKeyBytes = 32
	// SmithyValueEncryptionNone selects unprotected values.
	SmithyValueEncryptionNone uint32 = 0
	// SmithyValueEncryptionCompact selects deterministic AES-SIV protection.
	SmithyValueEncryptionCompact uint32 = 1
	// SmithyValueEncryptionRobust selects randomized AES-GCM-SIV protection.
	SmithyValueEncryptionRobust uint32 = 2
)

// Smithy operation values carried by the native ABI.
const (
	SmithyOpcodePing   uint32 = 1
	SmithyOpcodeGet    uint32 = 2
	SmithyOpcodeSet    uint32 = 3
	SmithyOpcodeDelete uint32 = 4
	SmithyOpcodeStats  uint32 = 5
	SmithyOpcodeSync   uint32 = 6
)

// Smithy native ABI values shared by language adapters.
const (
	// SmithyFFIABIVersion is the native ABI version implemented by the core.
	SmithyFFIABIVersion uint32 = 3
	// SmithyFFIOperationGetJson identifies the native operation GetJson.
	SmithyFFIOperationGetJson uint32 = 7
	// SmithyFFIOperationSetJson identifies the native operation SetJson.
	SmithyFFIOperationSetJson uint32 = 8
	// SmithyFFIOperationReconnect identifies the native operation Reconnect.
	SmithyFFIOperationReconnect uint32 = 4294967041
	// SmithyFFIResultError is the native ABI result kind for Error.
	SmithyFFIResultError uint32 = 0
	// SmithyFFIResultOK is the native ABI result kind for Ok.
	SmithyFFIResultOK uint32 = 1
	// SmithyFFIResultValue is the native ABI result kind for Value.
	SmithyFFIResultValue uint32 = 2
	// SmithyFFIResultNotFound is the native ABI result kind for NotFound.
	SmithyFFIResultNotFound uint32 = 3
	// SmithyFFIResultCreated is the native ABI result kind for Created.
	SmithyFFIResultCreated uint32 = 4
	// SmithyFFIResultReplaced is the native ABI result kind for Replaced.
	SmithyFFIResultReplaced uint32 = 5
	// SmithyFFIResultDeleted is the native ABI result kind for Deleted.
	SmithyFFIResultDeleted uint32 = 6
	// SmithyFFIResultNotDeleted is the native ABI result kind for NotDeleted.
	SmithyFFIResultNotDeleted uint32 = 7
	// SmithyFFIResultConnected is the native ABI result kind for Connected.
	SmithyFFIResultConnected uint32 = 8
	// SmithyFFIResultNotStored is the native ABI result kind for NotStored.
	SmithyFFIResultNotStored uint32 = 9
	// SmithyFFISetConditionNone is the native ABI SET condition for None.
	SmithyFFISetConditionNone uint32 = 0
	// SmithyFFISetConditionIfAbsent is the native ABI SET condition for IfAbsent.
	SmithyFFISetConditionIfAbsent uint32 = 1
	// SmithyFFISetConditionIfPresent is the native ABI SET condition for IfPresent.
	SmithyFFISetConditionIfPresent uint32 = 2
	// SmithyFFIConnectionStateConnected identifies a native connection state.
	SmithyFFIConnectionStateConnected uint32 = 0
	// SmithyFFIConnectionStateReconnecting identifies a native connection state.
	SmithyFFIConnectionStateReconnecting uint32 = 1
	// SmithyFFIConnectionStateDisconnected identifies a native connection state.
	SmithyFFIConnectionStateDisconnected uint32 = 2
	// SmithyFFIConnectionStateClosed identifies a native connection state.
	SmithyFFIConnectionStateClosed uint32 = 3
	// SmithyFFIConnectionStateUnknown identifies a native connection state.
	SmithyFFIConnectionStateUnknown uint32 = 4
	// SmithyFFIErrorConfiguration identifies a structured native error code.
	SmithyFFIErrorConfiguration uint32 = 1
	// SmithyFFIErrorConnection identifies a structured native error code.
	SmithyFFIErrorConnection uint32 = 2
	// SmithyFFIErrorTimeout identifies a structured native error code.
	SmithyFFIErrorTimeout uint32 = 3
	// SmithyFFIErrorRuntime identifies a structured native error code.
	SmithyFFIErrorRuntime uint32 = 4
	// SmithyFFIErrorTransport identifies a structured native error code.
	SmithyFFIErrorTransport uint32 = 5
	// SmithyFFIErrorServer identifies a structured native error code.
	SmithyFFIErrorServer uint32 = 6
	// SmithyFFIErrorUnexpectedResponse identifies a structured native error code.
	SmithyFFIErrorUnexpectedResponse uint32 = 7
	// SmithyFFIErrorResponseTooLarge identifies a structured native error code.
	SmithyFFIErrorResponseTooLarge uint32 = 8
	// SmithyFFIErrorTls identifies a structured native error code.
	SmithyFFIErrorTls uint32 = 9
	// SmithyFFIErrorProtocol identifies a structured native error code.
	SmithyFFIErrorProtocol uint32 = 10
	// SmithyFFIErrorIo identifies a structured native error code.
	SmithyFFIErrorIo uint32 = 11
	// SmithyFFIErrorValue identifies a structured native error code.
	SmithyFFIErrorValue uint32 = 12
	// SmithyFFIErrorClosed identifies a structured native error code.
	SmithyFFIErrorClosed uint32 = 13
	// SmithyFFIErrorAmbiguous identifies a structured native error code.
	SmithyFFIErrorAmbiguous uint32 = 14
	// SmithyFFIErrorCancelled identifies a structured native error code.
	SmithyFFIErrorCancelled uint32 = 15
	// SmithyFFIPhaseUnknown identifies a structured native error phase.
	SmithyFFIPhaseUnknown uint32 = 0
	// SmithyFFIPhaseDnsResolution identifies a structured native error phase.
	SmithyFFIPhaseDnsResolution uint32 = 1
	// SmithyFFIPhaseConnectionSetup identifies a structured native error phase.
	SmithyFFIPhaseConnectionSetup uint32 = 2
	// SmithyFFIPhaseConnectionRetry identifies a structured native error phase.
	SmithyFFIPhaseConnectionRetry uint32 = 3
	// SmithyFFIPhaseStreamAcquisition identifies a structured native error phase.
	SmithyFFIPhaseStreamAcquisition uint32 = 4
	// SmithyFFIPhaseRequestWrite identifies a structured native error phase.
	SmithyFFIPhaseRequestWrite uint32 = 5
	// SmithyFFIPhaseResponseHeaderRead identifies a structured native error phase.
	SmithyFFIPhaseResponseHeaderRead uint32 = 6
	// SmithyFFIPhaseResponseBodyRead identifies a structured native error phase.
	SmithyFFIPhaseResponseBodyRead uint32 = 7
	// SmithyFFIPhaseTlsInitialization identifies a structured native error phase.
	SmithyFFIPhaseTlsInitialization uint32 = 8
	// SmithyFFIPhaseEndpointInitialization identifies a structured native error phase.
	SmithyFFIPhaseEndpointInitialization uint32 = 9
	// SmithyFFIPhaseConnectionInitialization identifies a structured native error phase.
	SmithyFFIPhaseConnectionInitialization uint32 = 10
	// SmithyFFIPhaseHandshake identifies a structured native error phase.
	SmithyFFIPhaseHandshake uint32 = 11
	// SmithyFFIPhaseStreamOpen identifies a structured native error phase.
	SmithyFFIPhaseStreamOpen uint32 = 12
	// SmithyFFIPhaseStreamWrite identifies a structured native error phase.
	SmithyFFIPhaseStreamWrite uint32 = 13
	// SmithyFFIPhaseStreamRead identifies a structured native error phase.
	SmithyFFIPhaseStreamRead uint32 = 14
	// SmithyFFIBackendNone identifies a native transport backend.
	SmithyFFIBackendNone uint32 = 0
	// SmithyFFIBackendQuinn identifies a native transport backend.
	SmithyFFIBackendQuinn uint32 = 1
	// SmithyFFIBackendCompio identifies a native transport backend.
	SmithyFFIBackendCompio uint32 = 2
	// SmithyFFIMetricsRequests identifies a metrics snapshot field.
	SmithyFFIMetricsRequests uint32 = 0
	// SmithyFFIMetricsHits identifies a metrics snapshot field.
	SmithyFFIMetricsHits uint32 = 1
	// SmithyFFIMetricsMisses identifies a metrics snapshot field.
	SmithyFFIMetricsMisses uint32 = 2
	// SmithyFFIMetricsRetries identifies a metrics snapshot field.
	SmithyFFIMetricsRetries uint32 = 3
	// SmithyFFIMetricsReconnects identifies a metrics snapshot field.
	SmithyFFIMetricsReconnects uint32 = 4
	// SmithyFFIMetricsCancellations identifies a metrics snapshot field.
	SmithyFFIMetricsCancellations uint32 = 5
	// SmithyFFIMetricsTransportErrors identifies a metrics snapshot field.
	SmithyFFIMetricsTransportErrors uint32 = 6
	// SmithyFFIMetricsProtocolErrors identifies a metrics snapshot field.
	SmithyFFIMetricsProtocolErrors uint32 = 7
	// SmithyFFIMetricsBytesSent identifies a metrics snapshot field.
	SmithyFFIMetricsBytesSent uint32 = 8
	// SmithyFFIMetricsBytesReceived identifies a metrics snapshot field.
	SmithyFFIMetricsBytesReceived uint32 = 9
	// SmithyFFIMetricsActiveLanes identifies a metrics snapshot field.
	SmithyFFIMetricsActiveLanes uint32 = 10
)

// Shared client defaults extracted from the Smithy service contract.
const (
	// SmithyDefaultMaxInFlight is the default number of request lanes.
	SmithyDefaultMaxInFlight = 256
	// SmithyMutationIDBytes is the fixed width of a mutation idempotency token.
	SmithyMutationIDBytes = 16
	// SmithyMaxPreviousDataProtectionKeys bounds the retired key read/delete window.
	SmithyMaxPreviousDataProtectionKeys = 8
	// SmithyDefaultConnectTimeoutMilliseconds is the default connection timeout.
	SmithyDefaultConnectTimeoutMilliseconds uint64 = 5000
	// SmithyDefaultRequestTimeoutMilliseconds is the default complete request timeout.
	SmithyDefaultRequestTimeoutMilliseconds uint64 = 2000
	// SmithyDefaultRetryMaxAttempts is the default total retry attempt count.
	SmithyDefaultRetryMaxAttempts = 2
	// SmithyDefaultZstandardLevel is the default Zstandard level.
	SmithyDefaultZstandardLevel int32 = 1
	// SmithyDefaultZstandardMinimumInputBytes is the compression input threshold.
	SmithyDefaultZstandardMinimumInputBytes = 1024
	// SmithyDefaultZstandardMinimumSavingsBytes is the compression savings threshold.
	SmithyDefaultZstandardMinimumSavingsBytes = 64
	// SmithyDefaultZstandardLevelMin is the minimum supported Zstandard level.
	SmithyDefaultZstandardLevelMin int32 = 1
	// SmithyDefaultZstandardLevelMax is the maximum supported Zstandard level.
	SmithyDefaultZstandardLevelMax int32 = 22
	// SmithyClientDefaultServerName is used when no TLS server name is supplied.
	SmithyClientDefaultServerName = "localhost"
	// SmithyClientCertificatePEMType is the PEM block type used for certificate chains.
	SmithyClientCertificatePEMType = "CERTIFICATE"
	// SmithyClientMinimumPositiveValue is the minimum accepted positive setting.
	SmithyClientMinimumPositiveValue = 1
)

// Smithy API enum string values extracted from the Smithy service contract.
const (
	// SmithySetConditionIfAbsentValue is the Smithy SetCondition value for if_absent.
	SmithySetConditionIfAbsentValue = "if_absent"
	// SmithySetConditionIfPresentValue is the Smithy SetCondition value for if_present.
	SmithySetConditionIfPresentValue = "if_present"
	// SmithySetOutcomeCreatedValue is the Smithy SetOutcome value for created.
	SmithySetOutcomeCreatedValue = "created"
	// SmithySetOutcomeReplacedValue is the Smithy SetOutcome value for replaced.
	SmithySetOutcomeReplacedValue = "replaced"
	// SmithySetOutcomeNotStoredValue is the Smithy SetOutcome value for not_stored.
	SmithySetOutcomeNotStoredValue = "not_stored"
)
