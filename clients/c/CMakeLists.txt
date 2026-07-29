cmake_minimum_required(VERSION 3.25)
project(openkache_client_c VERSION 0.1.0 LANGUAGES C)

add_library(openkache_client_c INTERFACE)
add_library(OpenKache::ClientC ALIAS openkache_client_c)
target_include_directories(
  openkache_client_c
  INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(openkache_client_c INTERFACE c_std_17)
