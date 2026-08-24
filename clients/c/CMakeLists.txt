cmake_minimum_required(VERSION 3.25)
project(openkache_client_c VERSION 0.1.0 LANGUAGES C)

include(GNUInstallDirs)
include(CMakePackageConfigHelpers)
find_package(Threads REQUIRED)

set(
  OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER
  ""
  CACHE FILEPATH
  "Optional generated Smithy C contract header; generate it when omitted"
)
set(
  OPENKACHE_CLIENT_BUN_EXECUTABLE
  ""
  CACHE FILEPATH
  "Bun executable used to generate the Smithy C contract when needed"
)

set(
  OPENKACHE_CLIENT_NATIVE_LIBRARY
  ""
  CACHE FILEPATH
  "Legacy single native library path; prefer the explicit shared/static paths"
)
set(
  OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED
  ""
  CACHE FILEPATH
  "Optional shared libopenkache-client-core library built with the ffi feature"
)
set(
  OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC
  ""
  CACHE FILEPATH
  "Optional static libopenkache-client-core library built with the ffi feature"
)
set(
  OPENKACHE_CLIENT_NATIVE_LIBRARY_NAME
  "openkache_client_core"
  CACHE STRING
  "pkg-config library name without the lib prefix or file extension"
)

if(OPENKACHE_CLIENT_NATIVE_LIBRARY)
  if(
    OPENKACHE_CLIENT_NATIVE_LIBRARY MATCHES "\\.(a|lib)$"
    AND NOT OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC
  )
    set(
      OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC
      "${OPENKACHE_CLIENT_NATIVE_LIBRARY}"
    )
  elseif(NOT OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED)
    set(
      OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED
      "${OPENKACHE_CLIENT_NATIVE_LIBRARY}"
    )
  endif()
endif()

foreach(
  OPENKACHE_CLIENT_LIBRARY_PATH
  IN ITEMS
    OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED
    OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC
)
  if(
    NOT "${${OPENKACHE_CLIENT_LIBRARY_PATH}}" STREQUAL ""
    AND NOT EXISTS "${${OPENKACHE_CLIENT_LIBRARY_PATH}}"
  )
    message(
      FATAL_ERROR
      "${OPENKACHE_CLIENT_LIBRARY_PATH} does not exist: "
      "${${OPENKACHE_CLIENT_LIBRARY_PATH}}"
    )
  endif()
endforeach()

set(
  _openkache_client_generated_include_dir
  "${CMAKE_CURRENT_BINARY_DIR}/generated/include"
)
set(
  _openkache_client_generated_contract
  "${_openkache_client_generated_include_dir}/openkache/smithy_contract.h"
)
file(
  GLOB
  _openkache_client_generator_sources
  CONFIGURE_DEPENDS
  "${CMAKE_CURRENT_SOURCE_DIR}/../generator/*.ts"
)
set_property(
  DIRECTORY
  APPEND
  PROPERTY CMAKE_CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/../generate.ts"
    ${_openkache_client_generator_sources}
    "${CMAKE_CURRENT_SOURCE_DIR}/../../protocol/wire.ts"
    "${CMAKE_CURRENT_SOURCE_DIR}/../model"
    "${CMAKE_CURRENT_SOURCE_DIR}/../../protocol/model"
)
file(MAKE_DIRECTORY "${_openkache_client_generated_include_dir}/openkache")

if(OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER)
  if(NOT EXISTS "${OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER}")
    message(
      FATAL_ERROR
      "OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER does not exist: "
      "${OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER}"
    )
  endif()
  configure_file(
    "${OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER}"
    "${_openkache_client_generated_contract}"
    COPYONLY
  )
else()
  if(OPENKACHE_CLIENT_BUN_EXECUTABLE)
    set(
      _openkache_client_bun_executable
      "${OPENKACHE_CLIENT_BUN_EXECUTABLE}"
    )
  else()
    find_program(
      _openkache_client_bun_executable
      NAMES bun
      DOC "Bun executable used to generate the Smithy C contract"
    )
  endif()
  if(NOT _openkache_client_bun_executable)
    message(
      FATAL_ERROR
      "Bun is required to generate the Smithy C contract. Install Bun and "
      "Smithy CLI, or set OPENKACHE_CLIENT_SMITHY_CONTRACT_HEADER to a "
      "generated smithy_contract.h file."
    )
  endif()

  execute_process(
    COMMAND
      "${CMAKE_COMMAND}"
      -E
      env
      "OPENKACHE_GENERATION_TARGET=c-contract"
      "OPENKACHE_C_CONTRACT_OUTPUT=${_openkache_client_generated_contract}"
      "${_openkache_client_bun_executable}"
      "${CMAKE_CURRENT_SOURCE_DIR}/../generate.ts"
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/.."
    RESULT_VARIABLE _openkache_client_generation_result
    OUTPUT_VARIABLE _openkache_client_generation_stdout
    ERROR_VARIABLE _openkache_client_generation_stderr
  )
  if(NOT _openkache_client_generation_result EQUAL 0)
    message(
      FATAL_ERROR
      "Smithy C contract generation failed with exit code "
      "${_openkache_client_generation_result}.\n"
      "${_openkache_client_generation_stdout}"
      "${_openkache_client_generation_stderr}"
    )
  endif()
