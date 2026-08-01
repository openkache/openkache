// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated;

/** Stable wire, FFI, value-format, and client-default constants. */
public final class SmithyContract {
    private SmithyContract() {}

    public static final int ITEM_ID_BYTES = 32;
    public static final int MUTATION_ID_BYTES = 16;
    public static final int MAX_VALUE_BYTES = 67108864;
    public static final String ALPN = "openkache/1";
    public static final int DEFAULT_MAX_IN_FLIGHT = 256;
    public static final long DEFAULT_CONNECT_TIMEOUT_MILLISECONDS = 5000L;
    public static final long DEFAULT_REQUEST_TIMEOUT_MILLISECONDS = 2000L;
    public static final int DEFAULT_RETRY_MAX_ATTEMPTS = 2;
    public static final int DEFAULT_ZSTANDARD_LEVEL = 1;
    public static final int DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES = 1024;
    public static final int DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES = 64;
    public static final int VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES = 32;
    public static final int VALUE_FORMAT_ENCRYPTION_NONE = 0;
    public static final int VALUE_FORMAT_ENCRYPTION_COMPACT = 1;
    public static final int VALUE_FORMAT_ENCRYPTION_ROBUST = 2;
    public static final int FFI_ABI_VERSION = 3;

    public static final int OPCODE_Ping = 1;
    public static final int OPCODE_Get = 2;
    public static final int OPCODE_Set = 3;
    public static final int OPCODE_Delete = 4;
    public static final int OPCODE_Stats = 5;
    public static final int OPCODE_Sync = 6;

    public static final int FFI_OPERATION_GetJson = 7;
    public static final int FFI_OPERATION_SetJson = 8;
    public static final int FFI_OPERATION_Reconnect = 0xffffff01;

    public static final int FFI_RESULT_Error = 0;
    public static final int FFI_RESULT_Ok = 1;
    public static final int FFI_RESULT_Value = 2;
    public static final int FFI_RESULT_NotFound = 3;
    public static final int FFI_RESULT_Created = 4;
    public static final int FFI_RESULT_Replaced = 5;
    public static final int FFI_RESULT_Deleted = 6;
    public static final int FFI_RESULT_NotDeleted = 7;
    public static final int FFI_RESULT_Connected = 8;
    public static final int FFI_RESULT_NotStored = 9;

    public static final int FFI_SET_CONDITION_None = 0;
    public static final int FFI_SET_CONDITION_IfAbsent = 1;
    public static final int FFI_SET_CONDITION_IfPresent = 2;

    public static final int FFI_CONNECTION_STATE_Connected = 0;
    public static final int FFI_CONNECTION_STATE_Reconnecting = 1;
    public static final int FFI_CONNECTION_STATE_Disconnected = 2;
    public static final int FFI_CONNECTION_STATE_Closed = 3;
    public static final int FFI_CONNECTION_STATE_Unknown = 4;

    public static final int FFI_ERROR_Configuration = 1;
    public static final int FFI_ERROR_Connection = 2;
    public static final int FFI_ERROR_Timeout = 3;
    public static final int FFI_ERROR_Runtime = 4;
    public static final int FFI_ERROR_Transport = 5;
    public static final int FFI_ERROR_Server = 6;
    public static final int FFI_ERROR_UnexpectedResponse = 7;
    public static final int FFI_ERROR_ResponseTooLarge = 8;
    public static final int FFI_ERROR_Tls = 9;
    public static final int FFI_ERROR_Protocol = 10;
    public static final int FFI_ERROR_Io = 11;
    public static final int FFI_ERROR_Value = 12;
    public static final int FFI_ERROR_Closed = 13;
    public static final int FFI_ERROR_Ambiguous = 14;
    public static final int FFI_ERROR_Cancelled = 15;

    public static final int FFI_PHASE_Unknown = 0;
    public static final int FFI_PHASE_DnsResolution = 1;
    public static final int FFI_PHASE_ConnectionSetup = 2;
    public static final int FFI_PHASE_ConnectionRetry = 3;
    public static final int FFI_PHASE_StreamAcquisition = 4;
    public static final int FFI_PHASE_RequestWrite = 5;
    public static final int FFI_PHASE_ResponseHeaderRead = 6;
    public static final int FFI_PHASE_ResponseBodyRead = 7;
    public static final int FFI_PHASE_TlsInitialization = 8;
    public static final int FFI_PHASE_EndpointInitialization = 9;
    public static final int FFI_PHASE_ConnectionInitialization = 10;
    public static final int FFI_PHASE_Handshake = 11;
    public static final int FFI_PHASE_StreamOpen = 12;
    public static final int FFI_PHASE_StreamWrite = 13;
    public static final int FFI_PHASE_StreamRead = 14;

    public static final int FFI_BACKEND_None = 0;
    public static final int FFI_BACKEND_Quinn = 1;
    public static final int FFI_BACKEND_Compio = 2;

    public static final int FFI_METRICS_Requests = 0;
    public static final int FFI_METRICS_Hits = 1;
    public static final int FFI_METRICS_Misses = 2;
    public static final int FFI_METRICS_Retries = 3;
    public static final int FFI_METRICS_Reconnects = 4;
    public static final int FFI_METRICS_Cancellations = 5;
    public static final int FFI_METRICS_TransportErrors = 6;
    public static final int FFI_METRICS_ProtocolErrors = 7;
    public static final int FFI_METRICS_BytesSent = 8;
    public static final int FFI_METRICS_BytesReceived = 9;
    public static final int FFI_METRICS_ActiveLanes = 10;
}
