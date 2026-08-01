//go:build cgo

package openkache

/*
#cgo CFLAGS: -I${SRCDIR}/../core/include -I${SRCDIR}/../core/generated_local
#cgo linux LDFLAGS: -ldl

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "openkache_client.h"

#if defined(_WIN32)
#include <windows.h>
typedef HMODULE openkache_go_library_handle;
#else
#include <dlfcn.h>
typedef void *openkache_go_library_handle;
#endif

typedef uint32_t (*openkache_go_abi_fn)(void);
typedef openkache_client_result *(*openkache_go_connect_options_fn)(
    const openkache_client_connect_options_t *);
typedef openkache_client_result *(*openkache_go_execute_request_fn)(
    const openkache_client_handle *, uint64_t, uint32_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint32_t, uint8_t, uint64_t);
typedef openkache_client_result *(*openkache_go_execute_mutation_fn)(
    const openkache_client_handle *, uint64_t, uint32_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint32_t, uint8_t, uint64_t, const uint8_t *, size_t);
typedef uint32_t (*openkache_go_connection_state_fn)(
    const openkache_client_handle *);
typedef uint8_t (*openkache_go_cancel_fn)(
    const openkache_client_handle *, uint64_t);
typedef uint8_t (*openkache_go_metrics_fn)(
    const openkache_client_handle *, openkache_client_metrics_snapshot_t *);
typedef uint8_t (*openkache_go_result_metadata_fn)(
    const openkache_client_result *, openkache_client_error_metadata_t *);
typedef uint32_t (*openkache_go_result_kind_fn)(const openkache_client_result *);
typedef const uint8_t *(*openkache_go_result_data_fn)(const openkache_client_result *);
typedef size_t (*openkache_go_result_data_length_fn)(const openkache_client_result *);
typedef openkache_client_handle *(*openkache_go_result_take_client_fn)(
    openkache_client_result *);
typedef void (*openkache_go_result_free_fn)(openkache_client_result *);
typedef void (*openkache_go_client_free_fn)(openkache_client_handle *);

typedef struct openkache_go_library {
    openkache_go_library_handle handle;
    openkache_go_abi_fn abi;
    openkache_go_connect_options_fn connect_options;
    openkache_go_execute_request_fn execute_request;
    openkache_go_execute_request_fn execute_raw_request;
    openkache_go_execute_mutation_fn execute_mutation;
    openkache_go_execute_mutation_fn execute_raw_mutation;
    openkache_go_connection_state_fn connection_state;
    openkache_go_cancel_fn cancel;
    openkache_go_metrics_fn metrics;
    openkache_go_result_kind_fn result_kind;
    openkache_go_result_data_fn result_data;
    openkache_go_result_data_length_fn result_data_length;
    openkache_go_result_metadata_fn result_metadata;
    openkache_go_result_take_client_fn result_take_client;
    openkache_go_result_free_fn result_free;
    openkache_go_client_free_fn client_free;
} openkache_go_library;

static void *openkache_go_symbol(openkache_go_library_handle handle, const char *name) {
#if defined(_WIN32)
    return (void *)GetProcAddress(handle, name);
#else
    return dlsym(handle, name);
#endif
}

static void openkache_go_assign(void *target, size_t target_size, void *symbol) {
    memset(target, 0, target_size);
    memcpy(target, &symbol, target_size < sizeof(symbol) ? target_size : sizeof(symbol));
}

static char *openkache_go_error_copy(const char *message) {
    size_t length = strlen(message);
    char *copy = (char *)malloc(length + 1);
    if (copy == NULL) return NULL;
    memcpy(copy, message, length + 1);
    return copy;
}

static openkache_go_library_handle openkache_go_open(
    const char *path,
    const char **error_message
) {
#if defined(_WIN32)
    HMODULE handle = LoadLibraryA(path);
    if (handle == NULL) *error_message = "LoadLibraryA failed";
    return handle;
#else
    void *handle = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (handle == NULL) *error_message = dlerror();
    return handle;
#endif
}

static void openkache_go_close(openkache_go_library_handle handle) {
#if defined(_WIN32)
    if (handle != NULL) FreeLibrary(handle);
#else
    if (handle != NULL) dlclose(handle);
#endif
}

openkache_go_library *openkache_go_library_load(
    const char *requested_path,
    char **error_message
) {
    const char *candidates[] = {
        requested_path,
#if defined(_WIN32)
        "openkache_client.dll",
#elif defined(__APPLE__)
        "libopenkache_client.dylib",
        "libopenkache_client.so",
#else
        "libopenkache_client.so",
#endif
        NULL,
    };
    const char *last_error = "no native OpenKache client library found";
    openkache_go_library_handle handle = NULL;
    for (size_t index = 0;
         index < (sizeof(candidates) / sizeof(candidates[0])) - 1;
         ++index) {
        if (requested_path != NULL && index > 0) break;
        if (candidates[index] == NULL) continue;
        if (candidates[index][0] == '\0') continue;
        handle = openkache_go_open(candidates[index], &last_error);
        if (handle != NULL) break;
    }
    if (handle == NULL) {
        if (last_error == NULL) last_error = "loading native OpenKache client failed";
        *error_message = openkache_go_error_copy(last_error);
        return NULL;
    }

    openkache_go_library *library =
        (openkache_go_library *)calloc(1, sizeof(openkache_go_library));
    if (library == NULL) {
        openkache_go_close(handle);
        *error_message = openkache_go_error_copy("allocating native library state failed");
        return NULL;
    }
    library->handle = handle;
#define OPENKACHE_GO_LOAD(field, symbol_name) \
    openkache_go_assign(&library->field, sizeof(library->field), \
                        openkache_go_symbol(handle, symbol_name))
    OPENKACHE_GO_LOAD(abi, "openkache_client_abi_version");
    OPENKACHE_GO_LOAD(connect_options, "openkache_client_connect_with_options");
    OPENKACHE_GO_LOAD(execute_request, "openkache_client_execute_with_request_id");
    OPENKACHE_GO_LOAD(execute_raw_request, "openkache_client_execute_raw_with_request_id");
    OPENKACHE_GO_LOAD(execute_mutation,
                      "openkache_client_execute_with_request_id_and_mutation_id");
    OPENKACHE_GO_LOAD(execute_raw_mutation,
                      "openkache_client_execute_raw_with_request_id_and_mutation_id");
    OPENKACHE_GO_LOAD(connection_state, "openkache_client_connection_state");
    OPENKACHE_GO_LOAD(cancel, "openkache_client_cancel");
    OPENKACHE_GO_LOAD(metrics, "openkache_client_metrics_snapshot");
    OPENKACHE_GO_LOAD(result_kind, "openkache_client_result_kind");
    OPENKACHE_GO_LOAD(result_data, "openkache_client_result_data");
    OPENKACHE_GO_LOAD(result_data_length, "openkache_client_result_data_length");
    OPENKACHE_GO_LOAD(result_metadata, "openkache_client_result_error_metadata");
    OPENKACHE_GO_LOAD(result_take_client, "openkache_client_result_take_client");
    OPENKACHE_GO_LOAD(result_free, "openkache_client_result_free");
    OPENKACHE_GO_LOAD(client_free, "openkache_client_free");
#undef OPENKACHE_GO_LOAD

    if (library->abi == NULL || library->connect_options == NULL ||
        library->execute_request == NULL || library->execute_raw_request == NULL ||
        library->result_kind == NULL || library->result_data == NULL ||
        library->result_data_length == NULL || library->result_take_client == NULL ||
        library->result_free == NULL || library->client_free == NULL) {
        openkache_go_close(handle);
        free(library);
        *error_message = openkache_go_error_copy("native library is missing the OpenKache ABI");
        return NULL;
    }
    if (library->abi() != OPENKACHE_CLIENT_ABI_VERSION) {
        openkache_go_close(handle);
        free(library);
        *error_message = openkache_go_error_copy("native OpenKache ABI version is unsupported");
        return NULL;
    }
    return library;
}

void openkache_go_library_free(openkache_go_library *library) {
    if (library == NULL) return;
    openkache_go_close(library->handle);
    free(library);
}

uint32_t openkache_go_connection_state(
    const openkache_go_library *library,
    const openkache_client_handle *client
) {
    if (library == NULL || library->connection_state == NULL || client == NULL) {
        return OPENKACHE_SMITHY_FFI_CONNECTION_STATE_UNKNOWN;
    }
    return library->connection_state(client);
}

openkache_client_result *openkache_go_connect_options(
    const openkache_go_library *library,
    const openkache_client_connect_options_t *options
) {
    if (library == NULL || library->connect_options == NULL) return NULL;
    return library->connect_options(options);
}

openkache_client_result *openkache_go_execute_request(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    uint64_t request_id,
    uint32_t operation,
    const uint8_t *key, size_t key_length,
    const uint8_t *value, size_t value_length,
    uint32_t set_condition, uint8_t ttl_enabled, uint64_t ttl_ms,
    int raw
) {
    if (library == NULL) return NULL;
    openkache_go_execute_request_fn function =
        raw ? library->execute_raw_request : library->execute_request;
    if (function == NULL) return NULL;
    return function(client, request_id, operation, key, key_length, value,
                    value_length, set_condition, ttl_enabled, ttl_ms);
}

openkache_client_result *openkache_go_execute_mutation(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    uint64_t request_id,
    uint32_t operation,
    const uint8_t *key, size_t key_length,
    const uint8_t *value, size_t value_length,
    uint32_t set_condition, uint8_t ttl_enabled, uint64_t ttl_ms,
    const uint8_t *mutation_id, size_t mutation_id_length,
    int raw
) {
    if (library == NULL) return NULL;
    openkache_go_execute_mutation_fn function =
        raw ? library->execute_raw_mutation : library->execute_mutation;
    if (function == NULL) return NULL;
    return function(client, request_id, operation, key, key_length, value,
                    value_length, set_condition, ttl_enabled, ttl_ms,
                    mutation_id, mutation_id_length);
}

uint8_t openkache_go_cancel(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    uint64_t request_id
) {
    return (library == NULL || library->cancel == NULL || client == NULL)
        ? 0 : library->cancel(client, request_id);
}

uint8_t openkache_go_metrics(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    openkache_client_metrics_snapshot_t *snapshot
) {
    return (library == NULL || library->metrics == NULL || client == NULL)
        ? 0 : library->metrics(client, snapshot);
}

uint8_t openkache_go_result_metadata(
    const openkache_go_library *library,
    const openkache_client_result *result,
    openkache_client_error_metadata_t *metadata
) {
    return (library == NULL || library->result_metadata == NULL || result == NULL)
        ? 0 : library->result_metadata(result, metadata);
}

uint32_t openkache_go_result_kind(
    const openkache_go_library *library,
    const openkache_client_result *result
) {
    return (library == NULL || result == NULL) ? OPENKACHE_CLIENT_RESULT_ERROR
                                                : library->result_kind(result);
}

const uint8_t *openkache_go_result_data(
    const openkache_go_library *library,
    const openkache_client_result *result
) {
    return (library == NULL || result == NULL) ? NULL : library->result_data(result);
}

size_t openkache_go_result_data_length(
    const openkache_go_library *library,
    const openkache_client_result *result
) {
    return (library == NULL || result == NULL) ? 0 : library->result_data_length(result);
}

openkache_client_handle *openkache_go_result_take_client(
    const openkache_go_library *library,
    openkache_client_result *result
) {
    return (library == NULL || result == NULL) ? NULL : library->result_take_client(result);
}

void openkache_go_result_free(
    const openkache_go_library *library,
    openkache_client_result *result
) {
    if (library != NULL && result != NULL) library->result_free(result);
}

void openkache_go_client_free(
    const openkache_go_library *library,
    openkache_client_handle *client
) {
    if (library != NULL && client != NULL) library->client_free(client);
}
*/
import "C"

