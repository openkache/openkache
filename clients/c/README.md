# OpenKache C Client

This directory is the C package scaffold for the stable ABI exported by the
OpenKache Rust client. It intentionally contains no wrapper implementation.

## Purpose

C is the lowest-level consumer-facing binding. Its eventual header will expose
opaque handles and byte-oriented operations while Rust retains ownership of
transport, protocol, compression, and encryption.

## Commands

From `clients/c`:

```bash
cmake -S . -B target/build
cmake --build target/build
```

## Components

- `CMakeLists.txt` defines an installable interface target.
- `include/openkache/client.h` reserves the public include path.

## Configuration

Library names, symbol visibility, static versus dynamic linkage, and ABI header
contents will be finalized with the native packaging workflow.
