#ifndef OPENKACHE_CLIENT_ABI_H
#define OPENKACHE_CLIENT_ABI_H

/*
 * Stable C ABI shared by native language adapters.
 *
 * The implementation is exported by openkache-client-core when the `ffi`
 * feature is enabled. This header owns no resources and is safe to include
 * from C++ and other foreign-function generators.
 */

#include <stddef.h>
#include <stdint.h>

#if defined(__has_include)
#  if __has_include(<openkache/smithy_contract.h>)
#    include <openkache/smithy_contract.h>
#  else
#    include "smithy_contract.h"
#  endif
#else
#  include "smithy_contract.h"
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define OPENKACHE_CLIENT_ABI_VERSION OPENKACHE_SMITHY_FFI_ABI_VERSION
#define OPENKACHE_CLIENT_DATA_PROTECTION_KEY_BYTES \
    OPENKACHE_SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES

typedef struct openkache_client openkache_client_t;
typedef struct openkache_client_result openkache_client_result_t;

/* Naming aliases used by generated adapters in other languages. */
typedef openkache_client_t openkache_client_handle;
typedef openkache_client_result_t openkache_client_result;

/*
 * Decoded namespace descriptor returned by the shared ABI helper.
 * `default_expiration` is 0 for no expiry and 1 for fixed TTL.
 * `default_eviction` is 0 for evictable and 1 for eviction protected.
 * Override fields are 0 for disallowed and 1 for allowed.
 */
typedef struct openkache_client_namespace_descriptor {
    uint64_t namespace_id;
    uint64_t revision;
    uint64_t default_ttl_ms;
    uint32_t default_expiration;
    uint32_t expiration_override;
    uint32_t default_eviction;
    uint32_t eviction_override;
} openkache_client_namespace_descriptor_t;

#define OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_OK 0u
#define OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_INVALID 1u
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EXPIRATION_NO_EXPIRY 0u
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL 1u
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EVICTION_EVICTABLE 0u
#define OPENKACHE_CLIENT_NAMESPACE_DEFAULT_EVICTION_PROTECTED 1u
#define OPENKACHE_CLIENT_NAMESPACE_OVERRIDE_DISALLOWED 0u
#define OPENKACHE_CLIENT_NAMESPACE_OVERRIDE_ALLOWED 1u

typedef enum openkache_client_result_kind {
    OPENKACHE_CLIENT_RESULT_ERROR = OPENKACHE_SMITHY_FFI_RESULT_ERROR,
    OPENKACHE_CLIENT_RESULT_OK = OPENKACHE_SMITHY_FFI_RESULT_OK,
    OPENKACHE_CLIENT_RESULT_VALUE = OPENKACHE_SMITHY_FFI_RESULT_VALUE,
    OPENKACHE_CLIENT_RESULT_NOT_FOUND = OPENKACHE_SMITHY_FFI_RESULT_NOT_FOUND,
    OPENKACHE_CLIENT_RESULT_CREATED = OPENKACHE_SMITHY_FFI_RESULT_CREATED,
    OPENKACHE_CLIENT_RESULT_REPLACED = OPENKACHE_SMITHY_FFI_RESULT_REPLACED,
    OPENKACHE_CLIENT_RESULT_DELETED = OPENKACHE_SMITHY_FFI_RESULT_DELETED,
    OPENKACHE_CLIENT_RESULT_NOT_DELETED = OPENKACHE_SMITHY_FFI_RESULT_NOT_DELETED,
    OPENKACHE_CLIENT_RESULT_CONNECTED = OPENKACHE_SMITHY_FFI_RESULT_CONNECTED,
    OPENKACHE_CLIENT_RESULT_NOT_STORED = OPENKACHE_SMITHY_FFI_RESULT_NOT_STORED,
} openkache_client_result_kind_t;

typedef enum openkache_client_operation {
    OPENKACHE_CLIENT_OPERATION_PING = OPENKACHE_SMITHY_OPCODE_PING,
    OPENKACHE_CLIENT_OPERATION_GET = OPENKACHE_SMITHY_OPCODE_GET,
    OPENKACHE_CLIENT_OPERATION_SET = OPENKACHE_SMITHY_OPCODE_SET,
    OPENKACHE_CLIENT_OPERATION_DELETE = OPENKACHE_SMITHY_OPCODE_DELETE,
    OPENKACHE_CLIENT_OPERATION_STATS = OPENKACHE_SMITHY_OPCODE_STATS,
    OPENKACHE_CLIENT_OPERATION_SYNC = OPENKACHE_SMITHY_OPCODE_SYNC,
    OPENKACHE_CLIENT_OPERATION_NAMESPACE_OPEN = OPENKACHE_SMITHY_OPCODE_NAMESPACE_OPEN,
    OPENKACHE_CLIENT_OPERATION_NAMESPACE_UPDATE_POLICY =
        OPENKACHE_SMITHY_OPCODE_NAMESPACE_UPDATE_POLICY,
    OPENKACHE_CLIENT_OPERATION_NAMESPACE_DELETE = OPENKACHE_SMITHY_OPCODE_NAMESPACE_DELETE,
    /* Language-adapter operations; these are not wire opcodes. */
    OPENKACHE_CLIENT_OPERATION_GET_JSON = OPENKACHE_SMITHY_FFI_OPERATION_GET_JSON,
    OPENKACHE_CLIENT_OPERATION_SET_JSON = OPENKACHE_SMITHY_FFI_OPERATION_SET_JSON,
} openkache_client_operation_t;

