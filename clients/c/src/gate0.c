#include "../../core/include/openkache/client_abi.h"
#include "../include/openkache/client_abi.h"

#include <stdint.h>

/*
 * This translation unit is package-private.  It is the only place where the
 * generated shared-core ABI is visible; installed headers expose only the
 * maintained Gate 0 declarations from clients/c/include.
 */
#include <openkache/smithy_contract.h>

_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_ERROR ==
                   OPENKACHE_SMITHY_FFI_RESULT_ERROR,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_VALUE ==
                   OPENKACHE_SMITHY_FFI_RESULT_VALUE,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_NOT_FOUND ==
                   OPENKACHE_SMITHY_FFI_RESULT_NOT_FOUND,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_CREATED ==
                   OPENKACHE_SMITHY_FFI_RESULT_CREATED,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_REPLACED ==
                   OPENKACHE_SMITHY_FFI_RESULT_REPLACED,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_DELETED ==
                   OPENKACHE_SMITHY_FFI_RESULT_DELETED,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_NOT_DELETED ==
                   OPENKACHE_SMITHY_FFI_RESULT_NOT_DELETED,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_CONNECTED ==
                   OPENKACHE_SMITHY_FFI_RESULT_CONNECTED,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_UNKNOWN_MUTATION ==
                   OPENKACHE_SMITHY_FFI_RESULT_UNKNOWN_MUTATION,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_KEY_TEXT ==
                   OPENKACHE_SMITHY_FFI_KEY_SPEC_TEXT,
               "Gate 0 key constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_KEY_BYTES ==
                   OPENKACHE_SMITHY_FFI_KEY_SPEC_BYTES,
               "Gate 0 key constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_KEY_INTEGER ==
                   OPENKACHE_SMITHY_FFI_KEY_SPEC_INTEGER,
               "Gate 0 key constants drifted from Smithy");

static const uint8_t GATE0_ITEM_ID_ROOT[32] = {
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
    0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15,
    0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
};

openkache_client_result_t *
openkache_client_gate0_connect(const uint8_t *address, size_t address_length) {
  openkache_client_connect_options_t base = {
      .address = address,
      .address_length = address_length,
      .server_name = NULL,
      .server_name_length = 0,
      .certificate = NULL,
      .certificate_length = 0,
      .client_certificate_chain = NULL,
      .client_certificate_chain_length = 0,
      .client_private_key = NULL,
      .client_private_key_length = 0,
      .data_protection_key = NULL,
      .data_protection_key_length = 0,
      .compression_enabled = 0,
      .compression_level = OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL,
      .minimum_input_size = 0,
      .minimum_savings = 0,
      .encryption = OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE,
      .connect_timeout_ms =
          OPENKACHE_SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
      .request_timeout_ms =
          OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
      .retry_max_attempts = 0,
      .max_in_flight = 0,
  };
  openkache_client_connect_options_with_keyring_t keyring = {
      .abi_version = OPENKACHE_CLIENT_ABI_VERSION,
      .base = &base,
      .item_id_root_key = GATE0_ITEM_ID_ROOT,
      .item_id_root_key_length = sizeof(GATE0_ITEM_ID_ROOT),
      .value_keys = NULL,
      .value_key_count = 0,
      .active_value_key_id = 0,
      .value_encryption = OPENKACHE_SMITHY_VALUE_ENCRYPTION_NONE,
  };
  return openkache_client_connect_with_keyring_options_transport(
      &keyring, OPENKACHE_SMITHY_FFI_TRANSPORT_QUIC_INSECURE);
}

openkache_client_result_t *
openkache_client_gate0_get(const openkache_client_t *client,
                           const uint8_t *canonical_key,
                           size_t canonical_key_length) {
  const openkache_client_operation_field_t field = {
      .data = canonical_key,
      .length = canonical_key_length,
      .present = 1,
  };
  return openkache_client_execute_fields(client, OPENKACHE_SMITHY_OPCODE_GET,
                                         &field, 1);
}

openkache_client_result_t *openkache_client_gate0_set(
    const openkache_client_t *client, const uint8_t *canonical_key,
    size_t canonical_key_length, const uint8_t *value, size_t value_length) {
  const openkache_client_operation_field_t fields[2] = {
      {
          .data = canonical_key,
          .length = canonical_key_length,
          .present = 1,
      },
      {
          .data = value,
          .length = value_length,
          .present = 1,
      },
  };
  return openkache_client_execute_fields(client, OPENKACHE_SMITHY_OPCODE_SET,
                                         fields, 2);
}

openkache_client_result_t *
openkache_client_gate0_delete_value(const openkache_client_t *client,
                                    uint32_t key_kind, const uint8_t *key,
                                    size_t key_length) {
  openkache_client_request_t *request =
      openkache_client_execute_with_options_async(
          client, OPENKACHE_SMITHY_OPCODE_DELETE, key_kind, key, key_length,
          NULL, 0, 0, 0);
  if (request == NULL) {
    return NULL;
  }
  while (openkache_client_request_poll(request) ==
         OPENKACHE_SMITHY_FFI_REQUEST_STATE_PENDING) {
  }
  openkache_client_result_t *result = openkache_client_request_wait(request, 0);
  openkache_client_request_free(request);
  return result;
}

void openkache_client_gate0_close(openkache_client_t *client) {
  openkache_client_free(client);
}

uint32_t
openkache_client_gate0_result_kind(const openkache_client_result_t *result) {
  return openkache_client_result_kind(result);
}

uint32_t
openkache_client_gate0_result_status(const openkache_client_result_t *result) {
  return openkache_client_result_status(result);
}

uint32_t openkache_client_gate0_result_error_category(
    const openkache_client_result_t *result) {
  return openkache_client_result_error_category(result);
}

const uint8_t *
openkache_client_gate0_result_data(const openkache_client_result_t *result) {
  return openkache_client_result_data(result);
}

size_t openkache_client_gate0_result_data_length(
    const openkache_client_result_t *result) {
  return openkache_client_result_data_length(result);
}

openkache_client_t *
openkache_client_gate0_result_take_client(openkache_client_result_t *result) {
  return openkache_client_result_take_client(result);
}

void openkache_client_gate0_result_free(openkache_client_result_t *result) {
  openkache_client_result_free(result);
}
