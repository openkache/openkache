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
from typing import Iterator

from ._generated.smithy_contract import (
    SmithyFFINamespaceDescriptor,
    SMITHY_FFI_ABI_VERSION,
    SMITHY_FFI_CONNECTION_STATE_CLOSED,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_OK,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EXPIRATION_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EVICTION_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_TTL_MS_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_EVICTION_OVERRIDE_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_EXPIRATION_OVERRIDE_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_NAMESPACE_ID_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_REVISION_OFFSET,
    SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES,
    SMITHY_FFI_RESULT_CONNECTED,
    SMITHY_FFI_RESULT_ERROR,
    SMITHY_FFI_SET_CONDITION_ANY,
)
from ._generated.smithy_native_abi import SMITHY_NATIVE_FUNCTIONS, SmithyNativeConnectOptions
from ._generated.smithy_native_abi import (
    _CLIENT_POINTER,
    _RESULT_POINTER,
    _U8,
    _U8_POINTER,
)


class NativeError(RuntimeError):
    """Failure reported by the Rust client-core ABI."""


_NamespaceDescriptor = SmithyFFINamespaceDescriptor


@dataclass(frozen=True, slots=True)
class NativeResult:
    """One native result with both semantic and wire-level discriminators.

    The iterator intentionally yields the historical ``(kind, payload)`` pair so generated
    adapters written against the previous transport surface continue to work while generic
    callers can inspect ``status`` without losing a newly modeled success token.
    """

    kind: int
    status: int
    payload: bytes
    client: _CLIENT_POINTER | None = None

    def __iter__(self) -> Iterator[int | bytes]:
        yield self.kind
        yield self.payload


if ctypes.sizeof(_NamespaceDescriptor) != SMITHY_FFI_NAMESPACE_DESCRIPTOR_SIZE_BYTES:
    raise RuntimeError("native namespace descriptor size does not match the Smithy contract")
if _NamespaceDescriptor.namespace_id.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_NAMESPACE_ID_OFFSET:
    raise RuntimeError("native namespace descriptor namespace_id offset does not match the Smithy contract")
if _NamespaceDescriptor.revision.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_REVISION_OFFSET:
    raise RuntimeError("native namespace descriptor revision offset does not match the Smithy contract")
if _NamespaceDescriptor.default_ttl_ms.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_TTL_MS_OFFSET:
    raise RuntimeError("native namespace descriptor default_ttl_ms offset does not match the Smithy contract")
if _NamespaceDescriptor.default_expiration.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EXPIRATION_OFFSET:
    raise RuntimeError("native namespace descriptor default_expiration offset does not match the Smithy contract")
if _NamespaceDescriptor.expiration_override.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_EXPIRATION_OVERRIDE_OFFSET:
    raise RuntimeError("native namespace descriptor expiration_override offset does not match the Smithy contract")
if _NamespaceDescriptor.default_eviction.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_DEFAULT_EVICTION_OFFSET:
    raise RuntimeError("native namespace descriptor default_eviction offset does not match the Smithy contract")
if _NamespaceDescriptor.eviction_override.offset != SMITHY_FFI_NAMESPACE_DESCRIPTOR_EVICTION_OVERRIDE_OFFSET:
    raise RuntimeError("native namespace descriptor eviction_override offset does not match the Smithy contract")


