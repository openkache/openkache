//go:build cgo

package openkache

/*
#cgo CFLAGS: -I${SRCDIR}/../core/include
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
typedef openkache_client_result *(*openkache_go_connect_fn)(
    const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint8_t, int32_t, size_t, size_t, uint64_t, uint64_t);
typedef openkache_client_result *(*openkache_go_connect_ex_fn)(
    const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, const uint8_t *, size_t, const uint8_t *, size_t,
    uint8_t, int32_t, size_t, size_t, uint32_t, size_t, size_t, uint64_t, uint64_t);
typedef openkache_client_result *(*openkache_go_execute_fn)(
    const openkache_client_handle *, uint32_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint32_t, uint8_t, uint64_t);
typedef openkache_client_result *(*openkache_go_execute_raw_fn)(
    const openkache_client_handle *, uint32_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint32_t, uint8_t, uint64_t);
typedef uint32_t (*openkache_go_connection_state_fn)(
    const openkache_client_handle *);
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
    openkache_go_connect_fn connect;
    openkache_go_connect_ex_fn connect_ex;
    openkache_go_execute_fn execute;
    openkache_go_execute_raw_fn execute_raw;
    openkache_go_connection_state_fn connection_state;
    openkache_go_result_kind_fn result_kind;
    openkache_go_result_data_fn result_data;
    openkache_go_result_data_length_fn result_data_length;
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
    OPENKACHE_GO_LOAD(connect, "openkache_client_connect");
    OPENKACHE_GO_LOAD(connect_ex, "openkache_client_connect_ex");
    OPENKACHE_GO_LOAD(execute, "openkache_client_execute");
    OPENKACHE_GO_LOAD(execute_raw, "openkache_client_execute_raw");
    OPENKACHE_GO_LOAD(connection_state, "openkache_client_connection_state");
    OPENKACHE_GO_LOAD(result_kind, "openkache_client_result_kind");
    OPENKACHE_GO_LOAD(result_data, "openkache_client_result_data");
    OPENKACHE_GO_LOAD(result_data_length, "openkache_client_result_data_length");
    OPENKACHE_GO_LOAD(result_take_client, "openkache_client_result_take_client");
    OPENKACHE_GO_LOAD(result_free, "openkache_client_result_free");
    OPENKACHE_GO_LOAD(client_free, "openkache_client_free");
#undef OPENKACHE_GO_LOAD

    if (library->abi == NULL || library->connect == NULL || library->execute == NULL ||
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

int openkache_go_has_connect_ex(const openkache_go_library *library) {
    return library != NULL && library->connect_ex != NULL;
}

int openkache_go_has_execute_raw(const openkache_go_library *library) {
    return library != NULL && library->execute_raw != NULL;
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

openkache_client_result *openkache_go_connect(
    const openkache_go_library *library,
    const uint8_t *address, size_t address_length,
    const uint8_t *server_name, size_t server_name_length,
    const uint8_t *certificate, size_t certificate_length,
    const uint8_t *identity_certificate_chain, size_t identity_certificate_chain_length,
    const uint8_t *identity_private_key, size_t identity_private_key_length,
    const uint8_t *data_protection_key, size_t data_protection_key_length,
    uint8_t compression_enabled, int32_t compression_level,
    size_t minimum_input_size, size_t minimum_savings,
    uint32_t encryption,
    size_t retry_max_attempts, size_t max_in_flight,
    uint64_t connect_timeout_ms, uint64_t request_timeout_ms,
    uint8_t use_extended
) {
    if (library == NULL) return NULL;
    if (use_extended != 0) {
        if (library->connect_ex == NULL) return NULL;
        return library->connect_ex(
            address, address_length, server_name, server_name_length,
            certificate, certificate_length, identity_certificate_chain,
            identity_certificate_chain_length, identity_private_key,
            identity_private_key_length, data_protection_key,
            data_protection_key_length, compression_enabled, compression_level,
            minimum_input_size, minimum_savings, encryption,
            retry_max_attempts, max_in_flight, connect_timeout_ms,
            request_timeout_ms);
    }
    return library->connect(
        address, address_length, server_name, server_name_length,
        certificate, certificate_length, data_protection_key,
        data_protection_key_length, compression_enabled, compression_level,
        minimum_input_size, minimum_savings, connect_timeout_ms,
        request_timeout_ms);
}

openkache_client_result *openkache_go_execute(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    uint32_t operation,
    const uint8_t *application_key, size_t application_key_length,
    const uint8_t *value, size_t value_length,
    uint32_t set_condition, uint8_t ttl_enabled, uint64_t ttl_ms
) {
    if (library == NULL || library->execute == NULL) return NULL;
    return library->execute(
        client, operation, application_key, application_key_length, value,
        value_length, set_condition, ttl_enabled, ttl_ms);
}

openkache_client_result *openkache_go_execute_raw(
    const openkache_go_library *library,
    const openkache_client_handle *client,
    uint32_t operation,
    const uint8_t *item_id, size_t item_id_length,
    const uint8_t *value, size_t value_length,
    uint32_t set_condition, uint8_t ttl_enabled, uint64_t ttl_ms
) {
    if (library == NULL || library->execute_raw == NULL) return NULL;
    return library->execute_raw(
        client, operation, item_id, item_id_length, value, value_length,
        set_condition, ttl_enabled, ttl_ms);
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
	raw     bool

	mu     sync.Mutex
	cond   *sync.Cond
	active int
	closed bool
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

	connectTimeout, requestTimeout, err := durationMilliseconds(options.timeouts)
	if err != nil {
		C.free(address)
		C.free(serverName)
		C.free(certificate)
		C.free(identityCertificate)
		C.free(identityPrivateKey)
		C.free(dataProtectionKey)
		C.openkache_go_library_free(library)
		return nil, err
	}
	compressionLevel := options.compression.Level
	if compressionLevel == 0 {
		compressionLevel = SmithyClientDefaultCompressionLevel
	}
	minimumInputSize := options.compression.MinimumInputSize
	if minimumInputSize == 0 {
		minimumInputSize = SmithyClientDefaultCompressionMinimumInputSize
	}
	minimumSavings := options.compression.MinimumSavings
	if minimumSavings == 0 {
		minimumSavings = SmithyClientDefaultCompressionMinimumSavings
	}
	hasExtended := C.openkache_go_has_connect_ex(library) != 0
	useExtended := hasExtended
	if !hasExtended &&
		(len(options.identityCertificate) != 0 ||
			len(options.identityPrivateKey) != 0 ||
			options.retryAttempts != SmithyClientDefaultRetryMaxAttempts ||
			options.maxInFlight != SmithyClientDefaultMaxInFlight) {
		C.free(address)
		C.free(serverName)
		C.free(certificate)
		C.free(identityCertificate)
		C.free(identityPrivateKey)
		C.free(dataProtectionKey)
		C.openkache_go_library_free(library)
		return nil, &Error{
			Operation: "connect",
			Message:   "native library does not support the requested extended options",
		}
	}

	type connectReply struct {
		handle *nativeHandle
		err    error
	}
	reply := make(chan connectReply)
	go func() {
		result := C.openkache_go_connect(
			library,
			(*C.uint8_t)(address), C.size_t(len(options.address)),
			(*C.uint8_t)(serverName), C.size_t(len(options.serverName)),
			(*C.uint8_t)(certificate), C.size_t(len(options.certificate)),
			(*C.uint8_t)(identityCertificate), C.size_t(len(options.identityCertificate)),
			(*C.uint8_t)(identityPrivateKey), C.size_t(len(options.identityPrivateKey)),
			(*C.uint8_t)(dataProtectionKey), C.size_t(len(options.dataProtectionKey)),
			C.uint8_t(boolByte(options.compression.Enabled)), C.int32_t(compressionLevel),
			C.size_t(minimumInputSize), C.size_t(minimumSavings),
			C.uint32_t(SmithyValueEncryptionRobust),
			C.size_t(options.retryAttempts), C.size_t(options.maxInFlight),
			C.uint64_t(connectTimeout), C.uint64_t(requestTimeout),
			C.uint8_t(boolByte(useExtended)),
		)
		C.free(address)
		C.free(serverName)
		C.free(certificate)
		C.free(identityCertificate)
		C.free(identityPrivateKey)
		C.free(dataProtectionKey)
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
		raw:     C.openkache_go_has_execute_raw(library.ptr) != 0,
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
	client, err := h.begin()
	if err != nil {
		return nativeResult{}, err
	}
	keyMemory := C.CBytes(key)
	valueMemory := C.CBytes(value)

	condition := SmithyFFISetConditionNone
	switch options.Condition {
	case IfAbsent:
		condition = SmithyFFISetConditionIfAbsent
	case IfPresent:
		condition = SmithyFFISetConditionIfPresent
	}
	ttlEnabled := boolByte(options.TTLMillis != 0)

	done := make(chan nativeResult, 1)
	go func() {
		defer h.end()
		result := C.openkache_go_execute(
			h.library.ptr, client, C.uint32_t(operation),
			(*C.uint8_t)(keyMemory), C.size_t(len(key)),
			(*C.uint8_t)(valueMemory), C.size_t(len(value)),
			C.uint32_t(condition), C.uint8_t(ttlEnabled), C.uint64_t(options.TTLMillis),
		)
		C.free(keyMemory)
		C.free(valueMemory)
		kind, data := decodeResult(h.library, result)
		done <- nativeResult{kind: kind, data: data}
	}()

	select {
	case result := <-done:
		if result.kind == SmithyFFIResultError {
			return nativeResult{}, &Error{Message: string(result.data)}
		}
		return result, nil
	case <-ctx.Done():
		return nativeResult{}, ctx.Err()
	}
}

func (h *nativeHandle) executeRaw(
	ctx context.Context,
	operation uint32,
	itemID ItemID,
	value []byte,
	options SetOptions,
) (nativeResult, error) {
	if !h.raw {
		return nativeResult{}, &Error{
			Operation: "execute raw",
			Message:   "native library does not support exact-item-ID operations",
		}
	}
	client, err := h.begin()
	if err != nil {
		return nativeResult{}, err
	}
	itemIDMemory := C.CBytes(itemID[:])
	valueMemory := C.CBytes(value)

	condition := SmithyFFISetConditionNone
	switch options.Condition {
	case IfAbsent:
		condition = SmithyFFISetConditionIfAbsent
	case IfPresent:
		condition = SmithyFFISetConditionIfPresent
	}
	ttlEnabled := boolByte(options.TTLMillis != 0)

	done := make(chan nativeResult, 1)
	go func() {
		defer h.end()
		result := C.openkache_go_execute_raw(
			h.library.ptr, client, C.uint32_t(operation),
			(*C.uint8_t)(itemIDMemory), C.size_t(len(itemID)),
			(*C.uint8_t)(valueMemory), C.size_t(len(value)),
			C.uint32_t(condition), C.uint8_t(ttlEnabled), C.uint64_t(options.TTLMillis),
		)
		C.free(itemIDMemory)
		C.free(valueMemory)
		kind, data := decodeResult(h.library, result)
		done <- nativeResult{kind: kind, data: data}
	}()

	select {
	case result := <-done:
		if result.kind == SmithyFFIResultError {
			return nativeResult{}, &Error{Message: string(result.data)}
		}
		return result, nil
	case <-ctx.Done():
		return nativeResult{}, ctx.Err()
	}
}

func (h *nativeHandle) state() uint32 {
	client, err := h.begin()
	if err != nil {
		return SmithyFFIConnectionStateClosed
	}
	defer h.end()
	return uint32(C.openkache_go_connection_state(h.library.ptr, client))
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
) (uint32, []byte) {
	if result == nil {
		return SmithyFFIResultError, []byte("native ABI returned a null result")
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
	C.openkache_go_result_free(library.ptr, result)
	if kind == SmithyFFIResultError && len(data) == 0 {
		data = []byte("native ABI returned an error without a payload")
	}
	return kind, data
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
