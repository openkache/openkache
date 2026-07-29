# OpenKache Python Client

This directory is the package scaffold for a future thin Python binding to the
OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The Python package will provide idiomatic asynchronous APIs while delegating
transport, framing, compression, and encryption to the Rust core.

## Commands

From `clients/python`:

```bash
python -m compileall src
python -m build
```

## Components

- `pyproject.toml` defines package and registry metadata.
- `src/openkache/__init__.py` reserves the import package.
- `src/openkache/py.typed` marks the future package as typed.

## Configuration

Native library discovery and runtime options will be defined when the binding
is implemented.
