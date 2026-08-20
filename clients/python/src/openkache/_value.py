"""Lossless Python bindings for ``StructuredValue-CBOR-v1``.

The Rust value crate owns the wire profile.  This module only maps Python's
native values to the language-independent model and provides the small
lossless escape hatch needed by dynamic callers.  It deliberately does not
inspect JSON, infer integers from floats, or stringify unsupported values.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass
from enum import StrEnum
from typing import Any, Iterable, Iterator, Mapping, Sequence


class ValueErrorKind(StrEnum):
    """Stable local categories for value conversion and codec failures."""

    CONVERSION = "conversion"
    RESOURCE_LIMIT = "resource_limit"
    TRUNCATED = "truncated"
    TRAILING_BYTES = "trailing_bytes"
    INVALID_ENCODING = "invalid_encoding"
    UNSUPPORTED_TYPE = "unsupported_type"
    INVALID_UTF8 = "invalid_utf8"
    INVALID_INTEGER = "invalid_integer"
    NON_SCALAR_KEY = "non_scalar_key"
    DUPLICATE_KEY = "duplicate_key"


class StructuredValueError(ValueError):
    """A value conversion or StructuredValue-CBOR-v1 error."""

    def __init__(self, message: str, kind: ValueErrorKind = ValueErrorKind.CONVERSION):
        super().__init__(message)
        self.kind = kind


@dataclass(frozen=True, slots=True)
class UndefinedValue:
    """The model's value distinct from ``None``."""

    def __repr__(self) -> str:
        return "Undefined"


UNDEFINED = UndefinedValue()


@dataclass(frozen=True, slots=True)
class IntegerValue:
    """An exact arbitrary-precision model integer."""

    value: int

    def __post_init__(self) -> None:
        if isinstance(self.value, bool) or not isinstance(self.value, int):
            raise StructuredValueError("IntegerValue.value must be an int")

    def __int__(self) -> int:
        return self.value


@dataclass(frozen=True, slots=True)
class FloatValue:
    """A model float retaining IEEE width and raw bits."""

    width: int
    raw_bits: int

    def __post_init__(self) -> None:
        limits = {16: (1 << 16) - 1, 32: (1 << 32) - 1, 64: (1 << 64) - 1}
        maximum = limits.get(self.width)
        if maximum is None or isinstance(self.raw_bits, bool) or not isinstance(self.raw_bits, int):
            raise StructuredValueError("FloatValue requires width 16, 32, or 64")
        if not 0 <= self.raw_bits <= maximum:
            raise StructuredValueError("FloatValue.raw_bits is outside its width")


@dataclass(frozen=True, slots=True)
class ByteStringValue:
    """An uninterpreted model byte string."""

    value: bytes

    def __post_init__(self) -> None:
        if not isinstance(self.value, bytes):
            raise StructuredValueError("ByteStringValue.value must be bytes")

    def __bytes__(self) -> bytes:
        return self.value


@dataclass(frozen=True, slots=True)
class TextStringValue:
    """A well-formed UTF-8 model text string."""

    value: str

    def __post_init__(self) -> None:
        if not isinstance(self.value, str):
            raise StructuredValueError("TextStringValue.value must be str")
        try:
            self.value.encode("utf-8")
        except UnicodeEncodeError as error:
            raise StructuredValueError("text contains unpaired surrogates") from error

    def __str__(self) -> str:
        return self.value


@dataclass(frozen=True, slots=True)
class ArrayValue:
    """An ordered model array."""

    values: tuple[Value, ...]

    def __init__(self, values: Iterable[object]):
        object.__setattr__(self, "values", _convert_sequence(values))

    @classmethod
    def _from_values(cls, values: Iterable[Value]) -> ArrayValue:
        result = object.__new__(cls)
        object.__setattr__(result, "values", tuple(values))
        return result

    def __len__(self) -> int:
        return len(self.values)

    def __iter__(self) -> Iterator[Value]:
        return iter(self.values)

    def __getitem__(self, index: int) -> Value:
        return self.values[index]


