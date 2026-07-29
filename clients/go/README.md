# OpenKache Go Client

This directory is the module scaffold for a future thin Go binding to the
OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The Go module will provide context-aware APIs and deterministic native resource
cleanup while keeping protocol and security behavior in Rust.

## Commands

From `clients/go`:

```bash
go vet ./...
go build ./...
```

## Components

- `go.mod` defines the Go module.
- `doc.go` reserves the public package and documents the binding boundary.

## Configuration

CGO linkage, distributed native artifacts, and runtime options will be defined
when the binding is implemented.
