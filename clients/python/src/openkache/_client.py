"""Async Python API layered over the shared OpenKache client core."""

from __future__ import annotations

import asyncio
import base64
import json
import math
import secrets
import sys
from dataclasses import dataclass, field
from enum import IntEnum, StrEnum
from os import PathLike
from pathlib import Path
from typing import Any, Final, Iterable, Sequence

from ._generated import (
    SmithyDeleteInput,
    SmithyDeleteOutput,
    SmithyGetInput,
    SmithyGetOutput,
    SmithyOpenKacheApi,
    SmithyPingInput,
    SmithyPingOutput,
    SmithySetCondition,
    SmithySetInput,
    SmithySetOutcome,
    SmithySetOutput,
    SmithyStatsInput,
    SmithyStatsOutput,
    SmithySyncInput,
    SmithySyncOutput,
)
from ._generated.smithy_contract import (
    SMITHY_FFI_CONNECTION_STATE_CLOSED,
    SMITHY_FFI_CONNECTION_STATE_CONNECTED,
    SMITHY_FFI_CONNECTION_STATE_DISCONNECTED,
    SMITHY_FFI_CONNECTION_STATE_RECONNECTING,
    SMITHY_FFI_CONNECTION_STATE_UNKNOWN,
    SMITHY_FFI_BACKEND_NONE,
    SMITHY_FFI_OPERATION_GET_JSON,
    SMITHY_FFI_OPERATION_RECONNECT,
    SMITHY_FFI_OPERATION_SET_JSON,
    SMITHY_FFI_ERROR_CANCELLED,
    SMITHY_FFI_PHASE_UNKNOWN,
    SMITHY_FFI_RESULT_CREATED,
    SMITHY_FFI_RESULT_DELETED,
    SMITHY_FFI_RESULT_NOT_DELETED,
    SMITHY_FFI_RESULT_NOT_FOUND,
    SMITHY_FFI_RESULT_NOT_STORED,
    SMITHY_FFI_RESULT_REPLACED,
    SMITHY_FFI_RESULT_VALUE,
    SMITHY_FFI_SET_CONDITION_IF_ABSENT,
    SMITHY_FFI_SET_CONDITION_IF_PRESENT,
    SMITHY_FFI_SET_CONDITION_NONE,
    SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
    SMITHY_DEFAULT_MAX_IN_FLIGHT,
    SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS,
    SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
    SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
    SMITHY_DEFAULT_ZSTANDARD_LEVEL,
    SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX,
    SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN,
    SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
    SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
    SMITHY_CLIENT_CERTIFICATE_PEM_TYPE,
    SMITHY_CLIENT_DEFAULT_SERVER_NAME,
    SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE,
    SMITHY_ITEM_ID_BYTES,
    SMITHY_MUTATION_ID_BYTES,
    SMITHY_MAX_VALUE_BYTES,
    SMITHY_OPCODE_DELETE,
    SMITHY_OPCODE_GET,
    SMITHY_OPCODE_PING,
    SMITHY_OPCODE_SET,
    SMITHY_OPCODE_STATS,
    SMITHY_OPCODE_SYNC,
    SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES,
    SMITHY_VALUE_ENCRYPTION_COMPACT,
    SMITHY_VALUE_ENCRYPTION_ROBUST,
)
from ._native import (
    ErrorMetadata,
    MetricsSnapshot,
    NativeClient as _NativeClient,
    NativeError,
)


_UINT64_MAX: Final = (1 << 64) - 1
_SIZE_T_MAX: Final = (sys.maxsize << 1) | 1
_BINARY64_SIGNIFICAND_BITS: Final = 53
_BINARY64_MAX_INTEGER_BITS: Final = 1024


class OpenKacheError(RuntimeError):
    """Base error raised by the Python client."""

    def __init__(
        self,
        message: str,
        *,
        metadata: ErrorMetadata | None = None,
    ) -> None:
        super().__init__(message)
        self.metadata = metadata


class OpenKacheCancelledError(asyncio.CancelledError):
    """Cancellation carrying the caller operation and mutation retry metadata."""

    def __init__(self, *, metadata: ErrorMetadata) -> None:
        super().__init__("client operation canceled")
        self.metadata = metadata


class OpenKacheValueError(OpenKacheError, ValueError):
    """Invalid key, value, option, or value-format input."""


class ConnectionState(StrEnum):
    """Best-effort native connection state."""

    CONNECTED = "Connected"
    RECONNECTING = "Reconnecting"
    DISCONNECTED = "Disconnected"
    CLOSED = "Closed"
    UNKNOWN = "Unknown"


