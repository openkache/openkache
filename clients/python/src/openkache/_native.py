"""Small ctypes bridge for the shared Rust client-core C ABI.

The module deliberately contains no protocol, retry, TLS, or value-format logic.
Those concerns live in ``clients/core``; this file only owns pointer conversion
and native-resource lifetime.
"""

from __future__ import annotations

import ctypes
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from threading import Condition

from ._generated.smithy_contract import (
    SMITHY_FFI_ABI_VERSION,
    SMITHY_FFI_CONNECTION_STATE_CLOSED,
    SMITHY_FFI_RESULT_CONNECTED,
    SMITHY_FFI_RESULT_ERROR,
    SMITHY_FFI_SET_CONDITION_NONE,
    SMITHY_MUTATION_ID_BYTES,
)


class NativeError(RuntimeError):
    """Failure reported by the Rust client-core ABI."""

    def __init__(
        self,
        message: str,
        metadata: "ErrorMetadata | None" = None,
    ) -> None:
        super().__init__(message)
        self.metadata = metadata


@dataclass(frozen=True, slots=True)
class ErrorMetadata:
    """Structured metadata attached to a native operation failure."""

    code: int
    operation: int
    phase: int
    backend: int
    retryable: bool
    ambiguous: bool
    mutation_id: bytes | None


@dataclass(frozen=True, slots=True)
class MetricsSnapshot:
    """Point-in-time counters collected by one native client."""

    requests: int
    hits: int
    misses: int
    retries: int
    reconnects: int
    cancellations: int
    transport_errors: int
    protocol_errors: int
    bytes_sent: int
    bytes_received: int
    active_lanes: int


_U8 = ctypes.c_uint8
_U8_POINTER = ctypes.POINTER(_U8)
_RESULT_POINTER = ctypes.c_void_p
_CLIENT_POINTER = ctypes.c_void_p
_MAX_REQUEST_ID = (1 << 64) - 1


class _ConnectOptions(ctypes.Structure):
    _fields_ = [
        ("address", _U8_POINTER),
        ("address_length", ctypes.c_size_t),
        ("server_name", _U8_POINTER),
        ("server_name_length", ctypes.c_size_t),
        ("certificate", _U8_POINTER),
        ("certificate_length", ctypes.c_size_t),
        ("client_certificate_chain", _U8_POINTER),
        ("client_certificate_chain_length", ctypes.c_size_t),
        ("client_private_key", _U8_POINTER),
        ("client_private_key_length", ctypes.c_size_t),
        ("data_protection_key", _U8_POINTER),
        ("data_protection_key_length", ctypes.c_size_t),
        ("previous_data_protection_keys", _U8_POINTER),
        ("previous_data_protection_keys_length", ctypes.c_size_t),
        ("previous_data_protection_key_count", ctypes.c_size_t),
        ("compression_enabled", _U8),
        ("compression_level", ctypes.c_int32),
        ("minimum_input_size", ctypes.c_size_t),
        ("minimum_savings", ctypes.c_size_t),
        ("encryption", ctypes.c_uint32),
        ("connect_timeout_ms", ctypes.c_uint64),
        ("request_timeout_ms", ctypes.c_uint64),
        ("retry_max_attempts", ctypes.c_size_t),
        ("max_in_flight", ctypes.c_size_t),
    ]


class _ErrorMetadata(ctypes.Structure):
    _fields_ = [
        ("code", ctypes.c_uint32),
        ("operation", ctypes.c_uint32),
        ("phase", ctypes.c_uint32),
        ("backend", ctypes.c_uint32),
        ("retryable", _U8),
        ("ambiguous", _U8),
        ("mutation_id_length", _U8),
        ("reserved", _U8),
        ("mutation_id", _U8 * SMITHY_MUTATION_ID_BYTES),
    ]


