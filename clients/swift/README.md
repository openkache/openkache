# OpenKache Swift Client

This directory is the Swift Package scaffold for a future thin Swift binding to
the OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The package will provide Swift concurrency and ownership conventions while the
Rust core owns network and value behavior.

## Commands

From `clients/swift`:

```bash
swift build
```

## Components

- `Package.swift` defines the Swift package and library product.
- `Sources/OpenKache/OpenKache.swift` reserves the module entry point.

## Configuration

Supported Apple platforms, binary targets, native artifact signing, and runtime
options will be selected when the binding is implemented.
