#ifndef OPENKACHE_CLIENT_ABI_H
#define OPENKACHE_CLIENT_ABI_H

/*
 * Stable C ABI include retained for source compatibility.
 *
 * The Smithy generator emits the complete ABI into smithy_contract.h:
 * opaque handles, compatibility aliases, the connect-options layout, all
 * exported function declarations, and dynamic-loader function-pointer types.
 */

#if defined(__has_include)
#  if __has_include(<openkache/smithy_contract.h>)
#    include <openkache/smithy_contract.h>
#  else
#    include "smithy_contract.h"
#  endif
#else
#  include "smithy_contract.h"
#endif

#endif /* OPENKACHE_CLIENT_ABI_H */