endif()

set(_openkache_client_system_libs Threads::Threads)
if(CMAKE_DL_LIBS)
  list(APPEND _openkache_client_system_libs ${CMAKE_DL_LIBS})
endif()

# The imported core targets keep the C and C++ adapters header-only while
# carrying the native Rust library and its transitive system link flags.
add_library(OpenKache::ClientCore INTERFACE IMPORTED GLOBAL)
if(OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED)
  add_library(OpenKache::ClientCoreShared UNKNOWN IMPORTED GLOBAL)
  set_target_properties(
    OpenKache::ClientCoreShared
    PROPERTIES
      IMPORTED_LOCATION "${OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED}"
      INTERFACE_LINK_LIBRARIES "${_openkache_client_system_libs}"
  )
  set_property(
    TARGET OpenKache::ClientCore
    APPEND
    PROPERTY INTERFACE_LINK_LIBRARIES OpenKache::ClientCoreShared
  )
endif()
if(OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC)
  add_library(OpenKache::ClientCoreStatic UNKNOWN IMPORTED GLOBAL)
  set_target_properties(
    OpenKache::ClientCoreStatic
    PROPERTIES
      IMPORTED_LOCATION "${OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC}"
      INTERFACE_LINK_LIBRARIES "${_openkache_client_system_libs}"
  )
  if(NOT OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED)
    set_property(
      TARGET OpenKache::ClientCore
      APPEND
      PROPERTY INTERFACE_LINK_LIBRARIES OpenKache::ClientCoreStatic
    )
  endif()
endif()

function(openkache_client_c_configure target)
  target_include_directories(
    "${target}"
    INTERFACE
      $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
      $<INSTALL_INTERFACE:include>
  )
  target_compile_features("${target}" INTERFACE c_std_17)
endfunction()

add_library(openkache_client_gate0 STATIC src/gate0.c)
set_target_properties(
  openkache_client_gate0
  PROPERTIES
    POSITION_INDEPENDENT_CODE ON
    OUTPUT_NAME openkache-client-gate0
    EXPORT_NAME ClientGate0
)
target_include_directories(
  openkache_client_gate0
  PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/../core/include"
    "${_openkache_client_generated_include_dir}"
)
target_compile_features(openkache_client_gate0 PRIVATE c_std_17)
target_link_libraries(openkache_client_gate0 PRIVATE OpenKache::ClientCore)

add_library(openkache_client_c INTERFACE)
add_library(OpenKache::ClientC ALIAS openkache_client_c)
set_target_properties(openkache_client_c PROPERTIES EXPORT_NAME ClientC)
openkache_client_c_configure(openkache_client_c)
target_link_libraries(
  openkache_client_c
  INTERFACE
    openkache_client_gate0
    OpenKache::ClientCore
)

if(TARGET OpenKache::ClientCoreShared)
  add_library(openkache_client_c_shared INTERFACE)
  add_library(OpenKache::ClientCShared ALIAS openkache_client_c_shared)
  set_target_properties(
    openkache_client_c_shared
    PROPERTIES
      EXPORT_NAME ClientCShared
  )
  openkache_client_c_configure(openkache_client_c_shared)
  target_link_libraries(
    openkache_client_c_shared
    INTERFACE
      OpenKache::ClientCoreShared
  )
endif()

if(TARGET OpenKache::ClientCoreStatic)
  add_library(openkache_client_c_static INTERFACE)
  add_library(OpenKache::ClientCStatic ALIAS openkache_client_c_static)
  set_target_properties(
    openkache_client_c_static
    PROPERTIES
      EXPORT_NAME ClientCStatic
  )
  openkache_client_c_configure(openkache_client_c_static)
  target_link_libraries(
    openkache_client_c_static
    INTERFACE
      OpenKache::ClientCoreStatic
  )
endif()

set(_openkache_client_c_export_targets
    openkache_client_gate0 openkache_client_c)
if(TARGET openkache_client_c_shared)
  list(APPEND _openkache_client_c_export_targets openkache_client_c_shared)
endif()
if(TARGET openkache_client_c_static)
  list(APPEND _openkache_client_c_export_targets openkache_client_c_static)
endif()

install(
  TARGETS ${_openkache_client_c_export_targets}
  EXPORT openkache_client_targets
  ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR}
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}
  RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)