class Encryption(IntEnum):
    """Authenticated value-encryption profile implemented by the shared core."""

    COMPACT = SMITHY_VALUE_ENCRYPTION_COMPACT
    ROBUST = SMITHY_VALUE_ENCRYPTION_ROBUST


@dataclass(frozen=True, slots=True)
class ClientIdentity:
    """Mutual-TLS certificate chain and private key."""

    certificate_chain: tuple[bytes, ...]
    private_key: bytes = field(repr=False)

    def __init__(
        self,
        certificate_chain: Sequence[bytes | bytearray | memoryview],
        private_key: bytes | bytearray | memoryview,
    ) -> None:
        chain = tuple(_as_bytes(certificate, "certificate_chain entry") for certificate in certificate_chain)
        if not chain:
            raise OpenKacheValueError("certificate_chain must not be empty")
        if any(not certificate or not certificate.strip() for certificate in chain):
            raise OpenKacheValueError(
                "certificate_chain entries must contain certificate bytes"
            )
        object.__setattr__(self, "certificate_chain", chain)
        object.__setattr__(self, "private_key", _as_bytes(private_key, "private_key"))
        if not self.private_key or not self.private_key.strip():
            raise OpenKacheValueError("private_key must contain key bytes")


@dataclass(frozen=True, slots=True)
class DataProtectionKeyRing:
    """Active data-protection key plus a bounded retired-key window."""

    active: bytes
    previous: tuple[bytes, ...] = ()

    def __post_init__(self) -> None:
        active = _as_bytes(self.active, "key_ring.active")
        if len(active) != SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES:
            raise OpenKacheValueError(
                "key_ring.active must contain exactly "
                f"{SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes"
            )
        previous = tuple(
            _as_bytes(key, "key_ring.previous entry") for key in self.previous
        )
        if len(previous) > SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS:
            raise OpenKacheValueError(
                "key_ring.previous may contain at most "
                f"{SMITHY_MAX_PREVIOUS_DATA_PROTECTION_KEYS} keys"
            )
        if any(len(key) != SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES for key in previous):
            raise OpenKacheValueError(
                "each key_ring.previous entry must contain exactly "
                f"{SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes"
            )
        object.__setattr__(self, "active", active)
        object.__setattr__(self, "previous", previous)


@dataclass(frozen=True, slots=True)
class CompressionOptions:
    """Zstandard policy applied before core encryption."""

    enabled: bool = True
    level: int = SMITHY_DEFAULT_ZSTANDARD_LEVEL
    minimum_input_size: int = SMITHY_DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES
    minimum_savings: int = SMITHY_DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES

    def __post_init__(self) -> None:
        if not isinstance(self.enabled, bool):
            raise OpenKacheValueError("compression.enabled must be a bool")
        _positive_or_zero(self.level, "compression.level", allow_zero=False)
        if not (
            SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN
            <= self.level
            <= SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX
        ):
            raise OpenKacheValueError(
                "compression.level must be between "
                f"{SMITHY_DEFAULT_ZSTANDARD_LEVEL_MIN} and "
                f"{SMITHY_DEFAULT_ZSTANDARD_LEVEL_MAX}"
            )
        _positive_or_zero(
            self.minimum_input_size,
            "compression.minimum_input_size",
            maximum=_SIZE_T_MAX,
        )
        _positive_or_zero(
            self.minimum_savings,
            "compression.minimum_savings",
            maximum=_SIZE_T_MAX,
        )


@dataclass(frozen=True, slots=True)
class ClientTimeouts:
    """Connection and complete-request deadlines in milliseconds."""

    connect_ms: int = SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS
    request_ms: int = SMITHY_DEFAULT_REQUEST_TIMEOUT_MILLISECONDS

    def __post_init__(self) -> None:
        _positive_or_zero(
            self.connect_ms,
            "timeouts.connect_ms",
            allow_zero=False,
            maximum=_UINT64_MAX,
        )
        _positive_or_zero(
            self.request_ms,
            "timeouts.request_ms",
            allow_zero=False,
            maximum=_UINT64_MAX,
        )