class _MetricsSnapshot(ctypes.Structure):
    _fields_ = [
        ("requests", ctypes.c_uint64),
        ("hits", ctypes.c_uint64),
        ("misses", ctypes.c_uint64),
        ("retries", ctypes.c_uint64),
        ("reconnects", ctypes.c_uint64),
        ("cancellations", ctypes.c_uint64),
        ("transport_errors", ctypes.c_uint64),
        ("protocol_errors", ctypes.c_uint64),
        ("bytes_sent", ctypes.c_uint64),
        ("bytes_received", ctypes.c_uint64),
        ("active_lanes", ctypes.c_uint64),
    ]


def _as_native_buffer(
    data: bytes | bytearray,
) -> tuple[object | None, _U8_POINTER | None]:
    if not data:
        return None, None
    buffer = (_U8 * len(data)).from_buffer_copy(data)
    return buffer, ctypes.cast(buffer, _U8_POINTER)


def _zeroize_native_buffer(buffer: object | None) -> None:
    if buffer is None:
        return
    ctypes.memset(ctypes.addressof(buffer), 0, ctypes.sizeof(buffer))


def _library_candidates() -> tuple[Path, ...]:
    package_directory = Path(__file__).resolve().parent
    native_name = {
        "linux": "libopenkache_client_python_native.so",
        "darwin": "libopenkache_client_python_native.dylib",
        "win32": "openkache_client_python_native.dll",
    }.get(sys.platform)
    if native_name is None:
        return ()
    return (
        package_directory / native_name,
        package_directory.parent.parent / "native" / "target" / "release" / native_name,
        package_directory.parent.parent.parent.parent
        / "openkache"
        / "clients"
        / "python"
        / "native"
        / "target"
        / "release"
        / native_name,
    )


def _load_library(path: str | os.PathLike[str] | None) -> ctypes.CDLL:
    configured = os.environ.get("OPENKACHE_CLIENT_NATIVE")
    candidates = (Path(path),) if path is not None else ()
    if configured:
        candidates += (Path(configured),)
    candidates += _library_candidates()
    for candidate in candidates:
        if candidate.is_file():
            try:
                return ctypes.CDLL(str(candidate))
            except OSError as error:
                raise NativeError(f"failed to load native client {candidate}: {error}") from error
    searched = ", ".join(str(candidate) for candidate in candidates)
    raise NativeError(
        "OpenKache native client is not installed; build the package or set "
        f"OPENKACHE_CLIENT_NATIVE (searched: {searched or 'no platform artifact'})"
    )


