// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated

/** Stable wire, FFI, value-format, and client-default constants. */
object SmithyContract {
    const val ITEM_ID_BYTES: Int = 32
    const val MUTATION_ID_BYTES: Int = 16
    const val MAX_VALUE_BYTES: Int = 67108864
    const val ALPN: String = "openkache/1"
    const val DEFAULT_MAX_IN_FLIGHT: Int = 256
    const val DEFAULT_CONNECT_TIMEOUT_MILLISECONDS: Long = 5000L
    const val DEFAULT_REQUEST_TIMEOUT_MILLISECONDS: Long = 2000L
    const val DEFAULT_RETRY_MAX_ATTEMPTS: Int = 2
    const val VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES: Int = 32
    const val VALUE_FORMAT_ENCRYPTION_NONE: Int = 0
    const val VALUE_FORMAT_ENCRYPTION_COMPACT: Int = 1
    const val VALUE_FORMAT_ENCRYPTION_ROBUST: Int = 2
    const val FFI_ABI_VERSION: Int = 3

    const val OPCODE_Ping: Int = 1
    const val OPCODE_Get: Int = 2
    const val OPCODE_Set: Int = 3
    const val OPCODE_Delete: Int = 4
    const val OPCODE_Stats: Int = 5
    const val OPCODE_Sync: Int = 6

    const val FFI_OPERATION_GetJson: Int = 7
    const val FFI_OPERATION_SetJson: Int = 8
    const val FFI_OPERATION_Reconnect: Int = 0xffffff01

    const val FFI_RESULT_Error: Int = 0
    const val FFI_RESULT_Ok: Int = 1
    const val FFI_RESULT_Value: Int = 2
    const val FFI_RESULT_NotFound: Int = 3
    const val FFI_RESULT_Created: Int = 4
    const val FFI_RESULT_Replaced: Int = 5
    const val FFI_RESULT_Deleted: Int = 6
    const val FFI_RESULT_NotDeleted: Int = 7
    const val FFI_RESULT_Connected: Int = 8
    const val FFI_RESULT_NotStored: Int = 9

    const val FFI_SET_CONDITION_None: Int = 0
    const val FFI_SET_CONDITION_IfAbsent: Int = 1
    const val FFI_SET_CONDITION_IfPresent: Int = 2

    const val FFI_CONNECTION_STATE_Connected: Int = 0
    const val FFI_CONNECTION_STATE_Reconnecting: Int = 1
    const val FFI_CONNECTION_STATE_Disconnected: Int = 2
    const val FFI_CONNECTION_STATE_Closed: Int = 3
    const val FFI_CONNECTION_STATE_Unknown: Int = 4

    const val FFI_ERROR_Configuration: Int = 1
    const val FFI_ERROR_Connection: Int = 2
    const val FFI_ERROR_Timeout: Int = 3
    const val FFI_ERROR_Runtime: Int = 4
    const val FFI_ERROR_Transport: Int = 5
    const val FFI_ERROR_Server: Int = 6
    const val FFI_ERROR_UnexpectedResponse: Int = 7
    const val FFI_ERROR_ResponseTooLarge: Int = 8
    const val FFI_ERROR_Tls: Int = 9
    const val FFI_ERROR_Protocol: Int = 10
    const val FFI_ERROR_Io: Int = 11
    const val FFI_ERROR_Value: Int = 12
    const val FFI_ERROR_Closed: Int = 13
    const val FFI_ERROR_Ambiguous: Int = 14
    const val FFI_ERROR_Cancelled: Int = 15

    const val FFI_PHASE_Unknown: Int = 0
    const val FFI_PHASE_DnsResolution: Int = 1
    const val FFI_PHASE_ConnectionSetup: Int = 2
    const val FFI_PHASE_ConnectionRetry: Int = 3
    const val FFI_PHASE_StreamAcquisition: Int = 4
    const val FFI_PHASE_RequestWrite: Int = 5
    const val FFI_PHASE_ResponseHeaderRead: Int = 6
    const val FFI_PHASE_ResponseBodyRead: Int = 7
    const val FFI_PHASE_TlsInitialization: Int = 8
    const val FFI_PHASE_EndpointInitialization: Int = 9
    const val FFI_PHASE_ConnectionInitialization: Int = 10
    const val FFI_PHASE_Handshake: Int = 11
    const val FFI_PHASE_StreamOpen: Int = 12
    const val FFI_PHASE_StreamWrite: Int = 13
    const val FFI_PHASE_StreamRead: Int = 14

    const val FFI_BACKEND_None: Int = 0
    const val FFI_BACKEND_Quinn: Int = 1
    const val FFI_BACKEND_Compio: Int = 2

    const val FFI_METRICS_Requests: Int = 0
    const val FFI_METRICS_Hits: Int = 1
    const val FFI_METRICS_Misses: Int = 2
    const val FFI_METRICS_Retries: Int = 3
    const val FFI_METRICS_Reconnects: Int = 4
    const val FFI_METRICS_Cancellations: Int = 5
    const val FFI_METRICS_TransportErrors: Int = 6
    const val FFI_METRICS_ProtocolErrors: Int = 7
    const val FFI_METRICS_BytesSent: Int = 8
    const val FFI_METRICS_BytesReceived: Int = 9
    const val FFI_METRICS_ActiveLanes: Int = 10
}
