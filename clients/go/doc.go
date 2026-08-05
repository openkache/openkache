// Package openkache is the Go binding for the OpenKache cache server.
//
// Connection management, QUIC, TLS, retries, key derivation, compression, and
// value protection are implemented by openkache-client-core. This package
// provides context-aware Go methods and owns only Go-side validation,
// conversion, and native-handle lifetime.
//
//go:generate env OPENKACHE_GENERATION_TARGET=go bun ../generate.ts
//go:generate env OPENKACHE_GENERATION_TARGET=c-contract OPENKACHE_C_CONTRACT_OUTPUT=../core/generated_local/smithy_contract.h bun ../generate.ts
package openkache