@dataclass(frozen=True, slots=True)
class SetOptions:
    """Atomic existence condition and optional positive TTL."""

    condition: SmithySetCondition | str | None = None
    ttl_ms: int | None = None
    mutation_id: bytes | bytearray | memoryview | None = None

    def __post_init__(self) -> None:
        condition = self.condition
        if isinstance(condition, str):
            try:
                condition = SmithySetCondition(condition)
            except ValueError as error:
                raise OpenKacheValueError(
                    "condition must be 'if_absent' or 'if_present'"
                ) from error
            object.__setattr__(self, "condition", condition)
        elif condition is not None and not isinstance(condition, SmithySetCondition):
            raise OpenKacheValueError(
                "condition must be 'if_absent', 'if_present', or None"
            )
        if self.ttl_ms is not None:
            _positive_or_zero(
                self.ttl_ms,
                "ttl_ms",
                allow_zero=False,
                maximum=_UINT64_MAX,
            )
        if self.mutation_id is not None:
            mutation_id = _as_bytes(self.mutation_id, "mutation_id")
            if len(mutation_id) != SMITHY_MUTATION_ID_BYTES:
                raise OpenKacheValueError(
                    f"mutation_id must contain exactly {SMITHY_MUTATION_ID_BYTES} bytes"
                )
            object.__setattr__(self, "mutation_id", mutation_id)

    @property
    def _condition_code(self) -> int:
        if self.condition is None:
            return SMITHY_FFI_SET_CONDITION_NONE
        if self.condition is SmithySetCondition.IF_ABSENT:
            return SMITHY_FFI_SET_CONDITION_IF_ABSENT
        return SMITHY_FFI_SET_CONDITION_IF_PRESENT


@dataclass(frozen=True, slots=True)
class ServerStats:
    """Validated server statistics response."""

    storage: str
    workers: tuple[str, ...]

    @classmethod
    def from_json(cls, text: str) -> ServerStats:
        try:
            value = json.loads(text)
        except json.JSONDecodeError as error:
            raise OpenKacheError(f"STATS decoding failed: {error}") from error
        if not isinstance(value, dict):
            raise OpenKacheError("STATS response must be an object")
        storage = value.get("storage")
        workers = value.get("workers")
        if not isinstance(storage, str):
            raise OpenKacheError("STATS response.storage must be a string")
        if (
            not isinstance(workers, list)
            or not all(isinstance(worker, str) for worker in workers)
        ):
            raise OpenKacheError("STATS response.workers must be a string array")
        return cls(storage=storage, workers=tuple(workers))


SetCondition = SmithySetCondition
SetOutcome = SmithySetOutcome