/*
 * Local lifecycle operation; it is not a wire opcode. Keep its Smithy u32
 * value as a macro because C enum constants must fit in an `int`.
 */
#define OPENKACHE_CLIENT_OPERATION_RECONNECT OPENKACHE_SMITHY_FFI_OPERATION_RECONNECT

typedef enum openkache_client_connection_state {
    OPENKACHE_CLIENT_CONNECTION_CONNECTED = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CONNECTED,
    OPENKACHE_CLIENT_CONNECTION_RECONNECTING = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_RECONNECTING,
    OPENKACHE_CLIENT_CONNECTION_DISCONNECTED = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_DISCONNECTED,
    OPENKACHE_CLIENT_CONNECTION_CLOSED = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_CLOSED,
    OPENKACHE_CLIENT_CONNECTION_UNKNOWN = OPENKACHE_SMITHY_FFI_CONNECTION_STATE_UNKNOWN,
} openkache_client_connection_state_t;

typedef enum openkache_client_set_condition {
    OPENKACHE_CLIENT_SET_CONDITION_ANY = OPENKACHE_SMITHY_FFI_SET_CONDITION_ANY,
    OPENKACHE_CLIENT_SET_CONDITION_IF_ABSENT =
        OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_ABSENT,
    OPENKACHE_CLIENT_SET_CONDITION_IF_PRESENT =
        OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_PRESENT,
} openkache_client_set_condition_t;

typedef enum openkache_client_encryption {
    OPENKACHE_CLIENT_ENCRYPTION_NONE = OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE,
    OPENKACHE_CLIENT_ENCRYPTION_COMPACT = OPENKACHE_SMITHY_VALUE_ENCRYPTION_COMPACT,
    OPENKACHE_CLIENT_ENCRYPTION_ROBUST = OPENKACHE_SMITHY_VALUE_ENCRYPTION_ROBUST,
} openkache_client_encryption_t;

typedef struct openkache_client_connect_options {
    const uint8_t *address;
    size_t address_length;
    const uint8_t *server_name;
    size_t server_name_length;
    const uint8_t *certificate;
    size_t certificate_length;
    const uint8_t *client_certificate_chain;
    size_t client_certificate_chain_length;
    const uint8_t *client_private_key;
    size_t client_private_key_length;
    const uint8_t *data_protection_key;
    size_t data_protection_key_length;
    uint8_t compression_enabled;
    int32_t compression_level;
    size_t minimum_input_size;
    size_t minimum_savings;
    uint32_t encryption;
    uint64_t connect_timeout_ms;
    uint64_t request_timeout_ms;
    size_t retry_max_attempts;
    size_t max_in_flight;
} openkache_client_connect_options_t;

uint32_t openkache_client_abi_version(void);

/*
 * Connects a protected client.
 *
 * `address` is a UTF-8 host/port authority such as "127.0.0.1:4433" or
 * "cache.example.com:4433".
 * `server_name` is the TLS DNS name or IP identity.
 * `certificate` is one DER-encoded server trust certificate, a PEM chain,
 * or an empty buffer to use system trust roots.
 * All input buffers are copied before this call returns.
 */
openkache_client_result_t *openkache_client_connect(
    const uint8_t *address,
    size_t address_length,
    const uint8_t *server_name,
    size_t server_name_length,
    const uint8_t *certificate,
    size_t certificate_length,
    const uint8_t *data_protection_key,
    size_t data_protection_key_length,
    uint8_t compression_enabled,
    int32_t compression_level,
    size_t minimum_input_size,
    size_t minimum_savings,
    uint64_t connect_timeout_ms,
    uint64_t request_timeout_ms
);

/*
 * Extended connection entry point. It accepts the same fields as the
 * legacy function plus optional mutual TLS, encryption, retry, and lane
 * settings. Zero numeric extension fields select shared-core defaults.
 */
openkache_client_result_t *openkache_client_connect_ex(
    const uint8_t *address,
    size_t address_length,
    const uint8_t *server_name,
    size_t server_name_length,
    const uint8_t *certificate,
    size_t certificate_length,
    const uint8_t *client_certificate_chain,
    size_t client_certificate_chain_length,
    const uint8_t *client_private_key,
    size_t client_private_key_length,
    const uint8_t *data_protection_key,
    size_t data_protection_key_length,
    uint8_t compression_enabled,
    int32_t compression_level,
    size_t minimum_input_size,
    size_t minimum_savings,
    uint32_t encryption,
    size_t retry_max_attempts,
    size_t max_in_flight,
    uint64_t connect_timeout_ms,
    uint64_t request_timeout_ms
);