class _NativeApi:
    """Configured ctypes signatures for one loaded native library."""

    def __init__(self, path: str | os.PathLike[str] | None = None) -> None:
        library = _load_library(path)
        self.library = library
        self.abi_version = self._function(
            "openkache_client_abi_version", (), ctypes.c_uint32
        )
        if self.abi_version() != SMITHY_FFI_ABI_VERSION:
            raise NativeError(
                f"unsupported OpenKache native ABI version {self.abi_version()}"
            )
        self.connect_options = self._function(
            "openkache_client_connect_with_options",
            (ctypes.POINTER(_ConnectOptions),),
            _RESULT_POINTER,
        )
        execute_with_request_id_arguments = (
            _CLIENT_POINTER,
            ctypes.c_uint64,
            ctypes.c_uint32,
            _U8_POINTER,
            ctypes.c_size_t,
            _U8_POINTER,
            ctypes.c_size_t,
            ctypes.c_uint32,
            _U8,
            ctypes.c_uint64,
        )
        self.execute_with_request_id = self._function(
            "openkache_client_execute_with_request_id",
            execute_with_request_id_arguments,
            _RESULT_POINTER,
        )
        self.execute_raw_with_request_id = self._function(
            "openkache_client_execute_raw_with_request_id",
            execute_with_request_id_arguments,
            _RESULT_POINTER,
        )
        execute_with_mutation_arguments = execute_with_request_id_arguments + (
            _U8_POINTER,
            ctypes.c_size_t,
        )
        self.execute_with_request_id_and_mutation_id = self._function(
            "openkache_client_execute_with_request_id_and_mutation_id",
            execute_with_mutation_arguments,
            _RESULT_POINTER,
        )
        self.execute_raw_with_request_id_and_mutation_id = self._function(
            "openkache_client_execute_raw_with_request_id_and_mutation_id",
            execute_with_mutation_arguments,
            _RESULT_POINTER,
        )
        self.cancel = self._function(
            "openkache_client_cancel",
            (_CLIENT_POINTER, ctypes.c_uint64),
            _U8,
        )
        self.metrics_snapshot = self._function(
            "openkache_client_metrics_snapshot",
            (_CLIENT_POINTER, ctypes.POINTER(_MetricsSnapshot)),
            _U8,
        )
        self.result_error_metadata = self._function(
            "openkache_client_result_error_metadata",
            (_RESULT_POINTER, ctypes.POINTER(_ErrorMetadata)),
            _U8,
        )
        self.connection_state = self._function(
            "openkache_client_connection_state", (_CLIENT_POINTER,), ctypes.c_uint32
        )
        self.result_kind = self._function(
            "openkache_client_result_kind", (_RESULT_POINTER,), ctypes.c_uint32
        )
        self.result_data = self._function(
            "openkache_client_result_data", (_RESULT_POINTER,), _U8_POINTER
        )
        self.result_length = self._function(
            "openkache_client_result_data_length",
            (_RESULT_POINTER,),
            ctypes.c_size_t,
        )
        self.result_take_client = self._function(
            "openkache_client_result_take_client",
            (_RESULT_POINTER,),
            _CLIENT_POINTER,
        )
        self.result_free = self._function(
            "openkache_client_result_free", (_RESULT_POINTER,), None
        )
        self.client_free = self._function(
            "openkache_client_free", (_CLIENT_POINTER,), None
        )

    def _function(
        self,
        name: str,
        arguments: tuple[object, ...],
        result: object,
    ) -> object:
        try:
            function = getattr(self.library, name)
        except AttributeError as error:
            raise NativeError(f"native client is missing ABI symbol {name}") from error
        function.argtypes = list(arguments)
        function.restype = result
        return function

    def read_result(
        self, result: _RESULT_POINTER, *, take_client: bool = False
    ) -> tuple[int, bytes, _CLIENT_POINTER | None]:
        if not result:
            raise NativeError("native client returned a null result")
        metadata = None
        try:
            kind = int(self.result_kind(result))
            length = int(self.result_length(result))
            pointer = self.result_data(result)
            if length and not pointer:
                raise NativeError("native client returned a null payload pointer")
            payload = b"" if length == 0 else ctypes.string_at(pointer, length)
            if kind == SMITHY_FFI_RESULT_ERROR:
                metadata_value = _ErrorMetadata()
                if self.result_error_metadata(result, ctypes.byref(metadata_value)):
                    metadata = ErrorMetadata(
                        code=int(metadata_value.code),
                        operation=int(metadata_value.operation),
                        phase=int(metadata_value.phase),
                        backend=int(metadata_value.backend),
                        retryable=bool(metadata_value.retryable),
                        ambiguous=bool(metadata_value.ambiguous),
                        mutation_id=(
                            bytes(
                                metadata_value.mutation_id[
                                    : int(metadata_value.mutation_id_length)
                                ]
                            )
                            if metadata_value.mutation_id_length
                            else None
                        ),
                    )
            client = self.result_take_client(result) if take_client else None
        finally:
            self.result_free(result)
        if kind == SMITHY_FFI_RESULT_ERROR:
            message = payload.decode("utf-8", errors="replace")
            raise NativeError(message or "native client operation failed", metadata)
        return kind, payload, client


