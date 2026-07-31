#ifndef OPENKACHE_CLIENT_CORE_H
#define OPENKACHE_CLIENT_CORE_H

/*
 * Versioned native ABI shared by the OpenKache language bindings.
 *
 * The symbols are exported by the openkache-client Rust cdylib.  Handles and
 * result payloads remain owned by Rust; callers must use the result accessors
 * and free functions below rather than releasing memory themselves.
 */

#include <stddef.h>
#include <stdint.h>

#include "openkache/smithy_contract.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FfiClient OpenKacheClient;
typedef struct FfiResult OpenKacheClientResult;

uint32_t openkache_client_abi_version(void);

/*
 * Legacy connection entry point.  `certificate` must be one DER certificate
 * or a PEM chain.  It uses system-independent defaults for retry and
 * concurrency and requires the certificate to be non-empty.
 */
OpenKacheClientResult *openkache_client_connect(
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

/*
 * Complete connection configuration.  An empty trust certificate selects
 * system roots.  Client certificate and private key are optional, but must be
 * supplied together.  Each certificate argument accepts one DER certificate
 * or a PEM chain.
 */
OpenKacheClientResult *openkache_client_connect_ex(
    const uint8_t *address,
    size_t address_length,
    const uint8_t *server_name,
    size_t server_name_length,
    const uint8_t *certificate,
    size_t certificate_length,
    const uint8_t *client_certificate,
    size_t client_certificate_length,
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
    uint64_t request_timeout_ms);

OpenKacheClientResult *openkache_client_execute(
    const OpenKacheClient *client,
    uint32_t operation,
    const uint8_t *application_key,
    size_t application_key_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms);

/*
 * Executes an exact protocol item-ID operation. GET and SET values bypass
 * application-key derivation and value protection. Only GET, SET, and DELETE
 * are valid operations; `item_id_length` must equal the Smithy item-ID width.
 */
OpenKacheClientResult *openkache_client_execute_raw(
    const OpenKacheClient *client,
    uint32_t operation,
    const uint8_t *item_id,
    size_t item_id_length,
    const uint8_t *value,
    size_t value_length,
    uint32_t set_condition,
    uint8_t ttl_enabled,
    uint64_t ttl_ms);

/* Returns one of OPENKACHE_SMITHY_CONNECTION_STATE_* values. */
uint32_t openkache_client_connection_state(const OpenKacheClient *client);

uint32_t openkache_client_result_kind(const OpenKacheClientResult *result);
const uint8_t *openkache_client_result_data(const OpenKacheClientResult *result);
size_t openkache_client_result_data_length(const OpenKacheClientResult *result);
OpenKacheClient *openkache_client_result_take_client(OpenKacheClientResult *result);
void openkache_client_result_free(OpenKacheClientResult *result);
void openkache_client_free(OpenKacheClient *client);

#ifdef __cplusplus
}
#endif

#endif
