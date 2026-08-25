#include "../../core/include/openkache/client_abi.h"
#include "../include/openkache/client_abi.h"

#include <stdint.h>

/*
 * This translation unit is package-private.  It consumes the generated
 * shared-core ABI while the maintained Gate 0 declarations remain the public
 * entry point; CMake installs both in one self-contained include tree.
 */
#include <openkache/smithy_contract.h>

/*
 * This admission-aware wait is intentionally package-private.  It is not
 * part of the generated public ABI because Gate 0 hides native cancellation
 * and request handles from maintained callers.
 */
extern openkache_client_result_t *openkache_client_request_wait_mutation(
    openkache_client_request_t *request, uint64_t timeout_ms);
extern openkache_client_request_t *
openkache_client_execute_structured_fields_async(
    const openkache_client_t *client, uint32_t operation,
    const openkache_client_operation_field_t *fields, size_t field_count);

_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_ERROR ==
                   OPENKACHE_SMITHY_FFI_RESULT_ERROR,
               "Gate 0 result constants drifted from Smithy");
_Static_assert(OPENKACHE_CLIENT_GATE0_RESULT_OK ==
                   OPENKACHE_SMITHY_FFI_RESULT_OK,
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
_Static_assert(OPENKACHE_SMITHY_GATE0_ALPN_VERSION ==
                   OPENKACHE_SMITHY_VALUE_FORMAT_VERSION,
               "Gate 0 protocol versions drifted from Smithy");
_Static_assert(OPENKACHE_SMITHY_GATE0_NAMESPACE_ID > 0,
               "Gate 0 namespace identity must be positive");
_Static_assert(OPENKACHE_CLIENT_GATE0_NAMESPACE_ID ==
                   OPENKACHE_SMITHY_GATE0_NAMESPACE_ID,
               "Gate 0 namespace identity drifted from Smithy");
_Static_assert(OPENKACHE_SMITHY_GATE0_VALUE_SELECTOR ==
                   (OPENKACHE_SMITHY_GATE0_ENCRYPTION |
                    (OPENKACHE_SMITHY_GATE0_COMPRESSION << 2u) |
                    (OPENKACHE_SMITHY_VALUE_SERIALIZATION_STRUCTURED << 4u)),
               "Gate 0 value selector drifted from Smithy");

static const uint8_t gate0_item_id_root
    [OPENKACHE_SMITHY_GATE0_ITEM_ID_ROOT_KEY_LENGTH] = {
        OPENKACHE_SMITHY_GATE0_ITEM_ID_ROOT_KEY_BYTES,
};

/*
 * Gate 0 derives Item IDs only after resolving the empty default namespace.
 * The shared core stores the descriptor ID for subsequent structured
 * operations, so reject a server that selected anything other than the
 * generated Gate 0 identity before the caller's key reaches that path.
 *
 * The generated ABI currently has no result-construction entry point.  The
 * zero namespace passed to execute_scoped intentionally exercises its normal
 * owned error-result path when the descriptor is malformed or incompatible.
 */
static openkache_client_result_t *
gate0_namespace_preflight(const openkache_client_t *client) {
  openkache_client_result_t *namespace_result =
      openkache_client_namespace_open(
          client, NULL, 0, OPENKACHE_SMITHY_OPEN_CREATE_IF_MISSING, 0, 0);
  if (namespace_result == NULL) {
    return NULL;
  }

  const uint32_t result_kind =
      openkache_client_result_kind(namespace_result);
  if (result_kind != OPENKACHE_CLIENT_GATE0_RESULT_OK &&
      result_kind != OPENKACHE_CLIENT_GATE0_RESULT_CREATED) {
    return namespace_result;
  }

  openkache_client_namespace_descriptor_t descriptor = {0};
  const uint8_t *payload =
      openkache_client_result_data(namespace_result);
  const size_t payload_length =
      openkache_client_result_data_length(namespace_result);
  const uint32_t decode_status = openkache_client_namespace_descriptor_decode(
      payload, payload_length, &descriptor);
  const int namespace_matches =
      decode_status == OPENKACHE_SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_OK &&
      descriptor.namespace_id == OPENKACHE_CLIENT_GATE0_NAMESPACE_ID;
  openkache_client_result_free(namespace_result);
  if (namespace_matches) {
    return NULL;
  }

  return openkache_client_execute_scoped(
      client, OPENKACHE_SMITHY_OPCODE_GET, 0, NULL, 0, NULL, 0, 0, 0);
}

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
      .compression_enabled = OPENKACHE_SMITHY_GATE0_COMPRESSION,
      .compression_level = OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_LEVEL,
      .minimum_input_size =
          OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
      .minimum_savings =
          OPENKACHE_SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
      .encryption = OPENKACHE_SMITHY_GATE0_ENCRYPTION,
      .connect_timeout_ms =
          OPENKACHE_SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
      .request_timeout_ms =
          OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
      .retry_max_attempts = OPENKACHE_SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
      .max_in_flight = OPENKACHE_SMITHY_DEFAULT_MAX_IN_FLIGHT,
  };
  openkache_client_connect_options_with_keyring_t keyring = {
      .abi_version = OPENKACHE_CLIENT_ABI_VERSION,
      .base = &base,
      .item_id_root_key = gate0_item_id_root,
      .item_id_root_key_length = sizeof(gate0_item_id_root),
      .value_keys = NULL,
      .value_key_count = 0,
      .active_value_key_id = OPENKACHE_SMITHY_GATE0_ENCRYPTION,
      .value_encryption = OPENKACHE_SMITHY_GATE0_ENCRYPTION,
  };
  return openkache_client_connect_with_keyring_options_transport(
      &keyring, OPENKACHE_SMITHY_FFI_TRANSPORT_QUIC_INSECURE);
}

