"""Small synchronous facade for the maintained OpenKache v1 client.

Gate 0 intentionally keeps the Python surface narrow.  The five public
operations are ``connect``, ``get``, ``set``, ``delete``, and ``close``.
Mapped keys are encoded by the shared key contract and values always travel
through ``StructuredValue-CBOR-v1``; per-request policy controls are not part
of this facade.
"""

from __future__ import annotations

import socket
from dataclasses import dataclass
from enum import StrEnum
from typing import Final, Generic, NoReturn, TypeVar

from ._generated.smithy_contract import (
    SMITHY_FFI_RESULT_CREATED,
    SMITHY_FFI_RESULT_DELETED,
    SMITHY_FFI_RESULT_NOT_DELETED,
    SMITHY_FFI_RESULT_NOT_FOUND,
    SMITHY_FFI_RESULT_NOT_STORED,
    SMITHY_FFI_RESULT_REPLACED,
    SMITHY_FFI_RESULT_UNKNOWN_MUTATION,
    SMITHY_FFI_RESULT_VALUE,
    SMITHY_OPCODE_DELETE,
)
from ._native import NativeClient as _NativeClient, NativeError
from ._value import StructuredValueError, decode_value, encode_value


_I64_MIN: Final = -(1 << 63)
_I64_MAX: Final = (1 << 63) - 1
_MAX_CANONICAL_KEY_BYTES: Final = 1_048_576
_T = TypeVar("_T")


class OpenKacheError(RuntimeError):
    """Base exception raised by the maintained client."""


class OpenKacheValueError(OpenKacheError, ValueError):
    """Invalid key, value, or local value-codec input."""


class OpenKacheUnknownMutationError(OpenKacheError):
    """A mutation may have reached the server without a definitive outcome."""

    kind = "unknown_mutation"


class OpenKacheIncompatibleServerError(OpenKacheError):
    """The server returned an outcome outside the maintained Gate 0 contract."""

    kind = "incompatible_server_outcome"


# This spelling existed in the preview package. Keep it importable for code that
# only catches the error; Gate 0 never exposes cancellation controls.
UnknownMutationError = OpenKacheUnknownMutationError


@dataclass(frozen=True, slots=True)
class Found(Generic[_T]):
    """A successful ``get`` result containing the decoded value."""

    value: _T

    def __bool__(self) -> bool:
        return True


@dataclass(frozen=True, slots=True)
class Missing:
    """A ``get`` result for a key that is not present."""

    def __bool__(self) -> bool:
        return False

    def __repr__(self) -> str:
        return "Missing"


MISSING = Missing()
GetResult = Found[_T] | Missing


class SetOutcome(StrEnum):
    """The server's definitive SET result."""

    CREATED = "created"
    REPLACED = "replaced"


def _set_outcome(kind: object) -> SetOutcome:
    if isinstance(kind, SetOutcome):
        return kind
    if isinstance(kind, str):
        if kind == "not_stored":
            raise OpenKacheIncompatibleServerError(
                "SET returned the unsupported conditional outcome 'not_stored'"
            )
        try:
            return SetOutcome(kind)
        except ValueError as error:
            raise OpenKacheError(f"SET returned unexpected native result {kind!r}") from error
    try:
        numeric_kind = int(kind)
    except (TypeError, ValueError) as error:
        raise OpenKacheError(f"SET returned unexpected native result {kind!r}") from error
    if numeric_kind == SMITHY_FFI_RESULT_NOT_STORED:
        raise OpenKacheIncompatibleServerError(
            "SET returned the unsupported conditional outcome 'not_stored'"
        )
    try:
        return {
            SMITHY_FFI_RESULT_CREATED: SetOutcome.CREATED,
            SMITHY_FFI_RESULT_REPLACED: SetOutcome.REPLACED,
        }[numeric_kind]
    except KeyError as error:
        raise OpenKacheError(f"SET returned unexpected native result {kind!r}") from error


class DeleteOutcome(StrEnum):
    """The server's definitive DELETE result."""

    DELETED = "deleted"
    NOT_FOUND = "not_found"


def _delete_outcome(kind: object) -> DeleteOutcome:
    if isinstance(kind, DeleteOutcome):
        return kind
    if isinstance(kind, str):
        try:
            return DeleteOutcome(kind)
        except ValueError as error:
            raise OpenKacheError(
                f"DELETE returned unexpected native result {kind!r}"
            ) from error
    try:
        if int(kind) == SMITHY_FFI_RESULT_DELETED:
            return DeleteOutcome.DELETED
        if int(kind) == SMITHY_FFI_RESULT_NOT_DELETED:
            return DeleteOutcome.NOT_FOUND
    except (TypeError, ValueError):
        pass
    raise OpenKacheError(f"DELETE returned unexpected native result {kind!r}")


