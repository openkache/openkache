#ifndef OPENKACHE_CLIENT_ABI_H
#define OPENKACHE_CLIENT_ABI_H

/*
 * Stable OpenKache native-client ABI.
 *
 * The implementation is owned by openkache-client-core. Language bindings
 * should include this header and treat every returned handle as opaque. The
 * ABI uses an owned-result pattern so bindings never free Rust allocations
 * directly.
 */

#include <stddef.h>
#include <stdint.h>

#include "smithy_contract.h"

#ifdef __cplusplus
extern "C" {
#endif

#define OPENKACHE_CLIENT_ABI_VERSION OPENKACHE_SMITHY_FFI_ABI_VERSION

enum openkache_client_result_kind {
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
};

enum openkache_client_operation {
    OPENKACHE_CLIENT_OPERATION_PING = OPENKACHE_SMITHY_OPCODE_PING,
    OPENKACHE_CLIENT_OPERATION_GET = OPENKACHE_SMITHY_OPCODE_GET,
    OPENKACHE_CLIENT_OPERATION_SET = OPENKACHE_SMITHY_OPCODE_SET,
    OPENKACHE_CLIENT_OPERATION_DELETE = OPENKACHE_SMITHY_OPCODE_DELETE,
    OPENKACHE_CLIENT_OPERATION_STATS = OPENKACHE_SMITHY_OPCODE_STATS,
    OPENKACHE_CLIENT_OPERATION_SYNC = OPENKACHE_SMITHY_OPCODE_SYNC,
};

enum openkache_client_set_condition {
    OPENKACHE_CLIENT_SET_CONDITION_NONE = OPENKACHE_SMITHY_FFI_SET_CONDITION_NONE,
    OPENKACHE_CLIENT_SET_CONDITION_IF_ABSENT =
        OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_ABSENT,
    OPENKACHE_CLIENT_SET_CONDITION_IF_PRESENT =
        OPENKACHE_SMITHY_FFI_SET_CONDITION_IF_PRESENT,
};

typedef struct openkache_client_result openkache_client_result;
typedef struct openkache_client_handle openkache_client_handle;

uint32_t openkache_client_abi_version(void);

openkache_client_result *openkache_client_connect(
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
    uint64_t request_timeout_ms);

openkache_client_result *openkache_client_connect_ex(
    const uint8_t *address,
    size_t address_length,
    const uint8_t *server_name,
    size_t server_name_length,
    const uint8_t *certificate,
    size_t certificate_length,
    const uint8_t *identity_certificate_chain,
    size_t identity_certificate_chain_length,
    const uint8_t *identity_private_key,
    size_t identity_private_key_length,
    const uint8_t *data_protection_key,
    size_t data_protection_key_length,
    uint8_t compression_enabled,
    int32_t compression_level,
    size_t minimum_input_size,
    size_t minimum_savings,
    uint64_t connect_timeout_ms,
    uint64_t request_timeout_ms,
    uint64_t retry_max_attempts,
    size_t max_in_flight);

openkache_client_result *openkache_client_execute(
    const openkache_client_handle *client,
    uint32_t operation,
    const uint8_t *application_key,
    size_t application_key_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms);

openkache_client_result *openkache_client_execute_raw(
    const openkache_client_handle *client,
    uint32_t operation,
    const uint8_t *item_id,
    size_t item_id_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms);

uint32_t openkache_client_result_kind(const openkache_client_result *result);
const uint8_t *openkache_client_result_data(const openkache_client_result *result);
size_t openkache_client_result_data_length(const openkache_client_result *result);
openkache_client_handle *openkache_client_result_take_client(
    openkache_client_result *result);
void openkache_client_result_free(openkache_client_result *result);
void openkache_client_free(openkache_client_handle *client);

#ifdef __cplusplus
}
#endif

#endif /* OPENKACHE_CLIENT_ABI_H */