openkache_client_result_t *
openkache_client_gate0_get(const openkache_client_t *client,
                           const uint8_t *canonical_key,
                           size_t canonical_key_length) {
  openkache_client_result_t *namespace_error =
      gate0_namespace_preflight(client);
  if (namespace_error != NULL) {
    return namespace_error;
  }
  const openkache_client_operation_field_t field = {
      .data = canonical_key,
      .length = canonical_key_length,
      .present = 1,
  };
  return openkache_client_execute_fields(
      client, OPENKACHE_SMITHY_OPCODE_GET, &field,
      OPENKACHE_SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE);
}

openkache_client_result_t *openkache_client_gate0_set(
    const openkache_client_t *client, const uint8_t *canonical_key,
    size_t canonical_key_length, const uint8_t *value, size_t value_length) {
  openkache_client_result_t *namespace_error =
      gate0_namespace_preflight(client);
  if (namespace_error != NULL) {
    return namespace_error;
  }
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
  openkache_client_request_t *request =
      openkache_client_execute_structured_fields_async(
          client, OPENKACHE_SMITHY_OPCODE_SET, fields,
          OPENKACHE_SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE + 1u);
  if (request == NULL) {
    return NULL;
  }
  openkache_client_result_t *result = openkache_client_request_wait_mutation(
      request, OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS);
  openkache_client_request_free(request);
  return result;
}

openkache_client_result_t *
openkache_client_gate0_delete_value(const openkache_client_t *client,
                                    const uint8_t *canonical_key,
                                    size_t canonical_key_length) {
  openkache_client_result_t *namespace_error =
      gate0_namespace_preflight(client);
  if (namespace_error != NULL) {
    return namespace_error;
  }
  const openkache_client_operation_field_t fields[1] = {
      {
          .data = canonical_key,
          .length = canonical_key_length,
          .present = 1,
      },
  };
  openkache_client_request_t *request =
      openkache_client_execute_structured_fields_async(
          client, OPENKACHE_SMITHY_OPCODE_DELETE, fields,
          OPENKACHE_SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE);
  if (request == NULL) {
    return NULL;
  }
  openkache_client_result_t *result = openkache_client_request_wait_mutation(
      request, OPENKACHE_SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS);
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
