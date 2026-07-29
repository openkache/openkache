# OpenKache C++ Client

This directory is the C++ package scaffold for a future thin, RAII-style
wrapper over the OpenKache Rust client ABI. It intentionally contains no
connection or cache operation implementation.

## Purpose

The C++ layer will own typed views, futures, errors, and handle lifetime only.
Rust will continue to own all protocol and security behavior.

## Commands

From `clients/cpp`:

```bash
cmake -S . -B target/build
cmake --build target/build
```

## Components

- `CMakeLists.txt` defines an installable interface target.
- `include/openkache/client.hpp` reserves the public include path.

## Configuration

Executor integration, exception policy, native library discovery, and binary
packaging will be defined when the binding is implemented.
