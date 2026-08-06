//go:build !cgo

package openkache

import "context"

type unavailableNativeClient struct{}

func connectNative(context.Context, normalizedOptions) (nativeClient, error) {
	return nil, &Error{
		Operation: "connect",
		Message:   "the Go client requires CGO and the shared OpenKache native library",
	}
}

func (unavailableNativeClient) execute(
	context.Context,
	uint32,
	[]byte,
	[]byte,
	SetOptions,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "execute", Message: "native client unavailable"}
}

func (unavailableNativeClient) executeRaw(
	context.Context,
	uint32,
	ItemID,
	[]byte,
	SetOptions,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "execute", Message: "native client unavailable"}
}

func (unavailableNativeClient) executeScoped(
	context.Context,
	uint32,
	uint64,
	ItemID,
	[]byte,
	SetOptions,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "execute scoped", Message: "native client unavailable"}
}

func (unavailableNativeClient) executeScopedBytes(
	context.Context,
	uint32,
	uint64,
	[]byte,
	[]byte,
	SetOptions,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "execute scoped", Message: "native client unavailable"}
}

func (unavailableNativeClient) namespaceOpen(
	context.Context,
	[]byte,
	bool,
	uint8,
	uint64,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "namespace open", Message: "native client unavailable"}
}

func (unavailableNativeClient) namespaceUpdatePolicy(
	context.Context,
	uint64,
	uint64,
	uint8,
	uint64,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "namespace update policy", Message: "native client unavailable"}
}

func (unavailableNativeClient) namespaceDelete(
	context.Context,
	uint64,
	uint64,
) (nativeResult, error) {
	return nativeResult{}, &Error{Operation: "namespace delete", Message: "native client unavailable"}
}

func (unavailableNativeClient) decodeNamespaceDescriptor(
	[]byte,
) (nativeNamespaceDescriptor, error) {
	return nativeNamespaceDescriptor{}, &Error{
		Operation: "namespace descriptor",
		Message:   "native client unavailable",
	}
}

func (unavailableNativeClient) state() uint32 {
	return SmithyFFIConnectionStateUnknown
}

func (unavailableNativeClient) close() error {
	return nil
}
