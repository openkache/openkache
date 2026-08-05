#ifndef OPENKACHE_CLIENT_H
#define OPENKACHE_CLIENT_H

/*
 * Source-tree compatibility include. The canonical C ABI is shared by all
 * native adapters in clients/core/include/openkache/client_abi.h.
 */

#if defined(__has_include)
#  if __has_include(<openkache/client_abi.h>)
#    include <openkache/client_abi.h>
#  elif __has_include("../../../core/include/openkache/client_abi.h")
#    include "../../../core/include/openkache/client_abi.h"
#  endif
#else
#  include "../../../core/include/openkache/client_abi.h"
#endif

/*
 * Convenience wrapper for the experimental ECHO operation. The returned
 * result is owned by the caller and must be released with
 * openkache_client_result_free().
 */
static inline openkache_client_result_t *openkache_client_echo(
    const openkache_client_t *client,
    const uint8_t *value,
    size_t value_length)
{
    return openkache_client_execute(
        client,
        OPENKACHE_CLIENT_OPERATION_ECHO,
        NULL,
        0,
        value,
        value_length,
        OPENKACHE_CLIENT_SET_CONDITION_ANY,
        0,
        0);
}

#endif /* OPENKACHE_CLIENT_H */
