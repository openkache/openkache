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
	// SmithyValueEnvelopeMagicAndVersion is the legacy envelope prefix.
	SmithyValueEnvelopeMagicAndVersion = "\x4f\x4b\x56\x01"
)

// Smithy operation values carried by the native ABI.
const (
	SmithyOpcodePing uint32 = 1
	SmithyOpcodeGet uint32 = 2
	SmithyOpcodeSet uint32 = 3
	SmithyOpcodeDelete uint32 = 4
	SmithyOpcodeStats uint32 = 5
	SmithyOpcodeSync uint32 = 6
)

// Smithy native ABI values shared by language adapters.
const (
	// SmithyFFIABIVersion is the native ABI version implemented by the core.
	SmithyFFIABIVersion uint32 = 1
	// SmithyFFIAdapterOperationGetJson identifies the language-adapter operation GetJson.
	SmithyFFIAdapterOperationGetJson uint32 = 7
	// SmithyFFIAdapterOperationSetJson identifies the language-adapter operation SetJson.
	SmithyFFIAdapterOperationSetJson uint32 = 8
	// SmithyFFIAdapterOperationReconnect identifies the language-adapter operation Reconnect.
	SmithyFFIAdapterOperationReconnect uint32 = 9
	// SmithyFFIAdapterOperationState identifies the language-adapter operation State.
	SmithyFFIAdapterOperationState uint32 = 10
	// SmithyFFIAdapterOperationRawGet identifies the language-adapter operation RawGet.
	SmithyFFIAdapterOperationRawGet uint32 = 11
	// SmithyFFIAdapterOperationRawSet identifies the language-adapter operation RawSet.
	SmithyFFIAdapterOperationRawSet uint32 = 12
	// SmithyFFIAdapterOperationRawDelete identifies the language-adapter operation RawDelete.
	SmithyFFIAdapterOperationRawDelete uint32 = 13
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
	// SmithyFFIResultState is the native ABI result kind for State.
	SmithyFFIResultState uint32 = 10
	// SmithyFFISetConditionNone is the native ABI SET condition for None.
	SmithyFFISetConditionNone uint32 = 0
	// SmithyFFISetConditionIfAbsent is the native ABI SET condition for IfAbsent.
	SmithyFFISetConditionIfAbsent uint32 = 1
	// SmithyFFISetConditionIfPresent is the native ABI SET condition for IfPresent.
	SmithyFFISetConditionIfPresent uint32 = 2
	// SmithyFFIOperationReconnect requests an explicit connection replacement.
	SmithyFFIOperationReconnect uint32 = 4294967041
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
)

// Shared client defaults extracted from the Smithy service contract.
const (
	// SmithyClientDefaultServerName is used when no TLS server name is supplied.
	SmithyClientDefaultServerName = "localhost"
	// SmithyClientDefaultConnectTimeoutMS is the default connection timeout.
	SmithyClientDefaultConnectTimeoutMS uint64 = 5000
	// SmithyClientDefaultRequestTimeoutMS is the default complete request timeout.
	SmithyClientDefaultRequestTimeoutMS uint64 = 2000
	// SmithyClientDefaultRetryMaxAttempts is the default total retry attempt count.
	SmithyClientDefaultRetryMaxAttempts = 2
	// SmithyClientDefaultMaxInFlight is the default number of request lanes.
	SmithyClientDefaultMaxInFlight = 256
	// SmithyClientDefaultCompressionLevel is the default Zstandard level.
	SmithyClientDefaultCompressionLevel int32 = 1
	// SmithyClientDefaultCompressionMinimumInputSize is the compression input threshold.
	SmithyClientDefaultCompressionMinimumInputSize = 1024
	// SmithyClientDefaultCompressionMinimumSavings is the compression savings threshold.
	SmithyClientDefaultCompressionMinimumSavings = 64
	// SmithyClientCompressionLevelMin is the minimum supported Zstandard level.
	SmithyClientCompressionLevelMin int32 = 1
	// SmithyClientCompressionLevelMax is the maximum supported Zstandard level.
	SmithyClientCompressionLevelMax int32 = 22
)