class NativeClient:
    """Thread-safe handle for one core client worker."""

    def __init__(
        self,
        api: _NativeApi,
        handle: _CLIENT_POINTER,
    ) -> None:
        self._api = api
        self._handle = handle
        self._lifecycle = Condition()
        self._active_calls = 0
        self._next_request_id = 1

    @classmethod
    def connect(
        cls,
        *,
        address: bytes,
        server_name: bytes,
        certificate: bytes,
        client_certificate_chain: bytes = b"",
        client_private_key: bytes = b"",
        data_protection_key: bytes,
        previous_data_protection_keys: tuple[bytes, ...] = (),
        compression_enabled: bool,
        compression_level: int,
        minimum_input_size: int,
        minimum_savings: int,
        encryption: int,
        connect_timeout_ms: int,
        request_timeout_ms: int,
        max_in_flight: int,
        retry_max_attempts: int,
        native_path: str | os.PathLike[str] | None = None,
    ) -> NativeClient:
        api = _NativeApi(native_path)
        previous_keys = bytearray().join(previous_data_protection_keys)
        buffers = [
            _as_native_buffer(address),
            _as_native_buffer(server_name),
            _as_native_buffer(certificate),
            _as_native_buffer(client_certificate_chain),
            _as_native_buffer(client_private_key),
            _as_native_buffer(data_protection_key),
            _as_native_buffer(previous_keys),
        ]
        options = _ConnectOptions(
            address=buffers[0][1],
            address_length=len(address),
            server_name=buffers[1][1],
            server_name_length=len(server_name),
            certificate=buffers[2][1],
            certificate_length=len(certificate),
            client_certificate_chain=buffers[3][1],
            client_certificate_chain_length=len(client_certificate_chain),
            client_private_key=buffers[4][1],
            client_private_key_length=len(client_private_key),
            data_protection_key=buffers[5][1],
            data_protection_key_length=len(data_protection_key),
            previous_data_protection_keys=buffers[6][1],
            previous_data_protection_keys_length=len(previous_keys),
            previous_data_protection_key_count=len(previous_data_protection_keys),
            compression_enabled=1 if compression_enabled else 0,
            compression_level=compression_level,
            minimum_input_size=minimum_input_size,
            minimum_savings=minimum_savings,
            encryption=encryption,
            connect_timeout_ms=connect_timeout_ms,
            request_timeout_ms=request_timeout_ms,
            retry_max_attempts=retry_max_attempts,
            max_in_flight=max_in_flight,
        )
        try:
            result = api.connect_options(ctypes.byref(options))
            kind, _, handle = api.read_result(result, take_client=True)
            if kind != SMITHY_FFI_RESULT_CONNECTED or not handle:
                raise NativeError("native client did not return a connected handle")
            return cls(api, handle)
        finally:
            for buffer, _ in buffers:
                _zeroize_native_buffer(buffer)
            previous_keys[:] = b"\x00" * len(previous_keys)

    def execute(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        condition: int = SMITHY_FFI_SET_CONDITION_NONE,
        ttl_ms: int | None = None,
        mutation_id: bytes | None = None,
        request_id: int | None = None,
    ) -> tuple[int, bytes]:
        return self._execute(
            self._api.execute_with_request_id,
            operation,
            key=key,
            value=value,
            condition=condition,
            ttl_ms=ttl_ms,
            mutation_id=mutation_id,
            request_id=request_id,
        )

    def execute_raw(
        self,
        operation: int,
        *,
        item_id: bytes,
        value: bytes = b"",
        condition: int = SMITHY_FFI_SET_CONDITION_NONE,
        ttl_ms: int | None = None,
        mutation_id: bytes | None = None,
        request_id: int | None = None,
    ) -> tuple[int, bytes]:
        return self._execute(
            self._api.execute_raw_with_request_id,
            operation,
            key=item_id,
            value=value,
            condition=condition,
            ttl_ms=ttl_ms,
            mutation_id=mutation_id,
            request_id=request_id,
        )

    def connection_state(self) -> int:
        with self._lifecycle:
            if not self._handle:
                return SMITHY_FFI_CONNECTION_STATE_CLOSED
            handle = self._handle
            self._active_calls += 1
        try:
            return int(self._api.connection_state(handle))
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def _execute(
        self,
        function: object,
        operation: int,
        *,
        key: bytes,
        value: bytes,
        condition: int,
        ttl_ms: int | None,
        mutation_id: bytes | None,
        request_id: int | None,
    ) -> tuple[int, bytes]:
        key_buffer, key_pointer = _as_native_buffer(key)
        value_buffer, value_pointer = _as_native_buffer(value)
        mutation_buffer, mutation_pointer = _as_native_buffer(mutation_id or b"")
        # Keep ctypes buffers alive until the native call returns, while
        # allowing independent requests to use the core's bounded queue
        # concurrently.
        with self._lifecycle:
            if not self._handle:
                _zeroize_native_buffer(key_buffer)
                _zeroize_native_buffer(value_buffer)
                _zeroize_native_buffer(mutation_buffer)
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
            if request_id is None:
                request_id = self._allocate_request_id()
        try:
            selected_function = function
            if mutation_id is not None:
                selected_function = (
                    self._api.execute_with_request_id_and_mutation_id
                    if function is self._api.execute_with_request_id
                    else self._api.execute_raw_with_request_id_and_mutation_id
                )
            arguments = (
                handle,
                request_id,
                operation,
                key_pointer,
                len(key),
                value_pointer,
                len(value),
                condition,
                1 if ttl_ms is not None else 0,
                0 if ttl_ms is None else ttl_ms,
            )
            if mutation_id is not None:
                arguments += (mutation_pointer, len(mutation_id))
            result = selected_function(*arguments)
            kind, payload, _ = self._api.read_result(result)
            return kind, payload
        finally:
            _zeroize_native_buffer(key_buffer)
            _zeroize_native_buffer(value_buffer)
            _zeroize_native_buffer(mutation_buffer)
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def next_request_id(self) -> int:
        with self._lifecycle:
            return self._allocate_request_id()

    def _allocate_request_id(self) -> int:
        request_id = self._next_request_id
        if request_id <= 0 or request_id >= _MAX_REQUEST_ID:
            self._next_request_id = 1
        else:
            self._next_request_id = request_id + 1
        return request_id if request_id > 0 else 1

    def cancel(self, request_id: int) -> bool:
        with self._lifecycle:
            if not self._handle:
                return False
            handle = self._handle
            self._active_calls += 1
        try:
            return bool(self._api.cancel(handle, request_id))
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def metrics_snapshot(self) -> MetricsSnapshot:
        with self._lifecycle:
            if not self._handle:
                return MetricsSnapshot(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)
            handle = self._handle
            self._active_calls += 1
        try:
            value = _MetricsSnapshot()
            if not self._api.metrics_snapshot(handle, ctypes.byref(value)):
                raise NativeError("native client did not return metrics")
            return MetricsSnapshot(
                requests=int(value.requests),
                hits=int(value.hits),
                misses=int(value.misses),
                retries=int(value.retries),
                reconnects=int(value.reconnects),
                cancellations=int(value.cancellations),
                transport_errors=int(value.transport_errors),
                protocol_errors=int(value.protocol_errors),
                bytes_sent=int(value.bytes_sent),
                bytes_received=int(value.bytes_received),
                active_lanes=int(value.active_lanes),
            )
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def close(self) -> None:
        with self._lifecycle:
            handle, self._handle = self._handle, None
            while self._active_calls:
                self._lifecycle.wait()
        if handle:
            self._api.client_free(handle)

    def __del__(self) -> None:
        # Explicit ``close`` remains the deterministic lifecycle API, but a
        # best-effort finalizer prevents an abandoned native worker from
        # surviving ordinary reference-counted interpreter shutdown.
        try:
            self.close()
        except Exception:
            pass


__all__ = ["ErrorMetadata", "MetricsSnapshot", "NativeClient", "NativeError"]