def _as_native_buffer(data: bytes) -> tuple[object | None, _U8_POINTER | None]:
    if not data:
        return None, None
    buffer = (_U8 * len(data)).from_buffer_copy(data)
    return buffer, ctypes.cast(buffer, _U8_POINTER)


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
        for attribute_name, (symbol_name, arguments, result) in SMITHY_NATIVE_FUNCTIONS.items():
            setattr(self, attribute_name, self._function(symbol_name, arguments, result))
        if self.abi_version() != SMITHY_FFI_ABI_VERSION:
            raise NativeError(
                f"unsupported OpenKache native ABI version {self.abi_version()}"
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
    ) -> NativeResult:
        if not result:
            raise NativeError("native client returned a null result")
        try:
            kind = int(self.result_kind(result))
            status = int(self.result_status(result))
            length = int(self.result_length(result))
            pointer = self.result_data(result)
            if length and not pointer:
                raise NativeError("native client returned a null payload pointer")
            payload = b"" if length == 0 else ctypes.string_at(pointer, length)
            client = self.result_take_client(result) if take_client else None
        finally:
            self.result_free(result)
        if kind == SMITHY_FFI_RESULT_ERROR:
            message = payload.decode("utf-8", errors="replace")
            raise NativeError(message or "native client operation failed")
        return NativeResult(kind=kind, status=status, payload=payload, client=client)


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
        compression_enabled: bool,
        compression_level: int,
        minimum_input_size: int,
        minimum_savings: int,
        encryption: int,
        connect_timeout_ms: int,
        request_timeout_ms: int,
        max_in_flight: int,
        retry_max_attempts: int,
        key_format: int = 0,
        native_path: str | os.PathLike[str] | None = None,
    ) -> NativeClient:
        api = _NativeApi(native_path)
        buffers = [
            _as_native_buffer(address),
            _as_native_buffer(server_name),
            _as_native_buffer(certificate),
            _as_native_buffer(client_certificate_chain),
            _as_native_buffer(client_private_key),
            _as_native_buffer(data_protection_key),
        ]
        options = SmithyNativeConnectOptions(
            buffers[0][1],
            len(address),
            buffers[1][1],
            len(server_name),
            buffers[2][1],
            len(certificate),
            buffers[3][1],
            len(client_certificate_chain),
            buffers[4][1],
            len(client_private_key),
            buffers[5][1],
            len(data_protection_key),
            1 if compression_enabled else 0,
            compression_level,
            minimum_input_size,
            minimum_savings,
            encryption,
            connect_timeout_ms,
            request_timeout_ms,
            retry_max_attempts,
            max_in_flight,
            key_format,
        )
        result = api.connect_with_options(ctypes.byref(options))
        native_result = api.read_result(result, take_client=True)
        if native_result.kind != SMITHY_FFI_RESULT_CONNECTED or not native_result.client:
            raise NativeError("native client did not return a connected handle")
        return cls(api, native_result.client)

    def execute(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        condition: int = SMITHY_FFI_SET_CONDITION_ANY,
        ttl_ms: int | None = None,
    ) -> NativeResult:
        return self._execute(
            self._api.execute,
            operation,
            key=key,
            value=value,
            condition=condition,
            ttl_ms=ttl_ms,
        )

    def execute_typed(
        self,
        operation: int,
        key_spec: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        condition: int = SMITHY_FFI_SET_CONDITION_ANY,
        ttl_ms: int | None = None,
    ) -> NativeResult:
        return self._execute_typed(
            self._api.execute_typed,
            operation,
            key_spec=key_spec,
            key=key,
            value=value,
            condition=condition,
            ttl_ms=ttl_ms,
        )

    def execute_raw(
        self,
        operation: int,
        *,
        item_id: bytes,
        value: bytes = b"",
        condition: int = SMITHY_FFI_SET_CONDITION_ANY,
        ttl_ms: int | None = None,
    ) -> NativeResult:
        return self._execute(
            self._api.execute_raw,
            operation,
            key=item_id,
            value=value,
            condition=condition,
            ttl_ms=ttl_ms,
        )

    def execute_with_options(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        set_flags: int = 0,
        ttl_ms: int = 0,
    ) -> NativeResult:
        return self._execute_with_options(
            self._api.execute_with_options,
            operation,
            key=key,
            value=value,
            set_flags=set_flags,
            ttl_ms=ttl_ms,
        )

    def execute_typed_with_options(
        self,
        operation: int,
        key_spec: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        set_flags: int = 0,
        ttl_ms: int = 0,
    ) -> NativeResult:
        return self._execute_typed_with_options(
            self._api.execute_typed_with_options,
            operation,
            key_spec=key_spec,
            key=key,
            value=value,
            set_flags=set_flags,
            ttl_ms=ttl_ms,
        )

    def execute_raw_with_options(
        self,
        operation: int,
        *,
        item_id: bytes,
        value: bytes = b"",
        set_flags: int = 0,
        ttl_ms: int = 0,
    ) -> NativeResult:
        return self._execute_with_options(
            self._api.execute_raw_with_options,
            operation,
            key=item_id,
            value=value,
            set_flags=set_flags,
            ttl_ms=ttl_ms,
        )

    def execute_scoped(
        self,
        operation: int,
        *,
        namespace_id: int,
        item_id: bytes,
        value: bytes = b"",
        set_flags: int = 0,
        ttl_ms: int = 0,
    ) -> NativeResult:
        item_buffer, item_pointer = _as_native_buffer(item_id)
        value_buffer, value_pointer = _as_native_buffer(value)
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = self._api.execute_scoped(
                handle,
                operation,
                namespace_id,
                item_pointer,
                len(item_id),
                value_pointer,
                len(value),
                set_flags,
                ttl_ms,
            )
            native_result = self._api.read_result(result)
            del item_buffer, value_buffer
            return native_result
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def namespace_open(
        self,
        *,
        name: bytes,
        create_if_missing: bool,
        policy_flags: int = 0,
        ttl_ms: int = 0,
    ) -> NativeResult:
        name_buffer, name_pointer = _as_native_buffer(name)
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = self._api.namespace_open(
                handle,
                name_pointer,
                len(name),
                1 if create_if_missing else 0,
                policy_flags,
                ttl_ms,
            )
            native_result = self._api.read_result(result)
            del name_buffer
            return native_result
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def namespace_update_policy(
        self,
        *,
        namespace_id: int,
        expected_revision: int,
        policy_flags: int,
        ttl_ms: int,
    ) -> NativeResult:
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = self._api.namespace_update_policy(
                handle,
                namespace_id,
                expected_revision,
                policy_flags,
                ttl_ms,
            )
            return self._api.read_result(result)
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def namespace_delete(self, *, namespace_id: int, expected_revision: int) -> None:
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = self._api.namespace_delete(handle, namespace_id, expected_revision)
            self._api.read_result(result)
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def decode_namespace_descriptor(
        self,
        payload: bytes,
    ) -> tuple[int, int, int, int, int, int, int]:
        payload_buffer, payload_pointer = _as_native_buffer(payload)
        decoded = _NamespaceDescriptor()
        status = self._api.namespace_descriptor_decode(
            payload_pointer,
            len(payload),
            ctypes.byref(decoded),
        )
        del payload_buffer
        if status != SMITHY_FFI_NAMESPACE_DESCRIPTOR_DECODE_OK:
            raise NativeError("native ABI returned an invalid namespace descriptor")
        return (
            int(decoded.namespace_id),
            int(decoded.revision),
            int(decoded.default_ttl_ms),
            int(decoded.default_expiration),
            int(decoded.expiration_override),
            int(decoded.default_eviction),
            int(decoded.eviction_override),
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
    ) -> NativeResult:
        key_buffer, key_pointer = _as_native_buffer(key)
        value_buffer, value_pointer = _as_native_buffer(value)
        # Keep ctypes buffers alive until the native call returns, while
        # allowing independent requests to use the core's bounded queue
        # concurrently.
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = function(
                handle,
                operation,
                key_pointer,
                len(key),
                value_pointer,
                len(value),
                condition,
                1 if ttl_ms is not None else 0,
                0 if ttl_ms is None else ttl_ms,
            )
            return self._api.read_result(result)
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def _execute_with_options(
        self,
        function: object,
        operation: int,
        *,
        key: bytes,
        value: bytes,
        set_flags: int,
        ttl_ms: int,
    ) -> NativeResult:
        key_buffer, key_pointer = _as_native_buffer(key)
        value_buffer, value_pointer = _as_native_buffer(value)
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = function(
                handle,
                operation,
                key_pointer,
                len(key),
                value_pointer,
                len(value),
                set_flags,
                ttl_ms,
            )
            return self._api.read_result(result)
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def _execute_typed(
        self,
        function: object,
        operation: int,
        *,
        key_spec: int,
        key: bytes,
        value: bytes,
        condition: int,
        ttl_ms: int | None,
    ) -> NativeResult:
        key_buffer, key_pointer = _as_native_buffer(key)
        value_buffer, value_pointer = _as_native_buffer(value)
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = function(
                handle,
                operation,
                key_spec,
                key_pointer,
                len(key),
                value_pointer,
                len(value),
                condition,
                1 if ttl_ms is not None else 0,
                0 if ttl_ms is None else ttl_ms,
            )
            return self._api.read_result(result)
        finally:
            with self._lifecycle:
                self._active_calls -= 1
                if self._active_calls == 0:
                    self._lifecycle.notify_all()

    def _execute_typed_with_options(
        self,
        function: object,
        operation: int,
        *,
        key_spec: int,
        key: bytes,
        value: bytes,
        set_flags: int,
        ttl_ms: int,
    ) -> NativeResult:
        key_buffer, key_pointer = _as_native_buffer(key)
        value_buffer, value_pointer = _as_native_buffer(value)
        with self._lifecycle:
            if not self._handle:
                raise NativeError("client is closed")
            handle = self._handle
            self._active_calls += 1
        try:
            result = function(
                handle,
                operation,
                key_spec,
                key_pointer,
                len(key),
                value_pointer,
                len(value),
                set_flags,
                ttl_ms,
            )
            return self._api.read_result(result)
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


__all__ = ["NativeClient", "NativeError"]
