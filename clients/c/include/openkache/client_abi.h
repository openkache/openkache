#ifndef OPENKACHE_CLIENT_GATE0_ABI_H
#define OPENKACHE_CLIENT_GATE0_ABI_H

/*
 * Maintained Gate 0 C ABI.
 *
 * The generated shared-core ABI is an implementation detail of the package
 * wrapper and is deliberately not installed.  This header exposes only the
 * five operations and the result ownership helpers needed to consume them.
 */

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct openkache_client openkache_client_t;
typedef struct openkache_client_result openkache_client_result_t;

enum {
  OPENKACHE_CLIENT_GATE0_RESULT_ERROR = 0u,
  OPENKACHE_CLIENT_GATE0_RESULT_VALUE = 2u,
  OPENKACHE_CLIENT_GATE0_RESULT_NOT_FOUND = 3u,
  OPENKACHE_CLIENT_GATE0_RESULT_CREATED = 4u,
  OPENKACHE_CLIENT_GATE0_RESULT_REPLACED = 5u,
  OPENKACHE_CLIENT_GATE0_RESULT_DELETED = 6u,
  OPENKACHE_CLIENT_GATE0_RESULT_NOT_DELETED = 7u,
  OPENKACHE_CLIENT_GATE0_RESULT_CONNECTED = 8u,
  OPENKACHE_CLIENT_GATE0_RESULT_UNKNOWN_MUTATION = 12u,
};

enum {
  OPENKACHE_CLIENT_GATE0_STATUS_SUCCESS = 0u,
  OPENKACHE_CLIENT_GATE0_STATUS_NOT_FOUND = 1u,
  OPENKACHE_CLIENT_GATE0_STATUS_MUTATION = 2u,
  OPENKACHE_CLIENT_GATE0_STATUS_ERROR = 3u,
  OPENKACHE_CLIENT_GATE0_STATUS_UNKNOWN_MUTATION = 5u,
};

enum {
  OPENKACHE_CLIENT_GATE0_ERROR_NONE = 0u,
  OPENKACHE_CLIENT_GATE0_ERROR_INVALID_INPUT = 1u,
  OPENKACHE_CLIENT_GATE0_ERROR_CONFIGURATION = 2u,
  OPENKACHE_CLIENT_GATE0_ERROR_TIMEOUT = 3u,
  OPENKACHE_CLIENT_GATE0_ERROR_TRANSPORT = 4u,
  OPENKACHE_CLIENT_GATE0_ERROR_SERVER = 5u,
  OPENKACHE_CLIENT_GATE0_ERROR_PROTOCOL = 6u,
  OPENKACHE_CLIENT_GATE0_ERROR_VALUE = 7u,
  OPENKACHE_CLIENT_GATE0_ERROR_KEY = 8u,
  OPENKACHE_CLIENT_GATE0_ERROR_UNKNOWN_MUTATION = 10u,
  OPENKACHE_CLIENT_GATE0_ERROR_CLOSED = 12u,
  OPENKACHE_CLIENT_GATE0_ERROR_INTERNAL = 13u,
};

enum {
  OPENKACHE_CLIENT_GATE0_KEY_TEXT = 0u,
  OPENKACHE_CLIENT_GATE0_KEY_BYTES = 1u,
  OPENKACHE_CLIENT_GATE0_KEY_INTEGER = 2u,
};

/*
 * Connects with the fixed Gate 0 DevelopmentTrust profile.
 *
 * The address is borrowed for the duration of the call.  The returned result
 * owns either a client handle or an error payload.
 */
openkache_client_result_t *
openkache_client_gate0_connect(const uint8_t *address, size_t address_length);

/*
 * GET, SET, and DELETE consume one canonical StructuredValue key item.  SET
 * additionally consumes one complete StructuredValue-CBOR-v1 value item.
 */
openkache_client_result_t *
openkache_client_gate0_get(const openkache_client_t *client,
                           const uint8_t *canonical_key,
                           size_t canonical_key_length);

openkache_client_result_t *openkache_client_gate0_set(
    const openkache_client_t *client, const uint8_t *canonical_key,
    size_t canonical_key_length, const uint8_t *value, size_t value_length);

openkache_client_result_t *
openkache_client_gate0_delete_value(const openkache_client_t *client,
                                    uint32_t key_kind, const uint8_t *key,
                                    size_t key_length);

void openkache_client_gate0_close(openkache_client_t *client);

uint32_t
openkache_client_gate0_result_kind(const openkache_client_result_t *result);
uint32_t
openkache_client_gate0_result_status(const openkache_client_result_t *result);
uint32_t openkache_client_gate0_result_error_category(
    const openkache_client_result_t *result);
const uint8_t *
openkache_client_gate0_result_data(const openkache_client_result_t *result);
size_t openkache_client_gate0_result_data_length(
    const openkache_client_result_t *result);
openkache_client_t *
openkache_client_gate0_result_take_client(openkache_client_result_t *result);
void openkache_client_gate0_result_free(openkache_client_result_t *result);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* OPENKACHE_CLIENT_GATE0_ABI_H */