class OpenKacheClient:
    """Synchronous structured-value client with a deliberately small API."""

    def __init__(self, native: _NativeClient) -> None:
        self._native = native
        self._closed = False

    @classmethod
    def connect(cls, address: str) -> OpenKacheClient:
        """Open one development TLS 1.3 connection.

        Gate 0 intentionally has no certificate, retry, timeout, TTL, or
        transport arguments.  The private native adapter fixes the
        verification-disabled DevelopmentTrust profile, ``openkache/1`` ALPN,
        namespace 1, the development Item-ID root, and the uncompressed,
        unprotected StructuredValue-CBOR-v1 selector.  Production
        authentication configuration is deferred to a later maintained-client
        gate.
        """

        native_address = _resolve_address(address)
        try:
            native = _NativeClient.connect_gate0(address=native_address.encode("ascii"))
        except NativeError as error:
            _raise_native_error(error)
        except OSError as error:
            raise OpenKacheError(str(error)) from error
        return cls(native)

    def get(
        self,
        key: str | int | bytes | bytearray | memoryview,
    ) -> Found[object] | Missing:
        """Read one structured value as lossless model wrappers.

        ``Missing`` is distinct from ``Found(None)``.  The returned model
        retains ``Undefined``, integer/float distinctions, raw float bits,
        byte/text kinds, and scalar-key map identity.
        """

        self._assert_open()
        key_bytes = _key_bytes(key)
        operation = getattr(self._native, "get_structured", None)
        if not callable(operation):
            raise OpenKacheError(
                "native client does not support StructuredValue-CBOR-v1"
            )
        try:
            result = operation(key=key_bytes)
        except NativeError as error:
            _raise_native_error(error)
        kind, payload = _read_result(result, operation_name="GET")
        if kind == SMITHY_FFI_RESULT_NOT_FOUND:
            return MISSING
        if kind != SMITHY_FFI_RESULT_VALUE:
            raise OpenKacheError(f"GET returned unexpected native result {kind!r}")
        try:
            value = decode_value(payload)
        except StructuredValueError as error:
            raise OpenKacheValueError(
                f"StructuredValue-CBOR-v1 decoding failed: {error}"
            ) from error
        return Found(value)

    def set(
        self,
        key: str | int | bytes | bytearray | memoryview,
        value: object,
    ) -> SetOutcome:
        """Encode and store one value through StructuredValue-CBOR-v1."""

        self._assert_open()
        try:
            payload = encode_value(value)
        except StructuredValueError as error:
            raise OpenKacheValueError(
                f"StructuredValue-CBOR-v1 encoding failed: {error}"
            ) from error
        operation = getattr(self._native, "set_structured", None)
        if not callable(operation):
            raise OpenKacheError(
                "native client does not support StructuredValue-CBOR-v1"
            )
        try:
            result = operation(
                key=_key_bytes(key),
                value=payload,
                set_flags=0,
                ttl_ms=0,
            )
        except NativeError as error:
            _raise_native_error(error)
        kind, _ = _read_result(result, operation_name="SET")
        return _set_outcome(kind)

    def delete(self, key: str | int | bytes | bytearray | memoryview) -> DeleteOutcome:
        """Delete one mapped key and return its tagged server outcome."""

        self._assert_open()
        try:
            result = self._native.execute_with_options(
                SMITHY_OPCODE_DELETE,
                key=_key_bytes(key),
                value=b"",
                set_flags=0,
                ttl_ms=0,
            )
        except NativeError as error:
            _raise_native_error(error)
        kind, _ = _read_result(result, operation_name="DELETE")
        return _delete_outcome(kind)

    def close(self) -> None:
        """Release the native connection; repeated calls are harmless."""

        if self._closed:
            return
        self._closed = True
        self._native.close()

    def __enter__(self) -> OpenKacheClient:
        self._assert_open()
        return self

    def __exit__(self, *_: object) -> None:
        self.close()

    def _assert_open(self) -> None:
        if self._closed:
            raise OpenKacheError("client is closed")

Client = OpenKacheClient


