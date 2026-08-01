// Generated from the OpenKache Smithy contract. Do not edit.
library;

const int smithyItemIdBytes = 32;
const int smithyMutationIdBytes = 16;
const int smithyMaxValueBytes = 67108864;
const int smithyValueDataProtectionKeyBytes = 32;
const String smithyProtocolAlpn = "openkache/1";
const int smithyDefaultMaxInFlight = 256;
const int smithyDefaultConnectTimeoutMilliseconds = 5000;
const int smithyDefaultRequestTimeoutMilliseconds = 2000;
const int smithyDefaultRetryMaxAttempts = 2;
const int smithyDefaultZstandardLevel = 1;
const int smithyDefaultZstandardMinimumInputBytes = 1024;
const int smithyDefaultZstandardMinimumSavingsBytes = 64;
const int smithyFfiAbiVersion = 3;

const int smithy_opcode_ping = 1;
const int smithy_opcode_get = 2;
const int smithy_opcode_set = 3;
const int smithy_opcode_delete = 4;
const int smithy_opcode_stats = 5;
const int smithy_opcode_sync = 6;

const int smithy_ffi_operation_get_json = 7;
const int smithy_ffi_operation_set_json = 8;
const int smithy_ffi_operation_reconnect = 4294967041;

const int smithy_ffi_result_error = 0;
const int smithy_ffi_result_ok = 1;
const int smithy_ffi_result_value = 2;
const int smithy_ffi_result_not_found = 3;
const int smithy_ffi_result_created = 4;
const int smithy_ffi_result_replaced = 5;
const int smithy_ffi_result_deleted = 6;
const int smithy_ffi_result_not_deleted = 7;
const int smithy_ffi_result_connected = 8;
const int smithy_ffi_result_not_stored = 9;

const int smithy_ffi_set_condition_none = 0;
const int smithy_ffi_set_condition_if_absent = 1;
const int smithy_ffi_set_condition_if_present = 2;

const int smithy_ffi_connection_state_connected = 0;
const int smithy_ffi_connection_state_reconnecting = 1;
const int smithy_ffi_connection_state_disconnected = 2;
const int smithy_ffi_connection_state_closed = 3;
const int smithy_ffi_connection_state_unknown = 4;

const int smithy_ffi_error_configuration = 1;
const int smithy_ffi_error_connection = 2;
const int smithy_ffi_error_timeout = 3;
const int smithy_ffi_error_runtime = 4;
const int smithy_ffi_error_transport = 5;
const int smithy_ffi_error_server = 6;
const int smithy_ffi_error_unexpected_response = 7;
const int smithy_ffi_error_response_too_large = 8;
const int smithy_ffi_error_tls = 9;
const int smithy_ffi_error_protocol = 10;
const int smithy_ffi_error_io = 11;
const int smithy_ffi_error_value = 12;
const int smithy_ffi_error_closed = 13;
const int smithy_ffi_error_ambiguous = 14;
const int smithy_ffi_error_cancelled = 15;

const int smithy_ffi_phase_unknown = 0;
const int smithy_ffi_phase_dns_resolution = 1;
const int smithy_ffi_phase_connection_setup = 2;
const int smithy_ffi_phase_connection_retry = 3;
const int smithy_ffi_phase_stream_acquisition = 4;
const int smithy_ffi_phase_request_write = 5;
const int smithy_ffi_phase_response_header_read = 6;
const int smithy_ffi_phase_response_body_read = 7;
const int smithy_ffi_phase_tls_initialization = 8;
const int smithy_ffi_phase_endpoint_initialization = 9;
const int smithy_ffi_phase_connection_initialization = 10;
const int smithy_ffi_phase_handshake = 11;
const int smithy_ffi_phase_stream_open = 12;
const int smithy_ffi_phase_stream_write = 13;
const int smithy_ffi_phase_stream_read = 14;

const int smithy_ffi_backend_none = 0;
const int smithy_ffi_backend_quinn = 1;
const int smithy_ffi_backend_compio = 2;

const int smithy_ffi_metrics_requests = 0;
const int smithy_ffi_metrics_hits = 1;
const int smithy_ffi_metrics_misses = 2;
const int smithy_ffi_metrics_retries = 3;
const int smithy_ffi_metrics_reconnects = 4;
const int smithy_ffi_metrics_cancellations = 5;
const int smithy_ffi_metrics_transport_errors = 6;
const int smithy_ffi_metrics_protocol_errors = 7;
const int smithy_ffi_metrics_bytes_sent = 8;
const int smithy_ffi_metrics_bytes_received = 9;
const int smithy_ffi_metrics_active_lanes = 10;
