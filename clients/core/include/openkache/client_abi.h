#ifndef OPENKACHE_CLIENT_ABI_H
#define OPENKACHE_CLIENT_ABI_H

/*
 * Public compatibility include for the native client ABI.
 *
 * The Smithy generator owns every ABI constant, declaration, enum, and
 * structure in smithy_contract.h.  This shim intentionally contains no
 * duplicated numeric values or function prototypes: package builds place the
 * generated header on the include path before installing this compatibility
 * include.
 */
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

#endif /* OPENKACHE_CLIENT_ABI_H */