class OpenKacheClient:
    """Protected application-key client with asyncio-friendly operations.

    ``clients/core`` owns QUIC, TLS, retries, key derivation, compression, and
    authenticated value protection. Python only converts native objects and
    schedules blocking ctypes calls on worker threads.
    """

    def __init__(self, native: _NativeClient) -> None:
        self._native = native
        self._closed = False
        self._raw: RawClient | None = None

    @classmethod
    async def connect(
        cls,
        address: str,
        *,
        certificate: bytes | bytearray | memoryview | str | PathLike[str],
        data_protection_key: bytes | bytearray | memoryview | None = None,
        server_name: str | None = None,
        identity: ClientIdentity | None = None,
        compression: CompressionOptions | None = None,
        encryption: Encryption = Encryption.ROBUST,
        timeouts: ClientTimeouts | None = None,
        max_in_flight: int = SMITHY_DEFAULT_MAX_IN_FLIGHT,
        retry_max_attempts: int = SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
        key_ring: DataProtectionKeyRing | None = None,
        native_path: str | PathLike[str] | None = None,
    ) -> OpenKacheClient:
        try:
            settings = await asyncio.to_thread(
                _connection_settings,
                address,
                certificate=certificate,
                data_protection_key=data_protection_key,
                key_ring=key_ring,
                server_name=server_name,
                identity=identity,
                compression=compression,
                encryption=encryption,
                timeouts=timeouts,
                max_in_flight=max_in_flight,
                retry_max_attempts=retry_max_attempts,
                native_path=native_path,
            )
            native = await asyncio.to_thread(_NativeClient.connect, **settings)
        except NativeError as error:
            raise _map_native_error(error) from error
        except OSError as error:
            raise OpenKacheError(str(error)) from error
        return cls(native)

    @classmethod
    def connect_sync(cls, address: str, **kwargs: Any) -> OpenKacheClient:
        """Synchronous convenience wrapper for scripts without an event loop."""

        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(cls.connect(address, **kwargs))
        raise OpenKacheError("connect_sync cannot run inside an active event loop")

    async def ping(self) -> None:
        self._assert_open()
        await self._execute(SMITHY_OPCODE_PING)

    async def get(self, key: str | bytes | bytearray | memoryview) -> Any | None:
        """Gets a JSON value, or ``None`` when the key is absent."""

        self._assert_open()
        payload = await self._value_operation(SMITHY_FFI_OPERATION_GET_JSON, key)
        if payload is None:
            return None
        try:
            return json.loads(payload)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise OpenKacheError(f"value decoding failed: {error}") from error

    async def set(
        self,
        key: str | bytes | bytearray | memoryview,
        value: Any,
        options: SetOptions | None = None,
    ) -> SmithySetOutcome:
        """Canonicalizes, protects, and stores a JSON-compatible value."""

        self._assert_open()
        payload = _json_bytes(value)
        return await self._set_operation(SMITHY_FFI_OPERATION_SET_JSON, key, payload, options)

    async def get_raw(
        self, key: str | bytes | bytearray | memoryview
    ) -> bytes | None:
        """Gets exact decrypted Raw bytes, or ``None`` when absent."""

        self._assert_open()
        return await self._value_operation(SMITHY_OPCODE_GET, key)

    async def set_raw(
        self,
        key: str | bytes | bytearray | memoryview,
        value: bytes | bytearray | memoryview,
        options: SetOptions | None = None,
    ) -> SmithySetOutcome:
        """Stores exact bytes through the core Raw value format."""

        self._assert_open()
        return await self._set_operation(
            SMITHY_OPCODE_SET, key, _value_bytes(value), options
        )

    async def delete(
        self,
        key: str | bytes | bytearray | memoryview,
        options: SetOptions | None = None,
    ) -> bool:
        self._assert_open()
        kind, _ = await self._execute(
            SMITHY_OPCODE_DELETE,
            key=_key_bytes(key),
            options=options,
        )
        return _delete_outcome(kind)

    async def stats(self) -> ServerStats:
        return ServerStats.from_json(await self.stats_json())

    async def stats_json(self) -> str:
        self._assert_open()
        kind, payload = await self._execute(SMITHY_OPCODE_STATS)
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(f"STATS returned unexpected native result {kind}")
        try:
            return payload.decode("utf-8")
        except UnicodeDecodeError as error:
            raise OpenKacheError(f"STATS response is not UTF-8: {error}") from error

    async def sync(self) -> None:
        self._assert_open()
        await self._execute(SMITHY_OPCODE_SYNC)

    async def reconnect(self) -> None:
        self._assert_open()
        await self._execute(SMITHY_FFI_OPERATION_RECONNECT)

    def connection_state(self) -> ConnectionState:
        if self._closed:
            return ConnectionState.CLOSED
        try:
            return {
                SMITHY_FFI_CONNECTION_STATE_CONNECTED: ConnectionState.CONNECTED,
                SMITHY_FFI_CONNECTION_STATE_RECONNECTING: ConnectionState.RECONNECTING,
                SMITHY_FFI_CONNECTION_STATE_DISCONNECTED: ConnectionState.DISCONNECTED,
                SMITHY_FFI_CONNECTION_STATE_CLOSED: ConnectionState.CLOSED,
                SMITHY_FFI_CONNECTION_STATE_UNKNOWN: ConnectionState.UNKNOWN,
            }[self._native.connection_state()]
        except KeyError as error:
            raise OpenKacheError("native client returned an unknown connection state") from error

    def metrics_snapshot(self) -> MetricsSnapshot:
        """Returns the native request, retry, error, byte, and lane counters."""

        self._assert_open()
        return self._native.metrics_snapshot()

    def cancel(self, request_id: int) -> bool:
        """Cancels a native operation previously started with its request ID."""

        self._assert_open()
        return self._native.cancel(request_id)

    @property
    def raw(self) -> RawClient:
        """Smithy-shaped exact-item-ID API sharing this protected connection."""

        self._assert_open()
        if self._raw is None:
            self._raw = RawClient(self)
        return self._raw

    async def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        await asyncio.to_thread(self._native.close)

    async def __aenter__(self) -> OpenKacheClient:
        self._assert_open()
        return self

    async def __aexit__(self, *_: object) -> None:
        await self.close()

    def _assert_open(self) -> None:
        if self._closed:
            raise OpenKacheError("client is closed")

    async def _cancel_native(self, request_id: int) -> None:
        """Deliver cancellation without blocking the asyncio event loop."""

        await asyncio.to_thread(self._native.cancel, request_id)

    async def _execute(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        options: SetOptions | None = None,
        request_id: int | None = None,
    ) -> tuple[int, bytes]:
        self._assert_open()
        selected = options or SetOptions()
        selected = _mutation_options(operation, selected)
        request_id = request_id if request_id is not None else self._native.next_request_id()
        try:
            return await asyncio.to_thread(
                self._native.execute,
                operation,
                key=key,
                value=value,
                condition=selected._condition_code,
                ttl_ms=selected.ttl_ms,
                mutation_id=selected.mutation_id,
                request_id=request_id,
            )
        except asyncio.CancelledError:
            await self._cancel_native(request_id)
            raise OpenKacheCancelledError(
                metadata=ErrorMetadata(
                    code=SMITHY_FFI_ERROR_CANCELLED,
                    operation=operation,
                    phase=SMITHY_FFI_PHASE_UNKNOWN,
                    backend=SMITHY_FFI_BACKEND_NONE,
                    retryable=selected.mutation_id is not None,
                    ambiguous=selected.mutation_id is not None,
                    mutation_id=selected.mutation_id,
                )
            ) from None
        except NativeError as error:
            raise _map_native_error(error) from error

    async def _value_operation(
        self,
        operation: int,
        key: str | bytes | bytearray | memoryview,
        operation_name: str = "GET",
    ) -> bytes | None:
        kind, payload = await self._execute(operation, key=_key_bytes(key))
        if kind == SMITHY_FFI_RESULT_NOT_FOUND:
            return None
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(
                f"{operation_name} returned unexpected native result {kind}"
            )
        return payload

    async def _execute_raw(
        self,
        operation: int,
        *,
        item_id: bytes,
        value: bytes = b"",
        options: SetOptions | None = None,
        request_id: int | None = None,
    ) -> tuple[int, bytes]:
        self._assert_open()
        selected = options or SetOptions()
        selected = _mutation_options(operation, selected)
        request_id = request_id if request_id is not None else self._native.next_request_id()
        try:
            return await asyncio.to_thread(
                self._native.execute_raw,
                operation,
                item_id=item_id,
                value=value,
                condition=selected._condition_code,
                ttl_ms=selected.ttl_ms,
                mutation_id=selected.mutation_id,
                request_id=request_id,
            )
        except asyncio.CancelledError:
            await self._cancel_native(request_id)
            raise OpenKacheCancelledError(
                metadata=ErrorMetadata(
                    code=SMITHY_FFI_ERROR_CANCELLED,
                    operation=operation,
                    phase=SMITHY_FFI_PHASE_UNKNOWN,
                    backend=SMITHY_FFI_BACKEND_NONE,
                    retryable=selected.mutation_id is not None,
                    ambiguous=selected.mutation_id is not None,
                    mutation_id=selected.mutation_id,
                )
            ) from None
        except NativeError as error:
            raise _map_native_error(error) from error

    async def _set_operation(
        self,
        operation: int,
        key: str | bytes | bytearray | memoryview,
        value: bytes,
        options: SetOptions | None,
    ) -> SmithySetOutcome:
        kind, _ = await self._execute(
            operation,
            key=_key_bytes(key),
            value=value,
            options=options,
        )
        return _set_outcome(kind)