/*
 * Named-field convenience entry point for C callers. It is equivalent to
 * the flat `openkache_client_connect_ex` function.
 */
openkache_client_result_t *openkache_client_connect_with_options(
    const openkache_client_connect_options_t *options
);

/*
 * Executes one operation. The result payload is borrowed and must be copied
 * before freeing the result. Empty buffers are represented by a null pointer
 * or any pointer with length zero.
 */
openkache_client_result_t *openkache_client_execute(
    const openkache_client_t *client,
    uint32_t operation,
    const uint8_t *application_key,
    size_t application_key_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms
);

/*
 * Executes exact protocol item-ID operations. GET, SET, and DELETE require
 * exactly OPENKACHE_SMITHY_ITEM_ID_BYTES key bytes and bypass application-key
 * derivation and value protection.
 */
openkache_client_result_t *openkache_client_execute_raw(
    const openkache_client_t *client,
    uint32_t operation,
    const uint8_t *item_id,
    size_t item_id_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms
);

/*
 * Executes one protected operation with the complete wire SET policy byte.
 * `set_flags` and `ttl_ms` must be zero for operations other than SET.
 */
openkache_client_result_t *openkache_client_execute_with_options(
    const openkache_client_t *client,
    uint32_t operation,
    const uint8_t *application_key,
    size_t application_key_length,
    const uint8_t *value,
    size_t value_length,
    uint8_t set_flags,
    uint64_t ttl_ms
);

/*
 * Executes one exact-item-ID operation with the complete wire SET policy byte.
 * `set_flags` and `ttl_ms` must be zero for operations other than SET.
 */
openkache_client_result_t *openkache_client_execute_raw_with_options(
    const openkache_client_t *client,
    uint32_t operation,
    const uint8_t *item_id,
    size_t item_id_length,
    const uint8_t *value,
    size_t value_length,
    uint8_t set_flags,
    uint64_t ttl_ms
);

/*
 * Executes exact item-ID data-plane operations in an explicitly supplied
 * namespace. `set_flags` is the complete wire SET flag byte; it and `ttl_ms`
 * must be zero for operations other than SET.
 */
openkache_client_result_t *openkache_client_execute_scoped(
    const openkache_client_t *client,
    uint32_t operation,
    uint64_t namespace_id,
    const uint8_t *item_id,
    size_t item_id_length,
    const uint8_t *value,
    size_t value_length,
    uint8_t set_flags,
    uint64_t ttl_ms
);

/*
 * Namespace-management ABI calls. Namespace descriptors use the protocol
 * payload layout: namespace_id (u64 BE), revision (u64 BE), policy flags
 * (u8), and a canonical variable unsigned integer default TTL only when the
 * policy selects FixedTtl. No-expiry descriptors therefore contain 17 bytes;
 * FixedTtl descriptors contain 18 through 26 bytes.
 */
openkache_client_result_t *openkache_client_namespace_open(
    const openkache_client_t *client,
    const uint8_t *name,
    size_t name_length,
    uint8_t create_if_missing,
    uint8_t policy_flags,
    uint64_t ttl_ms
);

openkache_client_result_t *openkache_client_namespace_update_policy(
    const openkache_client_t *client,
    uint64_t namespace_id,
    uint64_t expected_revision,
    uint8_t policy_flags,
    uint64_t ttl_ms
);

openkache_client_result_t *openkache_client_namespace_delete(
    const openkache_client_t *client,
    uint64_t namespace_id,
    uint64_t expected_revision
);

/*
 * Decodes the complete namespace descriptor payload using the canonical Rust
 * protocol implementation. Returns
 * OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_OK on success and
 * OPENKACHE_CLIENT_NAMESPACE_DESCRIPTOR_DECODE_INVALID for malformed input.
 */
uint32_t openkache_client_namespace_descriptor_decode(
    const uint8_t *payload,
    size_t payload_length,
    openkache_client_namespace_descriptor_t *output
);

/* Returns one of openkache_client_connection_state_t; null handles return UNKNOWN. */
uint32_t openkache_client_connection_state(const openkache_client_t *client);

uint32_t openkache_client_result_kind(const openkache_client_result_t *result);
const uint8_t *openkache_client_result_data(const openkache_client_result_t *result);
size_t openkache_client_result_data_length(const openkache_client_result_t *result);
openkache_client_t *openkache_client_result_take_client(openkache_client_result_t *result);
void openkache_client_result_free(openkache_client_result_t *result);
void openkache_client_free(openkache_client_t *client);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENKACHE_CLIENT_ABI_H */