@dataclass(frozen=True, slots=True, eq=False)
class MapValue:
    """An ordered model map with scalar, structurally unique keys."""

    entries: tuple[tuple[Value, Value], ...]

    def __init__(self, entries: Iterable[tuple[object, object]]):
        converted_pairs = _convert_entries(entries)
        object.__setattr__(self, "entries", converted_pairs)

    @classmethod
    def _from_entries(
        cls, entries: Iterable[tuple[Value, Value]]
    ) -> MapValue:
        result = object.__new__(cls)
        object.__setattr__(result, "entries", tuple(entries))
        return result

    def __len__(self) -> int:
        return len(self.entries)

    def __iter__(self) -> Iterator[tuple[Value, Value]]:
        return iter(self.entries)

    def __getitem__(self, key: object) -> Value:
        sought = to_value(key)
        matches = [value for candidate, value in self.entries if model_equal(candidate, sought)]
        if len(matches) > 1:
            raise StructuredValueError("map lookup is ambiguous", ValueErrorKind.DUPLICATE_KEY)
        if not matches:
            raise KeyError(key)
        return matches[0]

    def __contains__(self, key: object) -> bool:
        sought = to_value(key)
        return any(model_equal(candidate, sought) for candidate, _ in self.entries)

    def keys(self) -> Iterator[Value]:
        return (key for key, _ in self.entries)

    def values(self) -> Iterator[Value]:
        return (value for _, value in self.entries)

    def items(self) -> Iterator[tuple[Value, Value]]:
        return iter(self.entries)

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, MapValue) or len(self.entries) != len(other.entries):
            return False
        unmatched = list(other.entries)
        for key, value in self.entries:
            for index, (other_key, other_value) in enumerate(unmatched):
                if model_equal(key, other_key):
                    if not model_equal(value, other_value):
                        return False
                    unmatched.pop(index)
                    break
            else:
                return False
        return True


Value = (
    UndefinedValue
    | IntegerValue
    | FloatValue
    | ByteStringValue
    | TextStringValue
    | ArrayValue
    | MapValue
    | None
    | bool
)

# Short names are useful when constructing a lossless value explicitly.
Undefined = UndefinedValue
Integer = IntegerValue
Float = FloatValue
ByteString = ByteStringValue
TextString = TextStringValue
Array = ArrayValue
Map = MapValue


@dataclass(frozen=True, slots=True)
class ValueLimits:
    """One bounded budget shared by encode and decode operations."""

    max_bytes: int = 67_108_864
    max_depth: int = 128
    max_items: int = 1_000_000
    max_integer_bytes: int = 1 << 20

    def __post_init__(self) -> None:
        if any(
            isinstance(value, bool) or not isinstance(value, int) or value <= 0
            for value in (
                self.max_bytes,
                self.max_depth,
                self.max_items,
                self.max_integer_bytes,
            )
        ):
            raise StructuredValueError("value limits must be positive integers")


def to_value(value: object) -> Value:
    """Converts one Python value to the generic model.

    ``bool`` is deliberately checked before ``int``.  Python ``int`` values
    remain arbitrary precision and Python tuples intentionally map to arrays.
    """

    return _convert(value, set())


def _convert(value: object, ancestors: set[int]) -> Value:
    if isinstance(
        value,
        (
            UndefinedValue,
            IntegerValue,
            FloatValue,
            ByteStringValue,
            TextStringValue,
            ArrayValue,
            MapValue,
        ),
    ):
        return value
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, int):
        return IntegerValue(value)
    if isinstance(value, float):
        return FloatValue(64, struct.unpack(">Q", struct.pack(">d", value))[0])
    if isinstance(value, (bytes, bytearray, memoryview)):
        return ByteStringValue(bytes(value))
    if isinstance(value, str):
        return TextStringValue(value)
    if isinstance(value, (list, tuple)):
        identity = id(value)
        if identity in ancestors:
            raise StructuredValueError(
                "value contains a cyclic reference",
                ValueErrorKind.CONVERSION,
            )
        ancestors.add(identity)
        try:
            return ArrayValue._from_values(_convert(child, ancestors) for child in value)
        finally:
            ancestors.remove(identity)
    if isinstance(value, Mapping):
        identity = id(value)
        if identity in ancestors:
            raise StructuredValueError(
                "value contains a cyclic reference",
                ValueErrorKind.CONVERSION,
            )
        ancestors.add(identity)
        try:
            entries: list[tuple[Value, Value]] = []
            for key, child in value.items():
                model_key, model_child = _convert(key, ancestors), _convert(child, ancestors)
                _validate_map_key(model_key, len(entries), entries)
                entries.append((model_key, model_child))
            return MapValue._from_entries(entries)
        finally:
            ancestors.remove(identity)
    raise StructuredValueError(
        f"unsupported Python value {type(value).__name__!s}",
        ValueErrorKind.CONVERSION,
    )