class RawClient(SmithyOpenKacheApi):
    """Smithy-generated exact item-ID operations over a shared connection."""

    def __init__(self, owner: OpenKacheClient) -> None:
        self._owner = owner

    async def ping(self, input: SmithyPingInput | None = None) -> SmithyPingOutput:
        del input
        await self._owner.ping()
        return SmithyPingOutput()

    async def get(self, input: SmithyGetInput) -> SmithyGetOutput:
        item_id = _item_id(input.item_id)
        kind, payload = await self._owner._execute_raw(
            SMITHY_OPCODE_GET, item_id=item_id
        )
        if kind == SMITHY_FFI_RESULT_NOT_FOUND:
            return SmithyGetOutput()
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(f"GET returned unexpected native result {kind}")
        return SmithyGetOutput(value=payload)

    async def set(self, input: SmithySetInput) -> SmithySetOutput:
        options = SetOptions(
            input.condition,
            input.ttl_milliseconds,
            input.mutation_id,
        )
        kind, _ = await self._owner._execute_raw(
            SMITHY_OPCODE_SET,
            item_id=_item_id(input.item_id),
            value=_value_bytes(input.value),
            options=options,
        )
        return SmithySetOutput(outcome=_set_outcome(kind))

    async def delete(self, input: SmithyDeleteInput) -> SmithyDeleteOutput:
        kind, _ = await self._owner._execute_raw(
            SMITHY_OPCODE_DELETE,
            item_id=_item_id(input.item_id),
            options=SetOptions(mutation_id=input.mutation_id),
        )
        return SmithyDeleteOutput(deleted=_delete_outcome(kind))

    async def stats(self, input: SmithyStatsInput | None = None) -> SmithyStatsOutput:
        del input
        return SmithyStatsOutput(json=await self._owner.stats_json())

    async def sync(self, input: SmithySyncInput | None = None) -> SmithySyncOutput:
        del input
        await self._owner.sync()
        return SmithySyncOutput()

    async def close(self) -> None:
        await self._owner.close()


