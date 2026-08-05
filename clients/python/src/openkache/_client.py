"""Async Python API layered over the shared OpenKache client core."""

from __future__ import annotations

import asyncio
import base64
import json
import math
import socket
import sys
from dataclasses import dataclass, field
from enum import IntEnum, StrEnum
from os import PathLike
from pathlib import Path
from typing import Any, Final, Iterable, Sequence

from ._generated import (
    SmithyEvictionDefault,
    SmithyEvictionMode,
    SmithyExpirationDefault,
    SmithyExpirationMode,
    SmithyNamespaceDeleteInput,
    SmithyNamespaceDeleteOutput,
    SmithyNamespaceDescriptor,
    SmithyNamespaceOpenInput,
    SmithyNamespaceOpenOutput,
    SmithyNamespacePolicy,
    SmithyNamespaceUpdatePolicyInput,
    SmithyNamespaceUpdatePolicyOutput,
    SmithyOverridePolicy,
    SmithySetCondition,
    SmithySetOutcome,
)
from ._generated.smithy_operations import (
    SmithyGeneratedOperations,
    SmithyOperationTransport,
)
from ._generated.smithy_contract import (
    SMITHY_FFI_CONNECTION_STATE_CLOSED,
    SMITHY_FFI_CONNECTION_STATE_CONNECTED,
    SMITHY_FFI_CONNECTION_STATE_DISCONNECTED,
    SMITHY_FFI_CONNECTION_STATE_RECONNECTING,
    SMITHY_FFI_CONNECTION_STATE_UNKNOWN,
    SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED,
    SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL,
    SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED,
    SMITHY_FFI_OPERATION_GET_JSON,
    SMITHY_FFI_OPERATION_RECONNECT,
    SMITHY_FFI_OPERATION_SET_JSON,
    SMITHY_FFI_RESULT_CREATED,
    SMITHY_FFI_RESULT_DELETED,
    SMITHY_FFI_RESULT_NOT_DELETED,
    SMITHY_FFI_RESULT_NOT_FOUND,
    SMITHY_FFI_RESULT_NOT_STORED,
    SMITHY_FFI_RESULT_REPLACED,
    SMITHY_FFI_RESULT_OK,
    SMITHY_FFI_RESULT_VALUE,
    SMITHY_FFI_SET_CONDITION_IF_ABSENT,
    SMITHY_FFI_SET_CONDITION_IF_PRESENT,
    SMITHY_FFI_SET_CONDITION_ANY,
    SMITHY_DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
    SMITHY_DEFAULT_MAX_IN_FLIGHT,
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
    SMITHY_MAX_VALUE_BYTES,
    SMITHY_OPCODE_DELETE,
    SMITHY_OPCODE_ECHO,
    SMITHY_OPCODE_GET,
    SMITHY_OPCODE_NAMESPACE_DELETE,
    SMITHY_OPCODE_NAMESPACE_OPEN,
    SMITHY_OPCODE_NAMESPACE_UPDATE_POLICY,
    SMITHY_OPCODE_PING,
    SMITHY_OPCODE_SET,
    SMITHY_OPCODE_STATS,
    SMITHY_OPCODE_SYNC,
    SMITHY_POLICY_DEFAULT_EXPIRATION_MASK,
    SMITHY_POLICY_EVICTION_OVERRIDE,
    SMITHY_POLICY_EVICTION_PROTECTED,
    SMITHY_POLICY_EXPIRATION_OVERRIDE,
    SMITHY_POLICY_FIXED_TTL,
    SMITHY_POLICY_NO_EXPIRY,
    SMITHY_SET_CONDITION_ANY_BITS,
    SMITHY_SET_EVICTABLE_BITS,
    SMITHY_SET_EVICTION_PROTECTED_BITS,
    SMITHY_SET_EXPLICIT_TTL_BITS,
    SMITHY_SET_INHERIT_EVICTION_BITS,
    SMITHY_SET_INHERIT_EXPIRATION_BITS,
    SMITHY_SET_NO_EXPIRY_BITS,
    SMITHY_SET_CONDITION_MASK,
    SMITHY_SET_EXPIRATION_MASK,
    SMITHY_SET_EVICTION_MASK,
    SMITHY_SET_RESERVED_MASK,
    SMITHY_SET_IF_ABSENT_BITS,
    SMITHY_SET_IF_PRESENT_BITS,
    SMITHY_MAX_VARUINT_BYTES,
    SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES,
    SMITHY_VALUE_ENCRYPTION_COMPACT,
    SMITHY_VALUE_ENCRYPTION_ROBUST,
)
from ._native import NativeClient as _NativeClient, NativeError