def _convert_sequence(values: Iterable[object]) -> tuple[Value, ...]:
    ancestors: set[int] = set()
    converted: list[Value] = []
    for value in values:
        converted.append(_convert(value, ancestors))
    return tuple(converted)


def _convert_entries(
    entries: Iterable[tuple[object, object]],
) -> tuple[tuple[Value, Value], ...]:
    ancestors: set[int] = set()
    converted: list[tuple[Value, Value]] = []
    for index, pair in enumerate(entries):
        if not isinstance(pair, (tuple, list)) or len(pair) != 2:
            raise StructuredValueError(
                f"map entry {index} must be a two-item pair",
                ValueErrorKind.CONVERSION,
            )
        key, value = _convert(pair[0], ancestors), _convert(pair[1], ancestors)
        _validate_map_key(key, index, converted)
        converted.append((key, value))
    return tuple(converted)


def model_equal(left: Value, right: Value) -> bool:
    """Compares two model values without Python's bool/int equality collapse."""

    if type(left) is not type(right):
        return False
    if isinstance(left, (UndefinedValue,)) and isinstance(right, UndefinedValue):
        return True
    if isinstance(left, (bool,)) and isinstance(right, bool):
        return left == right
    if isinstance(left, IntegerValue) and isinstance(right, IntegerValue):
        return left.value == right.value
    if isinstance(left, FloatValue) and isinstance(right, FloatValue):
        return left.width == right.width and left.raw_bits == right.raw_bits
    if isinstance(left, ByteStringValue) and isinstance(right, ByteStringValue):
        return left.value == right.value
    if isinstance(left, TextStringValue) and isinstance(right, TextStringValue):
        return left.value == right.value
    if isinstance(left, ArrayValue) and isinstance(right, ArrayValue):
        return len(left.values) == len(right.values) and all(
            model_equal(a, b) for a, b in zip(left.values, right.values)
        )
    if isinstance(left, MapValue) and isinstance(right, MapValue):
        return left == right
    if left is None and right is None:
        return True
    return False


def _is_scalar_key(value: Value) -> bool:
    return not isinstance(value, (ArrayValue, MapValue))


def _validate_map_key(
    key: Value,
    index: int,
    entries: Sequence[tuple[Value, Value]],
) -> None:
    if not _is_scalar_key(key):
        raise StructuredValueError(
            f"map key at entry {index} is not scalar",
            ValueErrorKind.NON_SCALAR_KEY,
        )
    if any(model_equal(key, previous) for previous, _ in entries):
        raise StructuredValueError(
            f"duplicate map key at entry {index}",
            ValueErrorKind.DUPLICATE_KEY,
        )