Client = OpenKacheClient


def _mutation_options(operation: int, options: SetOptions) -> SetOptions:
    """Adds one token to every mutation when the caller did not supply one."""

    if (
        operation not in (SMITHY_OPCODE_SET, SMITHY_OPCODE_DELETE, SMITHY_FFI_OPERATION_SET_JSON)
        or options.mutation_id is not None
    ):
        return options
    return SetOptions(
        condition=options.condition,
        ttl_ms=options.ttl_ms,
        mutation_id=secrets.token_bytes(SMITHY_MUTATION_ID_BYTES),
    )


def _connection_settings(
    address: str,
    *,
    certificate: bytes | bytearray | memoryview | str | PathLike[str],
    data_protection_key: bytes | bytearray | memoryview | None,
    key_ring: DataProtectionKeyRing | None,
    server_name: str | None,
    identity: ClientIdentity | None,
    compression: CompressionOptions | None,
    encryption: Encryption,
    timeouts: ClientTimeouts | None,
    max_in_flight: int,
    retry_max_attempts: int,
    native_path: str | PathLike[str] | None,
) -> dict[str, Any]:
    native_address, host = _resolve_address(address)
    certificate_bytes = _as_file_or_bytes(certificate, "certificate")
    if key_ring is not None and data_protection_key is not None:
        raise OpenKacheValueError(
            "provide either data_protection_key or key_ring, not both"
        )
    if key_ring is None:
        if data_protection_key is None:
            raise OpenKacheValueError(
                "data_protection_key or key_ring must be supplied"
            )
        protection_key = _as_bytes(data_protection_key, "data_protection_key")
        if len(protection_key) != SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES:
            raise OpenKacheValueError(
                "data_protection_key must contain exactly "
                f"{SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes"
            )
        previous_keys: tuple[bytes, ...] = ()
    else:
        protection_key = key_ring.active
        previous_keys = key_ring.previous
    compression = compression or CompressionOptions()
    timeouts = timeouts or ClientTimeouts()
    if not isinstance(encryption, Encryption):
        raise OpenKacheValueError("encryption must be an Encryption value")
    _positive_or_zero(
        max_in_flight,
        "max_in_flight",
        allow_zero=False,
        maximum=_SIZE_T_MAX,
    )
    _positive_or_zero(
        retry_max_attempts,
        "retry_max_attempts",
        allow_zero=False,
        maximum=_SIZE_T_MAX,
    )
    if not isinstance(native_path, (str, PathLike)) and native_path is not None:
        raise OpenKacheValueError("native_path must be a path or None")
    if isinstance(native_path, str) and not native_path:
        raise OpenKacheValueError("native_path must not be empty")
    if server_name is not None and (not isinstance(server_name, str) or not server_name):
        raise OpenKacheValueError("server_name must be a non-empty string or None")
    identity_chain = b""
    identity_key = b""
    if identity is not None:
        identity_chain = _certificate_chain_bytes(identity.certificate_chain)
        identity_key = identity.private_key
    try:
        native_address_bytes = native_address.encode("ascii")
        server_name_bytes = (
            server_name or SMITHY_CLIENT_DEFAULT_SERVER_NAME or host
        ).encode("utf-8")
    except UnicodeEncodeError as error:
        raise OpenKacheValueError("server_name must contain valid Unicode text") from error
    return {
        "address": native_address_bytes,
        "server_name": server_name_bytes,
        "certificate": certificate_bytes,
        "client_certificate_chain": identity_chain,
        "client_private_key": identity_key,
        "data_protection_key": protection_key,
        "previous_data_protection_keys": previous_keys,
        "compression_enabled": compression.enabled,
        "compression_level": compression.level,
        "minimum_input_size": compression.minimum_input_size,
        "minimum_savings": compression.minimum_savings,
        "encryption": int(encryption),
        "connect_timeout_ms": timeouts.connect_ms,
        "request_timeout_ms": timeouts.request_ms,
        "max_in_flight": max_in_flight,
        "retry_max_attempts": retry_max_attempts,
        "native_path": native_path,
    }