_UINT64_MAX: Final = (1 << 64) - 1
_SIZE_T_MAX: Final = (sys.maxsize << 1) | 1
_BINARY64_SIGNIFICAND_BITS: Final = 53
_BINARY64_MAX_INTEGER_BITS: Final = 1024


class OpenKacheError(RuntimeError):
    """Base error raised by the Python client."""


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
    """Atomic existence and item-policy selections for one SET."""

    condition: SmithySetCondition | str | None = None
    expiration_mode: SmithyExpirationMode | str | None = None
    eviction_mode: SmithyEvictionMode | str | None = None
    ttl_ms: int | None = None

    def __post_init__(self) -> None:
        condition = self.condition
        if isinstance(condition, str):
            try:
                condition = SmithySetCondition(condition)
            except ValueError as error:
                raise OpenKacheValueError(
                    "condition must be "
                    + ", ".join(
                        f"'{member.value}'" for member in SmithySetCondition
                    )
                ) from error
            object.__setattr__(self, "condition", condition)
        elif condition is not None and not isinstance(condition, SmithySetCondition):
            raise OpenKacheValueError(
                "condition must be "
                + ", ".join(f"'{member.value}'" for member in SmithySetCondition)
                + ", or None"
            )
        expiration_mode = self.expiration_mode
        if isinstance(expiration_mode, str):
            try:
                expiration_mode = SmithyExpirationMode(expiration_mode)
            except ValueError as error:
                raise OpenKacheValueError(
                    "expiration_mode must be "
                    + ", ".join(
                        f"'{member.value}'" for member in SmithyExpirationMode
                    )
                ) from error
            object.__setattr__(self, "expiration_mode", expiration_mode)
        elif expiration_mode is not None and not isinstance(
            expiration_mode, SmithyExpirationMode
        ):
            raise OpenKacheValueError(
                "expiration_mode must be a SmithyExpirationMode value or None"
            )
        eviction_mode = self.eviction_mode
        if isinstance(eviction_mode, str):
            try:
                eviction_mode = SmithyEvictionMode(eviction_mode)
            except ValueError as error:
                raise OpenKacheValueError(
                    "eviction_mode must be "
                    + ", ".join(
                        f"'{member.value}'" for member in SmithyEvictionMode
                    )
                ) from error
            object.__setattr__(self, "eviction_mode", eviction_mode)
        elif eviction_mode is not None and not isinstance(eviction_mode, SmithyEvictionMode):
            raise OpenKacheValueError(
                "eviction_mode must be a SmithyEvictionMode value or None"
            )
        if self.ttl_ms is not None:
            _positive_or_zero(
                self.ttl_ms,
                "ttl_ms",
                allow_zero=False,
                maximum=_UINT64_MAX,
            )
        selected_expiration = expiration_mode or (
            SmithyExpirationMode.EXPLICIT_TTL
            if self.ttl_ms is not None
            else SmithyExpirationMode.INHERIT
        )
        if selected_expiration is SmithyExpirationMode.EXPLICIT_TTL:
            if self.ttl_ms is None:
                raise OpenKacheValueError(
                    "ttl_ms is required with "
                    f"{SmithyExpirationMode.EXPLICIT_TTL.value} expiration_mode"
                )
        elif self.ttl_ms is not None:
            raise OpenKacheValueError(
                "ttl_ms is only valid with "
                f"{SmithyExpirationMode.EXPLICIT_TTL.value} expiration_mode"
            )
        object.__setattr__(self, "expiration_mode", selected_expiration)
        object.__setattr__(
            self,
            "eviction_mode",
            eviction_mode or SmithyEvictionMode.INHERIT,
        )

    @property
    def _condition_code(self) -> int:
        if self.condition is None:
            return SMITHY_FFI_SET_CONDITION_ANY
        if self.condition is SmithySetCondition.IF_ABSENT:
            return SMITHY_FFI_SET_CONDITION_IF_ABSENT
        return SMITHY_FFI_SET_CONDITION_IF_PRESENT

    @property
    def _wire_flags(self) -> int:
        condition = {
            None: SMITHY_SET_CONDITION_ANY_BITS,
            SmithySetCondition.ANY: SMITHY_SET_CONDITION_ANY_BITS,
            SmithySetCondition.IF_ABSENT: SMITHY_SET_IF_ABSENT_BITS,
            SmithySetCondition.IF_PRESENT: SMITHY_SET_IF_PRESENT_BITS,
        }[self.condition]
        expiration = {
            SmithyExpirationMode.INHERIT: SMITHY_SET_INHERIT_EXPIRATION_BITS,
            SmithyExpirationMode.NO_EXPIRY: SMITHY_SET_NO_EXPIRY_BITS,
            SmithyExpirationMode.EXPLICIT_TTL: SMITHY_SET_EXPLICIT_TTL_BITS,
        }[self.expiration_mode]
        eviction = {
            SmithyEvictionMode.INHERIT: SMITHY_SET_INHERIT_EVICTION_BITS,
            SmithyEvictionMode.EVICTABLE: SMITHY_SET_EVICTABLE_BITS,
            SmithyEvictionMode.EVICTION_PROTECTED: SMITHY_SET_EVICTION_PROTECTED_BITS,
        }[self.eviction_mode]
        return condition | expiration | eviction


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
        data_protection_key: bytes | bytearray | memoryview,
        server_name: str | None = None,
        identity: ClientIdentity | None = None,
        compression: CompressionOptions | None = None,
        encryption: Encryption = Encryption.ROBUST,
        timeouts: ClientTimeouts | None = None,
        max_in_flight: int = SMITHY_DEFAULT_MAX_IN_FLIGHT,
        retry_max_attempts: int = SMITHY_DEFAULT_RETRY_MAX_ATTEMPTS,
        native_path: str | PathLike[str] | None = None,
    ) -> OpenKacheClient:
        try:
            settings = await asyncio.to_thread(
                _connection_settings,
                address,
                certificate=certificate,
                data_protection_key=data_protection_key,
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
        except (NativeError, OSError) as error:
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

    async def echo(self, message: str) -> str:
        """Sends an experimental UTF-8 message and returns the echoed message."""

        self._assert_open()
        if not isinstance(message, str):
            raise OpenKacheValueError("message must be a string")
        kind, payload = await self._execute(
            SMITHY_OPCODE_ECHO,
            value=message.encode("utf-8"),
        )
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(f"ECHO returned unexpected native result {kind}")
        try:
            return payload.decode("utf-8")
        except UnicodeDecodeError as error:
            raise OpenKacheError(f"ECHO response is not UTF-8: {error}") from error

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
        self, key: str | bytes | bytearray | memoryview
    ) -> bool:
        self._assert_open()
        kind, _ = await self._execute(SMITHY_OPCODE_DELETE, key=_key_bytes(key))
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

    async def _execute(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        options: SetOptions | None = None,
    ) -> tuple[int, bytes]:
        self._assert_open()
        selected = options or SetOptions()
        try:
            return await asyncio.to_thread(
                self._native.execute_with_options,
                operation,
                key=key,
                value=value,
                set_flags=selected._wire_flags,
                ttl_ms=selected.ttl_ms or 0,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error

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
    ) -> tuple[int, bytes]:
        self._assert_open()
        selected = options or SetOptions()
        try:
            return await asyncio.to_thread(
                self._native.execute_raw_with_options,
                operation,
                item_id=item_id,
                value=value,
                set_flags=selected._wire_flags,
                ttl_ms=selected.ttl_ms or 0,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error

    async def _execute_scoped(
        self,
        operation: int,
        *,
        namespace_id: int,
        item_id: bytes = b"",
        value: bytes = b"",
        options: SetOptions | None = None,
    ) -> tuple[int, bytes]:
        self._assert_open()
        if not isinstance(namespace_id, int) or not 1 <= namespace_id <= _UINT64_MAX:
            raise OpenKacheValueError(
                "namespace_id must be a positive unsigned 64-bit integer"
            )
        selected = options or SetOptions()
        try:
            return await asyncio.to_thread(
                self._native.execute_scoped,
                operation,
                namespace_id=namespace_id,
                item_id=item_id,
                value=value,
                set_flags=selected._wire_flags,
                ttl_ms=selected.ttl_ms or 0,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error

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


class _RawOperationTransport(SmithyOperationTransport):
    """Native hooks consumed by generated Smithy operations."""

    def __init__(self, owner: OpenKacheClient) -> None:
        self._owner = owner

    def assert_open(self) -> None:
        self._owner._assert_open()

    async def invoke(
        self,
        operation: int,
        *,
        key: bytes = b"",
        value: bytes = b"",
        condition: SmithySetCondition | None = None,
        expiration_mode: SmithyExpirationMode | None = None,
        eviction_mode: SmithyEvictionMode | None = None,
        ttl_milliseconds: int | None = None,
        expected_kinds: tuple[int, ...],
    ) -> tuple[int, bytes]:
        options = SetOptions(
            condition=condition,
            expiration_mode=expiration_mode,
            eviction_mode=eviction_mode,
            ttl_ms=ttl_milliseconds,
        )
        kind, payload = await self._owner._execute(
            operation,
            key=key,
            value=value,
            options=options,
        )
        self._require_result(operation, kind, expected_kinds)
        return kind, payload

    async def invoke_scoped(
        self,
        operation: int,
        *,
        namespace_id: int,
        item_id: bytes = b"",
        value: bytes = b"",
        condition: SmithySetCondition | None = None,
        expiration_mode: SmithyExpirationMode | None = None,
        eviction_mode: SmithyEvictionMode | None = None,
        ttl_milliseconds: int | None = None,
        expected_kinds: tuple[int, ...],
    ) -> tuple[int, bytes]:
        options = SetOptions(
            condition=condition,
            expiration_mode=expiration_mode,
            eviction_mode=eviction_mode,
            ttl_ms=ttl_milliseconds,
        )
        kind, payload = await self._owner._execute_scoped(
            operation,
            namespace_id=namespace_id,
            item_id=_item_id(item_id) if item_id else b"",
            value=value,
            options=options,
        )
        self._require_result(operation, kind, expected_kinds)
        return kind, payload

    def decode_utf8(self, payload: bytes, operation: int) -> str:
        try:
            return payload.decode("utf-8")
        except UnicodeDecodeError as error:
            raise OpenKacheError(
                f"operation {operation} response is not UTF-8: {error}"
            ) from error

    @staticmethod
    def _require_result(operation: int, kind: int, expected_kinds: tuple[int, ...]) -> None:
        if kind not in expected_kinds:
            expected = ", ".join(str(value) for value in expected_kinds)
            raise OpenKacheError(
                f"operation {operation} returned unexpected native result {kind}; "
                f"expected one of {expected}"
            )

    async def namespace_open(
        self, input: SmithyNamespaceOpenInput
    ) -> SmithyNamespaceOpenOutput:
        if input.create_if_missing and input.policy is None:
            raise OpenKacheValueError(
                "namespace policy is required when create_if_missing is true"
            )
        if not input.create_if_missing and input.policy is not None:
            raise OpenKacheValueError(
                "namespace policy is only valid when create_if_missing is true"
            )
        policy_flags, ttl_ms = _namespace_policy_wire(input.policy)
        try:
            kind, payload = await asyncio.to_thread(
                self._owner._native.namespace_open,
                name=input.name.encode("utf-8"),
                create_if_missing=input.create_if_missing,
                policy_flags=policy_flags,
                ttl_ms=ttl_ms,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error
        if kind not in (SMITHY_FFI_RESULT_OK, SMITHY_FFI_RESULT_CREATED):
            raise OpenKacheError(f"NAMESPACE_OPEN returned unexpected native result {kind}")
        try:
            decoded = await asyncio.to_thread(
                self._owner._native.decode_namespace_descriptor,
                payload,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error
        return SmithyNamespaceOpenOutput(
            descriptor=_namespace_descriptor(decoded),
            created=kind == SMITHY_FFI_RESULT_CREATED,
        )

    async def namespace_update_policy(
        self, input: SmithyNamespaceUpdatePolicyInput
    ) -> SmithyNamespaceDescriptor:
        policy_flags, ttl_ms = _namespace_policy_wire(input.policy)
        try:
            kind, payload = await asyncio.to_thread(
                self._owner._native.namespace_update_policy,
                namespace_id=input.namespace_id,
                expected_revision=input.expected_revision,
                policy_flags=policy_flags,
                ttl_ms=ttl_ms,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(
                f"NAMESPACE_UPDATE_POLICY returned unexpected native result {kind}"
            )
        try:
            decoded = await asyncio.to_thread(
                self._owner._native.decode_namespace_descriptor,
                payload,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error
        return _namespace_descriptor(decoded)

    async def namespace_delete(self, input: SmithyNamespaceDeleteInput) -> None:
        try:
            await asyncio.to_thread(
                self._owner._native.namespace_delete,
                namespace_id=input.namespace_id,
                expected_revision=input.expected_revision,
            )
        except NativeError as error:
            raise OpenKacheError(str(error)) from error


class RawClient(SmithyGeneratedOperations):
    """Smithy-generated exact item-ID operations over a shared connection."""

    def __init__(self, owner: OpenKacheClient) -> None:
        super().__init__(_RawOperationTransport(owner))
        self._owner = owner

    async def close(self) -> None:
        await self._owner.close()


Client = OpenKacheClient


def _connection_settings(
    address: str,
    *,
    certificate: bytes | bytearray | memoryview | str | PathLike[str],
    data_protection_key: bytes | bytearray | memoryview,
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
    protection_key = _as_bytes(data_protection_key, "data_protection_key")
    if len(protection_key) != SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES:
        raise OpenKacheValueError(
            "data_protection_key must contain exactly "
            f"{SMITHY_VALUE_DATA_PROTECTION_KEY_BYTES} bytes"
        )
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
    try:
        infos = socket.getaddrinfo(host, port, type=socket.SOCK_DGRAM)
    except OSError as error:
        raise OpenKacheError(f"address DNS resolution failed: {error}") from error
    for family, _, _, _, sockaddr in infos:
        if family == socket.AF_INET:
            return f"{sockaddr[0]}:{sockaddr[1]}", host
        if family == socket.AF_INET6:
            return f"[{sockaddr[0]}]:{sockaddr[1]}", host
    raise OpenKacheError(f"address did not resolve to a UDP endpoint: {address}")


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


def _namespace_policy_wire(
    policy: SmithyNamespacePolicy | None,
) -> tuple[int, int]:
    if policy is None:
        return 0, 0
    try:
        default_expiration = SmithyExpirationDefault(policy.default_expiration)
        expiration_override = SmithyOverridePolicy(policy.expiration_override)
        default_eviction = SmithyEvictionDefault(policy.default_eviction)
        eviction_override = SmithyOverridePolicy(policy.eviction_override)
    except ValueError as error:
        raise OpenKacheValueError(f"invalid namespace policy enum: {error}") from error
    flags = SMITHY_POLICY_NO_EXPIRY
    ttl_ms = policy.default_ttl_milliseconds
    if default_expiration is SmithyExpirationDefault.NO_EXPIRY:
        if ttl_ms is not None:
            raise OpenKacheValueError(
                "default_ttl_milliseconds requires "
                f"{SmithyExpirationDefault.FIXED_TTL.value} default_expiration"
            )
    else:
        if ttl_ms is None:
            raise OpenKacheValueError(
                "default_ttl_milliseconds is required with "
                f"{SmithyExpirationDefault.FIXED_TTL.value} default_expiration"
            )
        _positive_or_zero(
            ttl_ms,
            "default_ttl_milliseconds",
            allow_zero=False,
            maximum=_UINT64_MAX,
        )
        flags |= SMITHY_POLICY_FIXED_TTL
    if expiration_override is SmithyOverridePolicy.ALLOWED:
        flags |= SMITHY_POLICY_EXPIRATION_OVERRIDE
    if default_eviction is SmithyEvictionDefault.EVICTION_PROTECTED:
        flags |= SMITHY_POLICY_EVICTION_PROTECTED
    if eviction_override is SmithyOverridePolicy.ALLOWED:
        flags |= SMITHY_POLICY_EVICTION_OVERRIDE
    return flags, ttl_ms or 0


def _namespace_descriptor(
    decoded: tuple[int, int, int, int, int, int, int],
) -> SmithyNamespaceDescriptor:
    (
        namespace_id,
        revision,
        default_ttl,
        default_expiration,
        expiration_override,
        default_eviction,
        eviction_override,
    ) = decoded
    policy = SmithyNamespacePolicy(
        default_expiration=(
            SmithyExpirationDefault.FIXED_TTL
            if default_expiration == SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL
            else SmithyExpirationDefault.NO_EXPIRY
        ),
        default_ttl_milliseconds=(
            default_ttl
            if default_expiration == SMITHY_FFI_NAMESPACE_DEFAULT_EXPIRATION_FIXED_TTL
            else None
        ),
        expiration_override=(
            SmithyOverridePolicy.ALLOWED
            if expiration_override == SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED
            else SmithyOverridePolicy.DISALLOWED
        ),
        default_eviction=(
            SmithyEvictionDefault.EVICTION_PROTECTED
            if default_eviction == SMITHY_FFI_NAMESPACE_DEFAULT_EVICTION_PROTECTED
            else SmithyEvictionDefault.EVICTABLE
        ),
        eviction_override=(
            SmithyOverridePolicy.ALLOWED
            if eviction_override == SMITHY_FFI_NAMESPACE_OVERRIDE_ALLOWED
            else SmithyOverridePolicy.DISALLOWED
        ),
    )
    return SmithyNamespaceDescriptor(
        namespace_id=namespace_id,
        revision=revision,
        policy=policy,
    )


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
    "OpenKacheClient",
    "OpenKacheError",
    "OpenKacheValueError",
    "RawClient",
    "ServerStats",
    "SetCondition",
    "SetOptions",
    "SetOutcome",
]
