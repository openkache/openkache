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

func (unavailableNativeClient) state() uint32 {
	return SmithyFFIConnectionStateUnknown
}

func (unavailableNativeClient) metrics() MetricsSnapshot {
	return MetricsSnapshot{}
}

func (unavailableNativeClient) close() error {
	return nil
}
