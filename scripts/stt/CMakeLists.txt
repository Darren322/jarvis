cmake_minimum_required(VERSION 3.16)
project(jarvis_stt LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
if(NOT CMAKE_BUILD_TYPE)
  set(CMAKE_BUILD_TYPE Release)
endif()
add_compile_options(-Wall -Wextra)

# Pure logic and its tests: no HailoRT, no network. Builds on any machine (`ctest`).
add_library(stt_logic STATIC stt_logic.cpp)
enable_testing()
add_executable(stt_logic_test stt_logic_test.cpp)
target_link_libraries(stt_logic_test PRIVATE stt_logic)
add_test(NAME stt_logic COMMAND stt_logic_test)

# The server needs HailoRT. Pass -DJARVIS_STT_BUILD_SERVER=OFF to build only the tests elsewhere.
option(JARVIS_STT_BUILD_SERVER "Build the jarvis-stt server (needs HailoRT)" ON)
if(JARVIS_STT_BUILD_SERVER)
  find_package(HailoRT 5.5.0 REQUIRED)
  find_package(Threads REQUIRED)

  # cpp-httplib is not committed: fetch the pinned release and verify its hash (plain HTTP, no OpenSSL).
  set(HTTPLIB_VERSION 0.65.0)
  set(HTTPLIB_SHA256 17d85dcf129d3cc67eefba846608016f43882c9de7fae811a930f9736a10eeaa)
  set(HTTPLIB_DIR ${CMAKE_BINARY_DIR}/cpp-httplib-${HTTPLIB_VERSION})
  file(DOWNLOAD
    https://raw.githubusercontent.com/yhirose/cpp-httplib/v${HTTPLIB_VERSION}/httplib.h
    ${HTTPLIB_DIR}/httplib.h
    EXPECTED_HASH SHA256=${HTTPLIB_SHA256}
    TLS_VERIFY ON)

  add_executable(jarvis-stt main.cpp)
  target_include_directories(jarvis-stt SYSTEM PRIVATE ${HTTPLIB_DIR})
  target_link_libraries(jarvis-stt PRIVATE stt_logic HailoRT::libhailort Threads::Threads)
endif()
