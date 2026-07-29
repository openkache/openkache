cmake_minimum_required(VERSION 3.25)
project(openkache_client_cpp VERSION 0.1.0 LANGUAGES CXX)

add_library(openkache_client_cpp INTERFACE)
add_library(OpenKache::ClientCpp ALIAS openkache_client_cpp)
target_include_directories(
  openkache_client_cpp
  INTERFACE
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
    $<INSTALL_INTERFACE:include>
)
target_compile_features(openkache_client_cpp INTERFACE cxx_std_20)
