// Package openkache is the Go binding for the OpenKache cache server.
//
// Connection management, QUIC, TLS, retries, key derivation, compression, and
// value protection are implemented by openkache-client-core. This package
// provides context-aware Go methods and owns only Go-side validation,
// conversion, and native-handle lifetime.
package openkache