def _read_result(result: object, *, operation_name: str) -> tuple[object, bytes]:
    """Normalize native tuple results and simple fake-adapter results."""

    if isinstance(result, tuple):
        if len(result) != 2:
            raise OpenKacheError(
                f"{operation_name} returned a malformed native result"
            )
        kind, payload = result
    elif result is None:
        return SMITHY_FFI_RESULT_NOT_FOUND, b""
    else:
        kind, payload = SMITHY_FFI_RESULT_VALUE, result
    if payload is None:
        return kind, b""
    if isinstance(payload, bytes):
        return kind, payload
    if isinstance(payload, (bytearray, memoryview)):
        return kind, bytes(payload)
    raise OpenKacheError(f"{operation_name} returned a non-byte payload")


def _raise_native_error(error: NativeError) -> NoReturn:
    if error.result_kind == SMITHY_FFI_RESULT_UNKNOWN_MUTATION:
        raise OpenKacheUnknownMutationError(str(error)) from error
    raise OpenKacheError(str(error)) from error


def _resolve_address(address: str) -> str:
    """Validate and normalize one host:port endpoint for the native ABI."""

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
            raise OpenKacheValueError("address must include a transport port") from error
    if not host or not port_text.isdecimal():
        raise OpenKacheValueError("address must contain a host and numeric transport port")
    port = int(port_text)
    if not 1 <= port <= 65_535:
        raise OpenKacheValueError("address port must be between 1 and 65535")
    try:
        infos = socket.getaddrinfo(host, port, type=0)
    except OSError as error:
        raise OpenKacheError(f"address DNS resolution failed: {error}") from error
    for family, _, _, _, sockaddr in infos:
        if family == socket.AF_INET:
            return f"{sockaddr[0]}:{sockaddr[1]}"
        if family == socket.AF_INET6:
            return f"[{sockaddr[0]}]:{sockaddr[1]}"
    raise OpenKacheError(f"address did not resolve to a transport endpoint: {address}")


def _key_bytes(value: str | int | bytes | bytearray | memoryview) -> bytes:
    """Encode one mapped key using canonical TypedKey CBOR."""

    if isinstance(value, str):
        try:
            payload = value.encode("utf-8")
        except UnicodeEncodeError as error:
            raise OpenKacheValueError("key must contain valid UTF-8 text") from error
        return _canonical_cbor_string(3, payload)
    if isinstance(value, (bytes, bytearray, memoryview)):
        return _canonical_cbor_string(2, bytes(value))
    if isinstance(value, bool):
        raise OpenKacheValueError("boolean keys are not supported")
    if isinstance(value, int):
        return _canonical_cbor_integer(value)
    raise OpenKacheValueError(
        "key must be a signed-i64 integer, UTF-8 string, or bytes-like value"
    )


def _canonical_cbor_string(major: int, payload: bytes) -> bytes:
    header = _canonical_cbor_argument(major, len(payload))
    total = len(header) + len(payload)
    if total > _MAX_CANONICAL_KEY_BYTES:
        raise OpenKacheValueError(
            f"canonical key exceeds {_MAX_CANONICAL_KEY_BYTES} bytes"
        )
    return header + payload


def _canonical_cbor_argument(major: int, value: int) -> bytes:
    if major not in (0, 1, 2, 3) or value < 0:
        raise OpenKacheValueError("invalid canonical CBOR key argument")
    prefix = major << 5
    if value <= 23:
        return bytes((prefix | value,))
    if value <= 0xFF:
        return bytes((prefix | 24, value))
    if value <= 0xFFFF:
        return bytes((prefix | 25,)) + value.to_bytes(2, "big")
    if value <= 0xFFFF_FFFF:
        return bytes((prefix | 26,)) + value.to_bytes(4, "big")
    if value <= 0xFFFF_FFFF_FFFF_FFFF:
        return bytes((prefix | 27,)) + value.to_bytes(8, "big")
    raise OpenKacheValueError("canonical key length exceeds CBOR uint64")


def _canonical_cbor_integer(value: int) -> bytes:
    if isinstance(value, bool) or not isinstance(value, int):
        raise OpenKacheValueError("key must be an integer")
    if value < _I64_MIN or value > _I64_MAX:
        raise OpenKacheValueError("integer keys must fit signed i64")
    negative = value < 0
    transformed = -value - 1 if negative else value
    return _canonical_cbor_argument(1 if negative else 0, transformed)


__all__ = [
    "Client",
    "Found",
    "GetResult",
    "DeleteOutcome",
    "Missing",
    "MISSING",
    "OpenKacheClient",
    "OpenKacheError",
    "OpenKacheIncompatibleServerError",
    "OpenKacheUnknownMutationError",
    "OpenKacheValueError",
    "SetOutcome",
]
