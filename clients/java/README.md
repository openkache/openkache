# OpenKache Java Client

This directory is the Maven package scaffold for a future thin Java binding to
the OpenKache Rust client ABI. It intentionally contains no connection or cache
operation implementation.

## Purpose

The Java package will provide asynchronous JVM APIs and native resource
lifecycle management while delegating all cache behavior to Rust.

## Commands

From `clients/java`:

```bash
mvn package
```

## Components

- `pom.xml` defines Maven publication and compiler metadata.
- `src/main/java/io/openkache/client/package-info.java` reserves the package.

## Configuration

JNI or Foreign Function and Memory API integration, native artifacts, and
runtime options will be selected when the binding is implemented.
