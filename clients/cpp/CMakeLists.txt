cmake_minimum_required(VERSION 3.25)
project(openkache_client_cpp VERSION 0.1.0 LANGUAGES CXX)

include(GNUInstallDirs)

if(NOT TARGET OpenKache::ClientC)
  add_subdirectory(
    "${CMAKE_CURRENT_SOURCE_DIR}/../c"
    "${CMAKE_CURRENT_BINARY_DIR}/openkache_client_c"
  )
endif()

add_library(openkache_client_cpp INTERFACE)
add_library(OpenKache::ClientCpp ALIAS openkache_client_cpp)
set_target_properties(openkache_client_cpp PROPERTIES EXPORT_NAME ClientCpp)
target_include_directories(
  openkache_client_cpp
  INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../c/include>
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/../core/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(openkache_client_cpp INTERFACE cxx_std_20)
target_link_libraries(openkache_client_cpp INTERFACE OpenKache::ClientC)

install(
  TARGETS openkache_client_cpp
  EXPORT openkache_client_cpp_targets
)
install(
  DIRECTORY include/
  DESTINATION ${CMAKE_INSTALL_INCLUDEDIR}
)
install(
  EXPORT openkache_client_cpp_targets
  NAMESPACE OpenKache::
  DESTINATION ${CMAKE_INSTALL_LIBDIR}/cmake/openkache-client
)
