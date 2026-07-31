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

#endif /* OPENKACHE_CLIENT_H */
