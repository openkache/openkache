cmake_minimum_required(VERSION 3.25)
project(openkache_client_cpp VERSION 0.1.0 LANGUAGES CXX)

include(GNUInstallDirs)

set(_openkache_client_cpp_adds_c FALSE)
if(NOT TARGET OpenKache::ClientC)
  set(_openkache_client_cpp_adds_c TRUE)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../c"
    "${CMAKE_CURRENT_BINARY_DIR}/openkache_client_c"
  )
endif()

function(openkache_client_cpp_require_cxx20 target)
  target_compile_features("${target}" INTERFACE cxx_std_20)
  set_target_properties(
    "${target}"
    PROPERTIES
      CXX_EXTENSIONS OFF
      CXX_STANDARD_REQUIRED ON
  )
endfunction()

function(openkache_client_cpp_configure target)
  target_include_directories(
    "${target}"
    INTERFACE
      $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
      $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../c/include>
      $<INSTALL_INTERFACE:include>
  )
  openkache_client_cpp_require_cxx20("${target}")
endfunction()

add_library(openkache_client_cpp INTERFACE)
add_library(OpenKache::ClientCpp ALIAS openkache_client_cpp)
set_target_properties(openkache_client_cpp PROPERTIES EXPORT_NAME ClientCpp)
openkache_client_cpp_configure(openkache_client_cpp)
target_link_libraries(openkache_client_cpp INTERFACE OpenKache::ClientC)

add_library(openkache_client INTERFACE)
add_library(OpenKache::Client ALIAS openkache_client)
set_target_properties(openkache_client PROPERTIES EXPORT_NAME Client)
target_link_libraries(openkache_client INTERFACE openkache_client_cpp)

if(TARGET OpenKache::ClientCShared)
  add_library(openkache_client_cpp_shared INTERFACE)
  add_library(OpenKache::ClientCppShared ALIAS openkache_client_cpp_shared)
  set_target_properties(
    openkache_client_cpp_shared
    PROPERTIES
      EXPORT_NAME ClientCppShared
  )
  openkache_client_cpp_configure(openkache_client_cpp_shared)
  target_link_libraries(
    openkache_client_cpp_shared
    INTERFACE
      OpenKache::ClientCShared
  )
endif()

if(TARGET OpenKache::ClientCStatic)
  add_library(openkache_client_cpp_static INTERFACE)
  add_library(OpenKache::ClientCppStatic ALIAS openkache_client_cpp_static)
  set_target_properties(
    openkache_client_cpp_static
    PROPERTIES
      EXPORT_NAME ClientCppStatic
  )
  openkache_client_cpp_configure(openkache_client_cpp_static)
  target_link_libraries(
    openkache_client_cpp_static
    INTERFACE
      OpenKache::ClientCStatic
  )
endif()

set(_openkache_client_cpp_export_targets openkache_client_cpp openkache_client)
if(TARGET openkache_client_cpp_shared)
  list(APPEND _openkache_client_cpp_export_targets openkache_client_cpp_shared)
endif()
if(TARGET openkache_client_cpp_static)
  list(APPEND _openkache_client_cpp_export_targets openkache_client_cpp_static)
endif()

install(
  TARGETS ${_openkache_client_cpp_export_targets}
  EXPORT openkache_client_cpp_targets
)
install(
  DIRECTORY include/
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)
install(
  FILES
    "${CMAKE_CURRENT_SOURCE_DIR}/../LICENSE"
    "${CMAKE_CURRENT_SOURCE_DIR}/README.md"
  DESTINATION "${CMAKE_INSTALL_DATADIR}/doc/OpenKacheClient"
)
if(OPENKACHE_CLIENT_THIRD_PARTY_NOTICES AND NOT _openkache_client_cpp_adds_c)
  install(
    FILES "${OPENKACHE_CLIENT_THIRD_PARTY_NOTICES}"
    DESTINATION "${CMAKE_INSTALL_DATADIR}/doc/OpenKacheClient"
  )
endif()
install(
  EXPORT openkache_client_cpp_targets
  FILE OpenKacheClientCppTargets.cmake
  NAMESPACE OpenKache::
  DESTINATION ${CMAKE_INSTALL_DATADIR}/cmake/OpenKacheClient
)
