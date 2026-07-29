# OpenKache Kotlin Client

This directory is the Gradle package scaffold for a future thin Kotlin binding
to the OpenKache Rust client ABI. It intentionally contains no connection or
cache operation implementation.

## Purpose

The Kotlin package will add coroutine-friendly APIs over the shared native core
without maintaining a Kotlin protocol implementation.

## Commands

From `clients/kotlin`:

```bash
gradle build
```

## Components

- `settings.gradle.kts` names the Gradle project.
- `build.gradle.kts` defines Kotlin/JVM publication metadata.
- `src/main/kotlin/io/openkache/client/OpenKache.kt` reserves the package entry point.

## Configuration

Coroutine APIs, JVM native loading, and multiplatform targets will be decided
when the binding is implemented.
