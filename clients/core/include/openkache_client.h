/*
 * Code generated from the OpenKache Smithy contract. DO NOT EDIT.
 *
 * Stable OpenKache native-client ABI.
 *
 * The implementation is owned by openkache-client-core. Language bindings
 * should include this header and treat every returned handle as opaque. The
 * ABI uses an owned-result pattern so bindings never free Rust allocations
 * directly.
 */
#ifndef OPENKACHE_CLIENT_H
#define OPENKACHE_CLIENT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define OPENKACHE_CLIENT_ABI_VERSION 1u

enum openkache_client_result_kind {
    OPENKACHE_CLIENT_RESULT_ERROR = 0u,
    OPENKACHE_CLIENT_RESULT_OK = 1u,
    OPENKACHE_CLIENT_RESULT_VALUE = 2u,
    OPENKACHE_CLIENT_RESULT_NOT_FOUND = 3u,
    OPENKACHE_CLIENT_RESULT_CREATED = 4u,
    OPENKACHE_CLIENT_RESULT_REPLACED = 5u,
    OPENKACHE_CLIENT_RESULT_DELETED = 6u,
    OPENKACHE_CLIENT_RESULT_NOT_DELETED = 7u,
    OPENKACHE_CLIENT_RESULT_CONNECTED = 8u,
    OPENKACHE_CLIENT_RESULT_NOT_STORED = 9u,
};

enum openkache_client_operation {
    OPENKACHE_CLIENT_OPERATION_PING = 1u,
    OPENKACHE_CLIENT_OPERATION_GET = 2u,
    OPENKACHE_CLIENT_OPERATION_SET = 3u,
    OPENKACHE_CLIENT_OPERATION_DELETE = 4u,
    OPENKACHE_CLIENT_OPERATION_STATS = 5u,
    OPENKACHE_CLIENT_OPERATION_SYNC = 6u,
};

enum openkache_client_set_condition {
    OPENKACHE_CLIENT_SET_CONDITION_NONE = 0u,
    OPENKACHE_CLIENT_SET_CONDITION_IF_ABSENT = 1u,
    OPENKACHE_CLIENT_SET_CONDITION_IF_PRESENT = 2u,
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

#endif /* OPENKACHE_CLIENT_H */