def encode_value(value: object, *, limits: ValueLimits | None = None) -> bytes:
    """Encodes a Python/native or lossless value as one CBOR item."""

    budget = limits or ValueLimits()
    model = to_value(value)
    output = bytearray()
    tasks: list[tuple[Value, int]] = [(model, 0)]
    item_count = 0
    while tasks:
        current, depth = tasks.pop()
        item_count += 1
        if item_count > budget.max_items:
            _resource("items", budget.max_items, item_count)
        if isinstance(current, UndefinedValue):
            _append(output, b"\xf7", budget)
        elif current is None:
            _append(output, b"\xf6", budget)
        elif isinstance(current, bool):
            _append(output, b"\xf5" if current else b"\xf4", budget)
        elif isinstance(current, IntegerValue):
            _encode_integer(current.value, output, budget)
        elif isinstance(current, FloatValue):
            width_bytes = {16: 2, 32: 4, 64: 8}[current.width]
            _append(output, bytes((0xF9 if width_bytes == 2 else 0xFA if width_bytes == 4 else 0xFB,)), budget)
            _append(output, current.raw_bits.to_bytes(width_bytes, "big"), budget)
        elif isinstance(current, TextStringValue):
            data = current.value.encode("utf-8")
            _append_head(3, len(data), output, budget)
            _append(output, data, budget)
        elif isinstance(current, ByteStringValue):
            _append_head(2, len(current.value), output, budget)
            _append(output, current.value, budget)
        elif isinstance(current, ArrayValue):
            if depth >= budget.max_depth:
                _resource("depth", budget.max_depth, depth + 1)
            _append_head(4, len(current.values), output, budget)
            if item_count + len(current.values) > budget.max_items:
                _resource("items", budget.max_items, item_count + len(current.values))
            tasks.extend((child, depth + 1) for child in reversed(current.values))
        elif isinstance(current, MapValue):
            if depth >= budget.max_depth:
                _resource("depth", budget.max_depth, depth + 1)
            _append_head(5, len(current.entries), output, budget)
            if item_count + 2 * len(current.entries) > budget.max_items:
                _resource("items", budget.max_items, item_count + 2 * len(current.entries))
            for key, child in reversed(current.entries):
                tasks.append((child, depth + 1))
                tasks.append((key, depth + 1))
        else:  # pragma: no cover - all values pass through to_value
            raise StructuredValueError("unsupported model value")
    return bytes(output)


def _resource(name: str, limit: int, actual: int) -> None:
    raise StructuredValueError(
        f"{name} limit {limit} exceeded by {actual}",
        ValueErrorKind.RESOURCE_LIMIT,
    )


def _append(output: bytearray, data: bytes, budget: ValueLimits) -> None:
    if len(output) + len(data) > budget.max_bytes:
        _resource("bytes", budget.max_bytes, len(output) + len(data))
    output.extend(data)


def _append_head(major: int, length: int, output: bytearray, budget: ValueLimits) -> None:
    if length < 24:
        head = bytes(((major << 5) | length,))
    elif length <= 0xFF:
        head = bytes(((major << 5) | 24, length))
    elif length <= 0xFFFF:
        head = bytes(((major << 5) | 25,)) + length.to_bytes(2, "big")
    elif length <= 0xFFFF_FFFF:
        head = bytes(((major << 5) | 26,)) + length.to_bytes(4, "big")
    elif length <= 0xFFFF_FFFF_FFFF_FFFF:
        head = bytes(((major << 5) | 27,)) + length.to_bytes(8, "big")
    else:
        _resource("bytes", budget.max_bytes, length)
    _append(output, head, budget)


