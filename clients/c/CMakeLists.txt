cmake_minimum_required(VERSION 3.25)
project(openkache_client_c VERSION 0.1.0 LANGUAGES C)

include(GNUInstallDirs)

set(
  OPENKACHE_CLIENT_NATIVE_LIBRARY
  ""
  CACHE FILEPATH
  "Optional libopenkache-client-core native library built with the ffi feature"
)

add_library(openkache_client_c INTERFACE)
add_library(OpenKache::ClientC ALIAS openkache_client_c)
set_target_properties(openkache_client_c PROPERTIES EXPORT_NAME ClientC)
target_include_directories(
  openkache_client_c
  INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../core/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(openkache_client_c INTERFACE c_std_17)

if(OPENKACHE_CLIENT_NATIVE_LIBRARY)
  if(NOT EXISTS "${OPENKACHE_CLIENT_NATIVE_LIBRARY}")
    message(
      FATAL_ERROR
      "OPENKACHE_CLIENT_NATIVE_LIBRARY does not exist: ${OPENKACHE_CLIENT_NATIVE_LIBRARY}"
    )
  endif()
  target_link_libraries(
    openkache_client_c
    INTERFACE
      "${OPENKACHE_CLIENT_NATIVE_LIBRARY}"
      ${CMAKE_DL_LIBS}
  )
endif()

install(
  TARGETS openkache_client_c
  EXPORT openkache_client_targets
)
install(
  DIRECTORY include/
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)
install(
  FILES
    "${CMAKE_CURRENT_SOURCE_DIR}/../core/include/openkache/client_abi.h"
    "${CMAKE_CURRENT_SOURCE_DIR}/../core/include/openkache/smithy_contract.h"
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}/openkache
)
install(
  FILES "${CMAKE_CURRENT_SOURCE_DIR}/../core/include/openkache_client.h"
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)
install(
  EXPORT openkache_client_targets
  NAMESPACE OpenKache::
  DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/openkache-client
)
