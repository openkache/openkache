# OpenKache JavaScript Client

This directory is the package scaffold for a future JavaScript binding to the
OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The package will expose an idiomatic JavaScript API without duplicating the
Rust implementation of transport, framing, compression, or encryption.

## Commands

From `clients/javascript`:

```bash
bun install --frozen-lockfile
bun run verify
```

## Components

- `package.json` defines the ESM package and validation command.
- `src/index.js` reserves the public module entry point.

## Configuration

Supported runtimes, native library packaging, and discovery rules will be
defined when the binding is implemented. Bun TypeScript users can use the
implemented client under `../typescript/` today.