def _encode_integer(value: int, output: bytearray, budget: ValueLimits) -> None:
    negative = value < 0
    transformed = -value - 1 if negative else value
    if transformed <= 0xFFFF_FFFF_FFFF_FFFF:
        _append_head(1 if negative else 0, transformed, output, budget)
        return
    magnitude = transformed.to_bytes(max(1, (transformed.bit_length() + 7) // 8), "big")
    if len(magnitude) > budget.max_integer_bytes:
        _resource("integer bytes", budget.max_integer_bytes, len(magnitude))
    _append_head(6, 3 if negative else 2, output, budget)
    _append_head(2, len(magnitude), output, budget)
    _append(output, magnitude, budget)


def decode_value(data: bytes | bytearray | memoryview, *, limits: ValueLimits | None = None) -> Value:
    """Decodes exactly one complete StructuredValue-CBOR-v1 item."""

    budget = limits or ValueLimits()
    source = bytes(data)
    if len(source) > budget.max_bytes:
        _resource("bytes", budget.max_bytes, len(source))
    if not source:
        raise StructuredValueError("value is truncated", ValueErrorKind.TRUNCATED)
    cursor = 0
    frames: list[list[Any]] = []
    missing = object()
    pending_key = object()
    root: Value | object = missing
    item_count = 0

    def accept(value: Value) -> None:
        nonlocal root
        while True:
            if not frames:
                root = value
                return
            frame = frames[-1]
            if frame[0] == "array":
                frame[2].append(value)
            else:
                if frame[3] is pending_key:
                    _validate_map_key(value, len(frame[2]), frame[2])
                    frame[3] = value
                else:
                    frame[2].append((frame[3], value))
                    frame[3] = pending_key
            frame[1] -= 1
            if frame[1] != 0:
                return
            frames.pop()
            value = (
                ArrayValue(frame[2])
                if frame[0] == "array"
                else MapValue(frame[2])
            )

    while root is missing:
        item_count += 1
        if item_count > budget.max_items:
            _resource("items", budget.max_items, item_count)
        major, argument = _read_head(source, cursor)
        cursor = argument[1]
        ai, length_or_value = argument[0], argument[2]
        if major in (0, 1):
            number = length_or_value
            value = IntegerValue(number if major == 0 else -number - 1)
            if isinstance(value, IntegerValue) and value.value.bit_length() > budget.max_integer_bytes * 8:
                _resource("integer bytes", budget.max_integer_bytes, (value.value.bit_length() + 7) // 8)
            accept(value)
        elif major in (2, 3):
            length = length_or_value
            # Check the declared bounded resource before looking at payload
            # availability. This keeps malformed vectors deterministic across
            # language bindings (for example ``58 05 41`` with max_bytes=4
            # is a resource-limit failure, even though its body is truncated).
            if length > budget.max_bytes:
                _resource("bytes", budget.max_bytes, length)
            if cursor + length > len(source):
                raise StructuredValueError("value is truncated", ValueErrorKind.TRUNCATED)
            content = source[cursor : cursor + length]
            cursor += length
            if major == 2:
                accept(ByteStringValue(content))
            else:
                try:
                    text = content.decode("utf-8")
                except UnicodeDecodeError as error:
                    raise StructuredValueError("text is not valid UTF-8", ValueErrorKind.INVALID_UTF8) from error
                accept(TextStringValue(text))
        elif major == 4:
            length = length_or_value
            if length > budget.max_items:
                _resource("items", budget.max_items, length)
            if length == 0:
                accept(ArrayValue(()))
            else:
                if len(frames) >= budget.max_depth:
                    _resource("depth", budget.max_depth, len(frames) + 1)
                frames.append(["array", length, []])
        elif major == 5:
            length = length_or_value
            if length * 2 > budget.max_items:
                _resource("items", budget.max_items, length * 2)
            if length == 0:
                accept(MapValue(()))
            else:
                if len(frames) >= budget.max_depth:
                    _resource("depth", budget.max_depth, len(frames) + 1)
                frames.append(["map", length * 2, [], pending_key])
        elif major == 6:
            # Tags 2 and 3 are the only profile tags.  Their payload must be a
            # definite byte string, handled directly so it cannot become a
            # second model item or an array/map frame.
            tag = length_or_value
            if tag not in (2, 3):
                raise StructuredValueError("unsupported CBOR tag", ValueErrorKind.UNSUPPORTED_TYPE)
            bmajor, bargument = _read_head(source, cursor)
            cursor = bargument[1]
            if bmajor != 2 or bargument[0] == 31:
                raise StructuredValueError("bignum tag must wrap bytes", ValueErrorKind.INVALID_INTEGER)
            length = bargument[2]
            if length == 0:
                raise StructuredValueError("bignum magnitude must not be empty", ValueErrorKind.INVALID_INTEGER)
            if length > budget.max_integer_bytes:
                _resource("integer bytes", budget.max_integer_bytes, length)
            if cursor + length > len(source):
                raise StructuredValueError("bignum magnitude is truncated", ValueErrorKind.TRUNCATED)
            magnitude = source[cursor : cursor + length]
            cursor += length
            if magnitude[0] == 0:
                raise StructuredValueError("bignum magnitude is not minimal", ValueErrorKind.INVALID_INTEGER)
            number = int.from_bytes(magnitude, "big")
            value = IntegerValue(number if tag == 2 else -number - 1)
            accept(value)
        elif major == 7:
            if ai == 20:
                accept(False)
            elif ai == 21:
                accept(True)
            elif ai == 22:
                accept(None)
            elif ai == 23:
                accept(UNDEFINED)
            elif ai == 25:
                accept(FloatValue(16, length_or_value))
            elif ai == 26:
                accept(FloatValue(32, length_or_value))
            elif ai == 27:
                accept(FloatValue(64, length_or_value))
            else:
                raise StructuredValueError("unsupported CBOR simple value", ValueErrorKind.UNSUPPORTED_TYPE)
        else:
            raise StructuredValueError("unsupported CBOR major type", ValueErrorKind.UNSUPPORTED_TYPE)

    if frames:
        raise StructuredValueError("value is truncated", ValueErrorKind.TRUNCATED)
    if cursor != len(source):
        raise StructuredValueError("trailing CBOR bytes", ValueErrorKind.TRAILING_BYTES)
    return root  # type: ignore[return-value]


def _read_head(source: bytes, cursor: int) -> tuple[int, tuple[int, int, int]]:
    if cursor >= len(source):
        raise StructuredValueError("value is truncated", ValueErrorKind.TRUNCATED)
    offset = cursor
    first = source[cursor]
    cursor += 1
    major, ai = first >> 5, first & 0x1F
    if ai < 24:
        return major, (ai, cursor, ai)
    if ai == 24:
        width = 1
    elif ai == 25:
        width = 2
    elif ai == 26:
        width = 4
    elif ai == 27:
        width = 8
    elif ai == 31:
        raise StructuredValueError(
            f"indefinite-length item at byte {offset}",
            ValueErrorKind.INVALID_ENCODING,
        )
    else:
        raise StructuredValueError("reserved CBOR additional information", ValueErrorKind.INVALID_ENCODING)
    if cursor + width > len(source):
        raise StructuredValueError("CBOR head is truncated", ValueErrorKind.TRUNCATED)
    value = int.from_bytes(source[cursor : cursor + width], "big")
    return major, (ai, cursor + width, value)


def to_native(value: Value) -> object:
    """Projects a lossless model value into ordinary Python native values."""

    if isinstance(value, UndefinedValue):
        raise StructuredValueError("Undefined has no Python native projection")
    if value is None or isinstance(value, bool):
        return value
    if isinstance(value, IntegerValue):
        return value.value
    if isinstance(value, FloatValue):
        if value.width == 16:
            return struct.unpack(">e", value.raw_bits.to_bytes(2, "big"))[0]
        if value.width == 32:
            return struct.unpack(">f", value.raw_bits.to_bytes(4, "big"))[0]
        return struct.unpack(">d", value.raw_bits.to_bytes(8, "big"))[0]
    if isinstance(value, ByteStringValue):
        return bytes(value.value)
    if isinstance(value, TextStringValue):
        return value.value
    if isinstance(value, ArrayValue):
        return [to_native(child) for child in value.values]
    if isinstance(value, MapValue):
        output: dict[object, object] = {}
        projected: list[object] = []
        for key, child in value.entries:
            native_key = to_native(key)
            if any(_native_keys_collapse(native_key, previous) for previous in projected):
                raise StructuredValueError(
                    "map keys cannot be represented by a Python dict without loss",
                    ValueErrorKind.CONVERSION,
                )
            projected.append(native_key)
            output[native_key] = to_native(child)
        return output
    raise StructuredValueError("unsupported model value")


def _native_keys_collapse(left: object, right: object) -> bool:
    # Python's bool/int and int/float equality are intentionally stricter than
    # the model.  Distinct NaN payloads are not collapsed by Python equality.
    if type(left) is not type(right):
        return left == right
    return left == right


def decode_native(
    data: bytes | bytearray | memoryview,
    *,
    limits: ValueLimits | None = None,
) -> object:
    """Decodes a payload and applies the strict native projection."""

    return to_native(decode_value(data, limits=limits))


__all__ = [
    "Array",
    "ArrayValue",
    "ByteString",
    "ByteStringValue",
    "Float",
    "FloatValue",
    "Integer",
    "IntegerValue",
    "Map",
    "MapValue",
    "StructuredValueError",
    "TextString",
    "TextStringValue",
    "UNDEFINED",
    "Undefined",
    "UndefinedValue",
    "Value",
    "ValueErrorKind",
    "ValueLimits",
    "decode_native",
    "decode_value",
    "encode_value",
    "model_equal",
    "to_native",
    "to_value",
]