def _resolve_address(address: str) -> tuple[str, str]:
    if not isinstance(address, str) or not address:
        raise OpenKacheValueError("address must be a non-empty host:port string")
    if address.startswith("["):
        closing = address.find("]")
        if closing < 0 or closing + 1 >= len(address) or address[closing + 1] != ":":
            raise OpenKacheValueError("address must use [ipv6]:port syntax")
        host, port_text = address[1:closing], address[closing + 2 :]
    else:
        try:
            host, port_text = address.rsplit(":", 1)
        except ValueError as error:
            raise OpenKacheValueError("address must include a UDP port") from error
    if not host or not port_text.isdecimal():
        raise OpenKacheValueError("address must contain a host and numeric UDP port")
    port = int(port_text)
    if not 1 <= port <= 65_535:
        raise OpenKacheValueError("address port must be between 1 and 65535")
    # Keep the authority intact. The shared core resolves hostnames once and
    # retains every address for reconnect rotation; resolving here would pin
    # Python clients to whichever address the interpreter returned first.
    return address, host


def _as_file_or_bytes(
    value: bytes | bytearray | memoryview | str | PathLike[str], name: str
) -> bytes:
    if isinstance(value, (str, PathLike)):
        try:
            return Path(value).read_bytes()
        except OSError as error:
            raise OpenKacheValueError(f"{name} could not be read: {error}") from error
    return _as_bytes(value, name)


def _as_bytes(value: bytes | bytearray | memoryview, name: str) -> bytes:
    if isinstance(value, bytes):
        return value
    if isinstance(value, (bytearray, memoryview)):
        return bytes(value)
    raise OpenKacheValueError(f"{name} must be bytes-like")


def _value_bytes(value: bytes | bytearray | memoryview) -> bytes:
    payload = _as_bytes(value, "value")
    if len(payload) > SMITHY_MAX_VALUE_BYTES:
        raise OpenKacheValueError(
            f"value exceeds the protocol maximum of {SMITHY_MAX_VALUE_BYTES} bytes"
        )
    return payload


def _key_bytes(value: str | bytes | bytearray | memoryview) -> bytes:
    if isinstance(value, str):
        try:
            encoded = value.encode("utf-8")
        except UnicodeEncodeError as error:
            raise OpenKacheValueError("key must contain valid UTF-8 text") from error
    else:
        encoded = _as_bytes(value, "key")
    if not encoded:
        raise OpenKacheValueError("key must not be empty")
    return encoded


def _item_id(value: bytes | bytearray | memoryview) -> bytes:
    item_id = _as_bytes(value, "item_id")
    if len(item_id) != SMITHY_ITEM_ID_BYTES:
        raise OpenKacheValueError(
            f"item_id must contain exactly {SMITHY_ITEM_ID_BYTES} bytes"
        )
    return item_id


def _certificate_chain_bytes(chain: Iterable[bytes]) -> bytes:
    certificates = tuple(_as_bytes(certificate, "certificate_chain entry") for certificate in chain)
    if not certificates:
        raise OpenKacheValueError("certificate_chain must not be empty")
    if any(not certificate or not certificate.strip() for certificate in certificates):
        raise OpenKacheValueError(
            "certificate_chain entries must contain certificate bytes"
        )
    if len(certificates) == 1:
        return certificates[0]
    pem_entries = []
    pem_begin = f"-----BEGIN {SMITHY_CLIENT_CERTIFICATE_PEM_TYPE}-----".encode(
        "ascii"
    )
    for certificate in certificates:
        trimmed = certificate.lstrip()
        if trimmed.startswith(pem_begin):
            pem_entries.append(trimmed.rstrip())
            continue
        encoded = base64.b64encode(certificate)
        body = b"\n".join(
            encoded[offset : offset + 64] for offset in range(0, len(encoded), 64)
        )
        pem_entries.append(
            f"-----BEGIN {SMITHY_CLIENT_CERTIFICATE_PEM_TYPE}-----\n".encode("ascii")
            + body
            + f"\n-----END {SMITHY_CLIENT_CERTIFICATE_PEM_TYPE}-----\n".encode("ascii")
        )
    return b"\n".join(pem_entries)


def _json_bytes(value: Any) -> bytes:
    """Encode a Python value for the core's JSON-input FFI operation.

    This is only the ABI transport representation. The core parses these bytes into its
    ``JsonValue`` model and owns canonical JSON serialization, value-format framing, compression,
    and encryption. Keeping the conversion here lets the ctypes boundary accept one stable byte
    buffer without making Python a second value-format implementation.
    """
    try:
        _validate_json_value(value, "$", set())
        text = json.dumps(
            value,
            ensure_ascii=False,
            separators=(",", ":"),
            allow_nan=False,
        )
        payload = text.encode("utf-8")
    except (TypeError, ValueError, UnicodeEncodeError) as error:
        raise OpenKacheValueError(f"value is not JSON-compatible: {error}") from error
    if len(payload) > SMITHY_MAX_VALUE_BYTES:
        raise OpenKacheValueError(
            f"value exceeds the protocol maximum of {SMITHY_MAX_VALUE_BYTES} bytes"
        )
    return payload