install(
  DIRECTORY include/
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)
if(OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED)
  install(
    FILES "${OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED}"
    DESTINATION ${CMAKE_INSTALL_LIBDIR}
  )
  get_filename_component(
    OPENKACHE_CLIENT_INSTALLED_SHARED_NAME
    "${OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED}"
    NAME
  )
else()
  set(OPENKACHE_CLIENT_INSTALLED_SHARED_NAME "")
endif()
if(OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC)
  install(
    FILES "${OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC}"
    DESTINATION ${CMAKE_INSTALL_LIBDIR}
  )
  get_filename_component(
    OPENKACHE_CLIENT_INSTALLED_STATIC_NAME
    "${OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC}"
    NAME
  )
else()
  set(OPENKACHE_CLIENT_INSTALLED_STATIC_NAME "")
endif()
install(
  EXPORT openkache_client_targets
  FILE OpenKacheClientCTargets.cmake
  NAMESPACE OpenKache::
  DESTINATION ${CMAKE_INSTALL_DATADIR}/cmake/OpenKacheClient
)

set(_openkache_client_cmake_install_dir
    "${CMAKE_INSTALL_DATADIR}/cmake/OpenKacheClient")
configure_package_config_file(
  "${CMAKE_CURRENT_SOURCE_DIR}/../cmake/OpenKacheClientConfig.cmake.in"
  "${CMAKE_CURRENT_BINARY_DIR}/OpenKacheClientConfig.cmake"
  INSTALL_DESTINATION "${_openkache_client_cmake_install_dir}"
  PATH_VARS CMAKE_INSTALL_LIBDIR CMAKE_INSTALL_INCLUDEDIR
)
write_basic_package_version_file(
  "${CMAKE_CURRENT_BINARY_DIR}/OpenKacheClientConfigVersion.cmake"
  COMPATIBILITY SameMajorVersion
)
install(
  FILES
    "${CMAKE_CURRENT_BINARY_DIR}/OpenKacheClientConfig.cmake"
    "${CMAKE_CURRENT_BINARY_DIR}/OpenKacheClientConfigVersion.cmake"
  DESTINATION "${_openkache_client_cmake_install_dir}"
)

set(_openkache_client_pkgconfig_dir "${CMAKE_CURRENT_BINARY_DIR}/pkgconfig")
file(MAKE_DIRECTORY "${_openkache_client_pkgconfig_dir}")
set(_openkache_client_pkgconfig_private_libs_list)
if(CMAKE_THREAD_LIBS_INIT)
  list(APPEND _openkache_client_pkgconfig_private_libs_list
       ${CMAKE_THREAD_LIBS_INIT})
endif()
if(CMAKE_DL_LIBS)
  list(APPEND _openkache_client_pkgconfig_private_libs_list ${CMAKE_DL_LIBS})
endif()
string(
  JOIN
  " "
  _openkache_client_pkgconfig_private_libs
  ${_openkache_client_pkgconfig_private_libs_list}
)

if(OPENKACHE_CLIENT_NATIVE_LIBRARY_SHARED)
  set(
    OPENKACHE_CLIENT_PKGCONFIG_LIBRARY_NAME
    "${OPENKACHE_CLIENT_NATIVE_LIBRARY_NAME}"
  )
  set(
    OPENKACHE_CLIENT_PKGCONFIG_LINK_LIBRARIES
    "-L\${libdir} -l${OPENKACHE_CLIENT_PKGCONFIG_LIBRARY_NAME}"
  )
  set(
    OPENKACHE_CLIENT_PKGCONFIG_PRIVATE_LIBS
    "${_openkache_client_pkgconfig_private_libs}"
  )
  configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/../cmake/openkache-client.pc.in"
    "${_openkache_client_pkgconfig_dir}/openkache-client.pc"
    @ONLY
  )
  install(
    FILES "${_openkache_client_pkgconfig_dir}/openkache-client.pc"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/pkgconfig"
  )
endif()
if(OPENKACHE_CLIENT_NATIVE_LIBRARY_STATIC)
  set(
    OPENKACHE_CLIENT_PKGCONFIG_LIBRARY_NAME
    "${OPENKACHE_CLIENT_NATIVE_LIBRARY_NAME}"
  )
  set(
    OPENKACHE_CLIENT_PKGCONFIG_LINK_LIBRARIES
    "\${libdir}/${OPENKACHE_CLIENT_INSTALLED_STATIC_NAME}"
  )
  set(
    OPENKACHE_CLIENT_PKGCONFIG_PRIVATE_LIBS
    "${_openkache_client_pkgconfig_private_libs}"
  )
  configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/../cmake/openkache-client.pc.in"
    "${_openkache_client_pkgconfig_dir}/openkache-client-static.pc"
    @ONLY
  )
  install(
    FILES "${_openkache_client_pkgconfig_dir}/openkache-client-static.pc"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/pkgconfig"
  )
endif()
