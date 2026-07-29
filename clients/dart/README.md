# OpenKache Dart Client

This directory is the Dart package scaffold for a future thin Dart binding to
the OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The Dart package will provide Future-based APIs and FFI resource management
without duplicating protocol or security logic.

## Commands

From `clients/dart`:

```bash
dart analyze
```

## Components

- `pubspec.yaml` defines package metadata and supported SDK versions.
- `lib/openkache.dart` reserves the public library entry point.

## Configuration

Flutter support, native asset bundling, isolate usage, and runtime options will
be defined when the binding is implemented.