def _validate_json_value(value: Any, path: str, ancestors: set[int]) -> None:
    if value is None or isinstance(value, (str, bool)):
        return
    if isinstance(value, int):
        if not _is_exact_binary64_integer(value):
            raise OpenKacheValueError(f"{path} exceeds the exact JSON number range")
        return
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            raise OpenKacheValueError(f"{path} must be finite")
        return
    if isinstance(value, (list, tuple)):
        identity = id(value)
        if identity in ancestors:
            raise OpenKacheValueError(f"{path} contains a cyclic reference")
        ancestors.add(identity)
        try:
            for index, child in enumerate(value):
                _validate_json_value(child, f"{path}[{index}]", ancestors)
        finally:
            ancestors.remove(identity)
        return
    if isinstance(value, dict):
        identity = id(value)
        if identity in ancestors:
            raise OpenKacheValueError(f"{path} contains a cyclic reference")
        ancestors.add(identity)
        try:
            for key, child in value.items():
                if not isinstance(key, str):
                    raise OpenKacheValueError(f"{path} has a non-string object key")
                _validate_json_value(child, f"{path}.{key}", ancestors)
        finally:
            ancestors.remove(identity)
        return
    raise OpenKacheValueError(f"{path} contains unsupported value {type(value).__name__}")


def _is_exact_binary64_integer(value: int) -> bool:
    """Return whether an integer survives the shared binary64 JSON model exactly."""

    magnitude = abs(value)
    bit_length = magnitude.bit_length()
    if bit_length > _BINARY64_MAX_INTEGER_BITS:
        return False
    if bit_length <= _BINARY64_SIGNIFICAND_BITS:
        return True
    discarded_bits = bit_length - _BINARY64_SIGNIFICAND_BITS
    if magnitude & ((1 << discarded_bits) - 1) != 0:
        return False
    # The bit-level test permits the largest finite binary64 value. A final
    # conversion keeps the boundary explicit and rejects any implementation
    # that would overflow it.
    return math.isfinite(float(value))


def _set_outcome(kind: int) -> SmithySetOutcome:
    try:
        return {
            SMITHY_FFI_RESULT_CREATED: SmithySetOutcome.CREATED,
            SMITHY_FFI_RESULT_REPLACED: SmithySetOutcome.REPLACED,
            SMITHY_FFI_RESULT_NOT_STORED: SmithySetOutcome.NOT_STORED,
        }[kind]
    except KeyError as error:
        raise OpenKacheError(
            f"SET returned unexpected native result {kind}"
        ) from error


def _delete_outcome(kind: int) -> bool:
    if kind == SMITHY_FFI_RESULT_DELETED:
        return True
    if kind == SMITHY_FFI_RESULT_NOT_DELETED:
        return False
    raise OpenKacheError(f"DELETE returned unexpected native result {kind}")


def _map_native_error(error: NativeError) -> OpenKacheError:
    return OpenKacheError(str(error), metadata=error.metadata)


def _positive_or_zero(
    value: int,
    name: str,
    *,
    allow_zero: bool = True,
    maximum: int | None = None,
) -> None:
    if isinstance(value, bool) or not isinstance(value, int):
        raise OpenKacheValueError(f"{name} must be an integer")
    minimum = 0 if allow_zero else SMITHY_CLIENT_MINIMUM_POSITIVE_VALUE
    if value < minimum:
        requirement = "non-negative" if allow_zero else "positive"
        raise OpenKacheValueError(f"{name} must be {requirement}")
    if maximum is not None and value > maximum:
        raise OpenKacheValueError(f"{name} must be at most {maximum}")


__all__ = [
    "Client",
    "ClientIdentity",
    "ClientTimeouts",
    "CompressionOptions",
    "ConnectionState",
    "DataProtectionKeyRing",
    "ErrorMetadata",
    "MetricsSnapshot",
    "OpenKacheClient",
    "OpenKacheCancelledError",
    "OpenKacheError",
    "OpenKacheValueError",
    "RawClient",
    "ServerStats",
    "SetCondition",
    "SetOptions",
    "SetOutcome",
]