import (
	"context"
	"errors"
	"os"
	"sync"
	"time"
	"unsafe"
)

type nativeLibrary struct {
	ptr *C.openkache_go_library
}

type nativeHandle struct {
	library *nativeLibrary
	client  *C.openkache_client_handle

	mu            sync.Mutex
	cond          *sync.Cond
	active        int
	closed        bool
	nextRequestID uint64
}

func connectNative(ctx context.Context, options normalizedOptions) (nativeClient, error) {
	path := options.nativeLibrary
	if path == "" {
		path = os.Getenv("OPENKACHE_CLIENT_LIBRARY")
	}
	var cPath *C.char
	if path != "" {
		cPath = C.CString(path)
		defer C.free(unsafe.Pointer(cPath))
	}
	var cError *C.char
	library := C.openkache_go_library_load(cPath, &cError)
	if library == nil {
		message := "native OpenKache client library could not be loaded"
		if cError != nil {
			message = C.GoString(cError)
			C.free(unsafe.Pointer(cError))
		}
		return nil, &Error{Operation: "connect", Message: message}
	}
	nativeLibrary := &nativeLibrary{ptr: library}

	address := C.CBytes([]byte(options.address))
	serverName := C.CBytes([]byte(options.serverName))
	certificate := C.CBytes(options.certificate)
	identityCertificate := C.CBytes(options.identityCertificate)
	identityPrivateKey := C.CBytes(options.identityPrivateKey)
	dataProtectionKey := C.CBytes(options.dataProtectionKey)
	previousKeys := C.CBytes(options.previousKeys)

	connectTimeout, requestTimeout, err := durationMilliseconds(options.timeouts)
	if err != nil {
		C.free(address)
		C.free(serverName)
		C.free(certificate)
		C.free(identityCertificate)
		C.free(identityPrivateKey)
		C.free(dataProtectionKey)
		C.free(previousKeys)
		C.openkache_go_library_free(library)
		return nil, err
	}
	compressionLevel := options.compression.Level
	if compressionLevel == 0 {
		compressionLevel = SmithyDefaultZstandardLevel
	}
	minimumInputSize := options.compression.MinimumInputSize
	if minimumInputSize == 0 {
		minimumInputSize = SmithyDefaultZstandardMinimumInputBytes
	}
	minimumSavings := options.compression.MinimumSavings
	if minimumSavings == 0 {
		minimumSavings = SmithyDefaultZstandardMinimumSavingsBytes
	}
	type connectReply struct {
		handle *nativeHandle
		err    error
	}
	reply := make(chan connectReply)
	go func() {
		connectOptions := C.openkache_client_connect_options_t{
			address:                              (*C.uint8_t)(address),
			address_length:                       C.size_t(len(options.address)),
			server_name:                          (*C.uint8_t)(serverName),
			server_name_length:                   C.size_t(len(options.serverName)),
			certificate:                          (*C.uint8_t)(certificate),
			certificate_length:                   C.size_t(len(options.certificate)),
			client_certificate_chain:             (*C.uint8_t)(identityCertificate),
			client_certificate_chain_length:      C.size_t(len(options.identityCertificate)),
			client_private_key:                   (*C.uint8_t)(identityPrivateKey),
			client_private_key_length:            C.size_t(len(options.identityPrivateKey)),
			data_protection_key:                  (*C.uint8_t)(dataProtectionKey),
			data_protection_key_length:           C.size_t(len(options.dataProtectionKey)),
			previous_data_protection_keys:        (*C.uint8_t)(previousKeys),
			previous_data_protection_keys_length: C.size_t(len(options.previousKeys)),
			previous_data_protection_key_count: C.size_t(
				len(options.previousKeys) / SmithyDataProtectionKeyBytes),
			compression_enabled: C.uint8_t(boolByte(options.compression.Enabled)),
			compression_level:   C.int32_t(compressionLevel),
			minimum_input_size:  C.size_t(minimumInputSize),
			minimum_savings:     C.size_t(minimumSavings),
			encryption:          C.uint32_t(options.encryption),
			connect_timeout_ms:  C.uint64_t(connectTimeout),
			request_timeout_ms:  C.uint64_t(requestTimeout),
			retry_max_attempts:  C.size_t(options.retryAttempts),
			max_in_flight:       C.size_t(options.maxInFlight),
		}
		result := C.openkache_go_connect_options(library, &connectOptions)
		C.free(address)
		C.free(serverName)
		C.free(certificate)
		C.free(identityCertificate)
		C.free(identityPrivateKey)
		C.free(dataProtectionKey)
		C.free(previousKeys)
		handle, err := decodeConnectResult(nativeLibrary, result)
		select {
		case reply <- connectReply{handle: handle, err: err}:
		case <-ctx.Done():
			if handle != nil {
				_ = handle.close()
			} else if err == nil {
				C.openkache_go_library_free(library)
			}
		}
	}()

	select {
	case result := <-reply:
		return result.handle, result.err
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

func decodeConnectResult(
	library *nativeLibrary,
	result *C.openkache_client_result,
) (*nativeHandle, error) {
	if result == nil {
		C.openkache_go_library_free(library.ptr)
		return nil, &Error{Operation: "connect", Message: "native ABI returned a null result"}
	}
	kind := uint32(C.openkache_go_result_kind(library.ptr, result))
	if kind != SmithyFFIResultConnected {
		length := C.openkache_go_result_data_length(library.ptr, result)
		var data []byte
		if length != 0 {
			pointer := C.openkache_go_result_data(library.ptr, result)
			if pointer == nil {
				data = []byte("native ABI returned a null payload")
			} else {
				data = C.GoBytes(unsafe.Pointer(pointer), C.int(length))
			}
		}
		C.openkache_go_result_free(library.ptr, result)
		C.openkache_go_library_free(library.ptr)
		if len(data) == 0 {
			data = []byte("native ABI returned an error without a payload")
		}
		return nil, &Error{Operation: "connect", Message: string(data)}
	}
	client := C.openkache_go_result_take_client(library.ptr, result)
	C.openkache_go_result_free(library.ptr, result)
	if client == nil {
		C.openkache_go_library_free(library.ptr)
		return nil, &Error{Operation: "connect", Message: "native ABI returned a null client"}
	}
	handle := &nativeHandle{
		library: library,
		client:  client,
	}
	handle.cond = sync.NewCond(&handle.mu)
	return handle, nil
}

func (h *nativeHandle) execute(
	ctx context.Context,
	operation uint32,
	key, value []byte,
	options SetOptions,
) (nativeResult, error) {
	return h.executeNative(ctx, operation, key, value, options, false)
}

func (h *nativeHandle) executeRaw(
	ctx context.Context,
	operation uint32,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	return h.executeNative(ctx, operation, itemID[:], value, options, true)
}

func (h *nativeHandle) executeNative(
	ctx context.Context,
	operation uint32,
	key, value []byte,
	options SetOptions,
	raw bool,
) (nativeResult, error) {
	client, err := h.begin()
	if err != nil {
		return nativeResult{}, err
	}
	keyMemory := C.CBytes(key)
	valueMemory := C.CBytes(value)
	var mutationMemory unsafe.Pointer
	if options.MutationID != nil {
		mutationMemory = C.CBytes(options.MutationID[:])
	}

	condition := SmithyFFISetConditionNone
	switch options.Condition {
	case IfAbsent:
		condition = SmithyFFISetConditionIfAbsent
	case IfPresent:
		condition = SmithyFFISetConditionIfPresent
	}
	ttlEnabled := boolByte(options.TTLMillis != 0)
	h.mu.Lock()
	requestID := h.nextRequestID
	h.nextRequestID++
	h.mu.Unlock()

	done := make(chan nativeResult, 1)
	go func() {
		defer h.end()
		var result *C.openkache_client_result
		if options.MutationID != nil {
			result = C.openkache_go_execute_mutation(
				h.library.ptr, client, C.uint64_t(requestID), C.uint32_t(operation),
				(*C.uint8_t)(keyMemory), C.size_t(len(key)),
				(*C.uint8_t)(valueMemory), C.size_t(len(value)),
				C.uint32_t(condition), C.uint8_t(ttlEnabled), C.uint64_t(options.TTLMillis),
				(*C.uint8_t)(mutationMemory), C.size_t(len(options.MutationID)),
				C.int(boolByte(raw)),
			)
		} else {
			result = C.openkache_go_execute_request(
				h.library.ptr, client, C.uint64_t(requestID), C.uint32_t(operation),
				(*C.uint8_t)(keyMemory), C.size_t(len(key)),
				(*C.uint8_t)(valueMemory), C.size_t(len(value)),
				C.uint32_t(condition), C.uint8_t(ttlEnabled), C.uint64_t(options.TTLMillis),
				C.int(boolByte(raw)),
			)
		}
		C.free(keyMemory)
		C.free(valueMemory)
		if mutationMemory != nil {
			C.free(mutationMemory)
		}
		kind, data, metadata := decodeResult(h.library, result)
		done <- nativeResult{kind: kind, data: data, metadata: metadata}
	}()

	select {
	case result := <-done:
		if result.kind == SmithyFFIResultError {
			return nativeResult{}, &Error{Message: string(result.data), Metadata: result.metadata}
		}
		return result, nil
	case <-ctx.Done():
		C.openkache_go_cancel(h.library.ptr, client, C.uint64_t(requestID))
		return nativeResult{}, contextNativeError(operation, ctx.Err(), options.MutationID)
	}
}

func contextNativeError(operation uint32, cause error, mutationID *MutationID) *Error {
	code := uint32(SmithyFFIErrorCancelled)
	message := "client operation canceled"
	retryable := false
	if errors.Is(cause, context.DeadlineExceeded) {
		code = SmithyFFIErrorTimeout
		message = "client operation timed out"
		retryable = true
	}
	var mutationBytes []byte
	ambiguous := false
	if mutationID != nil {
		// A caller-visible mutation token makes an interrupted mutation safe to
		// retry, but the request may already have reached the server.
		mutationBytes = append([]byte(nil), mutationID[:]...)
		ambiguous = true
		retryable = true
	}
	return &Error{
		Message: message,
		Cause:   cause,
		Metadata: &ErrorMetadata{
			Code:       code,
			Operation:  contextOperationCode(operation),
			Retryable:  retryable,
			Ambiguous:  ambiguous,
			MutationID: mutationBytes,
		},
	}
}

func contextOperationCode(operation uint32) uint32 {
	// Metadata operation IDs are caller-facing FFI operations. Keep adapter
	// operations such as GET_JSON and RECONNECT distinct from wire opcodes.
	return operation
}

func (h *nativeHandle) state() uint32 {
	client, err := h.begin()
	if err != nil {
		return SmithyFFIConnectionStateClosed
	}
	defer h.end()
	return uint32(C.openkache_go_connection_state(h.library.ptr, client))
}

func (h *nativeHandle) metrics() MetricsSnapshot {
	client, err := h.begin()
	if err != nil {
		return MetricsSnapshot{}
	}
	defer h.end()
	var snapshot C.openkache_client_metrics_snapshot_t
	if C.openkache_go_metrics(h.library.ptr, client, &snapshot) == 0 {
		return MetricsSnapshot{}
	}
	return MetricsSnapshot{
		Requests:        uint64(snapshot.requests),
		Hits:            uint64(snapshot.hits),
		Misses:          uint64(snapshot.misses),
		Retries:         uint64(snapshot.retries),
		Reconnects:      uint64(snapshot.reconnects),
		Cancellations:   uint64(snapshot.cancellations),
		TransportErrors: uint64(snapshot.transport_errors),
		ProtocolErrors:  uint64(snapshot.protocol_errors),
		BytesSent:       uint64(snapshot.bytes_sent),
		BytesReceived:   uint64(snapshot.bytes_received),
		ActiveLanes:     uint64(snapshot.active_lanes),
	}
}

func (h *nativeHandle) begin() (*C.openkache_client_handle, error) {
	h.mu.Lock()
	defer h.mu.Unlock()
	if h.closed || h.client == nil {
		return nil, ErrClosed
	}
	h.active++
	return h.client, nil
}

func (h *nativeHandle) end() {
	h.mu.Lock()
	h.active--
	if h.active == 0 {
		h.cond.Broadcast()
	}
	h.mu.Unlock()
}

func (h *nativeHandle) close() error {
	h.mu.Lock()
	if h.closed {
		h.mu.Unlock()
		return nil
	}
	h.closed = true
	for h.active != 0 {
		h.cond.Wait()
	}
	client := h.client
	h.client = nil
	h.mu.Unlock()

	C.openkache_go_client_free(h.library.ptr, client)
	C.openkache_go_library_free(h.library.ptr)
	return nil
}

func decodeResult(
	library *nativeLibrary,
	result *C.openkache_client_result,
) (uint32, []byte, *ErrorMetadata) {
	if result == nil {
		return SmithyFFIResultError, []byte("native ABI returned a null result"), nil
	}
	kind := uint32(C.openkache_go_result_kind(library.ptr, result))
	length := C.openkache_go_result_data_length(library.ptr, result)
	var data []byte
	if length != 0 {
		pointer := C.openkache_go_result_data(library.ptr, result)
		if pointer == nil {
			kind = SmithyFFIResultError
			data = []byte("native ABI returned a null payload")
		} else {
			data = C.GoBytes(unsafe.Pointer(pointer), C.int(length))
		}
	}
	var metadata *ErrorMetadata
	if kind == SmithyFFIResultError {
		var value C.openkache_client_error_metadata_t
		if C.openkache_go_result_metadata(library.ptr, result, &value) != 0 {
			var mutationID []byte
			if value.mutation_id_length != 0 {
				length := int(value.mutation_id_length)
				if length > SmithyMutationIDBytes {
					length = SmithyMutationIDBytes
				}
				mutationID = C.GoBytes(
					unsafe.Pointer(&value.mutation_id[0]),
					C.int(length),
				)
			}
			metadata = &ErrorMetadata{
				Code:       uint32(value.code),
				Operation:  uint32(value.operation),
				Phase:      uint32(value.phase),
				Backend:    uint32(value.backend),
				Retryable:  value.retryable != 0,
				Ambiguous:  value.ambiguous != 0,
				MutationID: mutationID,
			}
		}
	}
	C.openkache_go_result_free(library.ptr, result)
	if kind == SmithyFFIResultError && len(data) == 0 {
		data = []byte("native ABI returned an error without a payload")
	}
	return kind, data, metadata
}

func durationMilliseconds(timeouts TimeoutOptions) (uint64, uint64, error) {
	connect := uint64(timeouts.Connect / time.Millisecond)
	request := uint64(timeouts.Request / time.Millisecond)
	minimum := uint64(SmithyClientMinimumPositiveValue)
	if connect < minimum || request < minimum {
		return 0, 0, validationError("timeouts", "must be at least one millisecond")
	}
	return connect, request, nil
}

func boolByte(value bool) uint8 {
	if value {
		return uint8(SmithyClientMinimumPositiveValue)
	}
	return 0
}
