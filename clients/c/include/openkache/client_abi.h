#ifndef OPENKACHE_CLIENT_GATE0_ABI_H
#define OPENKACHE_CLIENT_GATE0_ABI_H

/*
 * Maintained Gate 0 C ABI.
 *
 * The Smithy-generated contract owns every shared discriminator and Gate 0
 * profile constant.  CMake places that projection on the build and install
 * include paths; this header aliases only the maintained result/key surface.
 */

#include <stddef.h>
#include <stdint.h>

#if defined(__has_include)
#  if __has_include(<openkache/smithy_contract.h>)
#    include <openkache/smithy_contract.h>
#  elif __has_include("smithy_contract.h")
#    include "smithy_contract.h"
#  else
#    error "generated openkache/smithy_contract.h is required"
#  endif
#else
#  include "smithy_contract.h"
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct openkache_client openkache_client_t;
typedef struct openkache_client_result openkache_client_result_t;

enum {
  OPENKACHE_CLIENT_GATE0_RESULT_ERROR = OPENKACHE_SMITHY_FFI_RESULT_ERROR,
  OPENKACHE_CLIENT_GATE0_RESULT_OK = OPENKACHE_SMITHY_FFI_RESULT_OK,
  OPENKACHE_CLIENT_GATE0_RESULT_VALUE = OPENKACHE_SMITHY_FFI_RESULT_VALUE,
  OPENKACHE_CLIENT_GATE0_RESULT_NOT_FOUND =
      OPENKACHE_SMITHY_FFI_RESULT_NOT_FOUND,
  OPENKACHE_CLIENT_GATE0_RESULT_CREATED = OPENKACHE_SMITHY_FFI_RESULT_CREATED,
  OPENKACHE_CLIENT_GATE0_RESULT_REPLACED = OPENKACHE_SMITHY_FFI_RESULT_REPLACED,
  OPENKACHE_CLIENT_GATE0_RESULT_DELETED = OPENKACHE_SMITHY_FFI_RESULT_DELETED,
  OPENKACHE_CLIENT_GATE0_RESULT_NOT_DELETED =
      OPENKACHE_SMITHY_FFI_RESULT_NOT_DELETED,
  OPENKACHE_CLIENT_GATE0_RESULT_CONNECTED =
      OPENKACHE_SMITHY_FFI_RESULT_CONNECTED,
  OPENKACHE_CLIENT_GATE0_RESULT_UNKNOWN_MUTATION =
      OPENKACHE_SMITHY_FFI_RESULT_UNKNOWN_MUTATION,
  OPENKACHE_CLIENT_GATE0_RESULT_RESOURCE_EXHAUSTED =
      OPENKACHE_SMITHY_FFI_RESULT_RESOURCE_EXHAUSTED,
};

enum {
  OPENKACHE_CLIENT_GATE0_NAMESPACE_ID =
      OPENKACHE_SMITHY_GATE0_NAMESPACE_ID,
};

enum {
  OPENKACHE_CLIENT_GATE0_STATUS_SUCCESS =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_SUCCESS,
  OPENKACHE_CLIENT_GATE0_STATUS_NOT_FOUND =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_NOT_FOUND,
  OPENKACHE_CLIENT_GATE0_STATUS_MUTATION =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_MUTATION,
  OPENKACHE_CLIENT_GATE0_STATUS_ERROR =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_ERROR,
  OPENKACHE_CLIENT_GATE0_STATUS_UNKNOWN_MUTATION =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_UNKNOWN_MUTATION,
  OPENKACHE_CLIENT_GATE0_STATUS_RESOURCE_EXHAUSTED =
      OPENKACHE_SMITHY_FFI_STATUS_CATEGORY_RESOURCE_EXHAUSTED,
};

enum {
  OPENKACHE_CLIENT_GATE0_ERROR_NONE =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_NONE,
  OPENKACHE_CLIENT_GATE0_ERROR_INVALID_INPUT =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_INVALID_INPUT,
  OPENKACHE_CLIENT_GATE0_ERROR_CONFIGURATION =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_CONFIGURATION,
  OPENKACHE_CLIENT_GATE0_ERROR_TIMEOUT =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_TIMEOUT,
  OPENKACHE_CLIENT_GATE0_ERROR_TRANSPORT =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_TRANSPORT,
  OPENKACHE_CLIENT_GATE0_ERROR_SERVER =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_SERVER,
  OPENKACHE_CLIENT_GATE0_ERROR_PROTOCOL =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_PROTOCOL,
  OPENKACHE_CLIENT_GATE0_ERROR_VALUE =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_VALUE,
  OPENKACHE_CLIENT_GATE0_ERROR_KEY =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_KEY,
  OPENKACHE_CLIENT_GATE0_ERROR_UNKNOWN_MUTATION =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_UNKNOWN_MUTATION,
  OPENKACHE_CLIENT_GATE0_ERROR_RESOURCE_EXHAUSTED =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_RESOURCE_EXHAUSTED,
  OPENKACHE_CLIENT_GATE0_ERROR_CLOSED =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_CLOSED,
  OPENKACHE_CLIENT_GATE0_ERROR_INTERNAL =
      OPENKACHE_SMITHY_FFI_ERROR_CATEGORY_INTERNAL,
};

enum {
  OPENKACHE_CLIENT_GATE0_KEY_TEXT = OPENKACHE_SMITHY_FFI_KEY_SPEC_TEXT,
  OPENKACHE_CLIENT_GATE0_KEY_BYTES = OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
  OPENKACHE_CLIENT_GATE0_KEY_INTEGER = OPENKACHE_SMITHY_FFI_KEY_SPEC_INTEGER,
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
                                    const uint8_t *canonical_key,
                                    size_t canonical_key_length);

/*
 * Explicitly closes and frees a Gate 0 client.
 *
 * The caller must ensure no operation is using the handle, call this function
 * exactly once for each non-null handle, and discard the handle afterwards.
 * C has no destructor or finalizer, so omitting this call leaks native
 * resources; callers must not rely on process teardown as a lifecycle boundary.
 */
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
