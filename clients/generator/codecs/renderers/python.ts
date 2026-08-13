/** python field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_python_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `def _smithy_encode_field_varuint(value: int) -> bytes:
    if value < 0:
        raise ValueError("field length is negative")
    if value < 0x80:
        return bytes((value,))
    if value < 0x4000:
        return bytes((0x80 | (value & 0x3f), value >> 6))
    if value < 0x200000:
        return bytes((0xc0 | (value & 0x1f), value >> 5, value >> 13))
    if value < 0x10000000:
        return bytes((0xe0 | (value & 0x0f), value >> 4, value >> 12, value >> 20))
    width = 8
    while width > 1 and value >> ((width - 1) * 8) == 0:
        width -= 1
    return bytes((0xf0 | (width - 1),)) + bytes(
        (value >> (index * 8)) & 0xff for index in range(width)
    )


def _smithy_decode_field_varuint(payload: bytes, offset: int, operation: int) -> tuple[int, int]:
    if offset >= len(payload):
        raise ValueError(f"operation {operation} field length is truncated")
    first = payload[offset]
    width = (
        1 if first < 0x80 else
        2 if first < 0xc0 else
        3 if first < 0xe0 else
        4 if first < 0xf0 else
        (first & 0x0f) + 2
    )
    if width > 9 or offset + width > len(payload):
        raise ValueError(f"operation {operation} field length is truncated")
    if width == 1:
        value = first
    elif width == 2:
        value = (first & 0x3f) | (payload[offset + 1] << 6)
    elif width == 3:
        value = (first & 0x1f) | (payload[offset + 1] << 5) | (payload[offset + 2] << 13)
    elif width == 4:
        value = (
            (first & 0x0f) |
            (payload[offset + 1] << 4) |
            (payload[offset + 2] << 12) |
            (payload[offset + 3] << 20)
        )
    else:
        value = sum(payload[offset + index] << ((index - 1) * 8) for index in range(1, width))
    if len(_smithy_encode_field_varuint(value)) != width:
        raise ValueError(f"operation {operation} field length is non-canonical")
    return value, offset + width


def _smithy_encode_field_sequence(values: list[bytes | None]) -> bytes:
    mask_bytes = (len(values) + 7) // 8
    payload = bytearray(mask_bytes)
    last_present = next(
        (index for index in range(len(values) - 1, -1, -1) if values[index] is not None),
        -1,
    )
    for index, value in enumerate(values):
        if value is not None and len(value) > ${framing.max_value_bytes}:
            raise ValueError("field-sequence entry exceeds the maximum value size")
        if value is None:
            continue
        encoded_length = (
            b"" if index == last_present else _smithy_encode_field_varuint(len(value))
        )
        next_length = len(payload) + len(encoded_length) + len(value)
        if next_length > ${framing.max_value_bytes}:
            raise ValueError("field-sequence payload exceeds the maximum value size")
        payload[index // 8] |= 1 << (index % 8)
        payload.extend(encoded_length)
        payload.extend(value)
    return bytes(payload)


def _smithy_decode_field_sequence(
    payload: bytes,
    field_count: int,
    operation: int,
) -> list[bytes | None]:
    values: list[bytes | None] = [None] * field_count
    mask_bytes = (field_count + 7) // 8
    if len(payload) < mask_bytes:
        raise ValueError(f"operation {operation} field sequence is missing its presence mask")
    if (
        mask_bytes > 0
        and field_count % 8 != 0
        and payload[mask_bytes - 1] & ~((1 << (field_count % 8)) - 1)
    ):
        raise ValueError(
            f"operation {operation} field sequence presence mask has unused bits set"
        )
    last_present = next(
        (
            index
            for index in range(field_count - 1, -1, -1)
            if payload[index // 8] & (1 << (index % 8))
        ),
        -1,
    )
    offset = mask_bytes
    for index in range(field_count):
        if not (payload[index // 8] & (1 << (index % 8))):
            continue
        if index == last_present:
            length = len(payload) - offset
        else:
            length, offset = _smithy_decode_field_varuint(payload, offset, operation)
        end = offset + length
        if length > ${framing.max_value_bytes} or end > len(payload):
            raise ValueError(f"operation {operation} field sequence entry is truncated")
        values[index] = payload[offset:end]
        offset = end
    if offset != len(payload):
        raise ValueError(f"operation {operation} field sequence contains trailing bytes")
    return values


def _smithy_encode_dense_fields(values: list[bytes]) -> bytes:
    total = sum(len(value) for value in values)
    if total > ${framing.max_value_bytes}:
        raise ValueError("dense field payload exceeds the maximum value size")
    return b"".join(values)


def _smithy_decode_dense_fields(
    payload: bytes,
    widths: list[int],
    operation: int,
) -> list[bytes | None]:
    values: list[bytes | None] = [None] * len(widths)
    offset = 0
    for index, width in enumerate(widths):
        if width < 0 or width > len(payload) - offset:
            raise ValueError(f"operation {operation} dense field payload is truncated")
        values[index] = payload[offset:offset + width]
        offset += width
    if offset != len(payload):
        raise ValueError(f"operation {operation} dense field payload has trailing bytes")
    return values


def _smithy_encode_u64(value: int) -> bytes:
    if value < 0 or value >= 1 << 64:
        raise ValueError("u64 field is outside the wire range")
    return value.to_bytes(8, "big")


def _smithy_decode_u64(payload: bytes, operation: int) -> int:
    if len(payload) != 8:
        raise ValueError(f"operation {operation} response has an invalid u64 field")
    return int.from_bytes(payload, "big")


def _smithy_encode_bool(value: bool) -> bytes:
    return bytes((1 if value else 0,))


def _smithy_decode_bool(payload: bytes, operation: int) -> bool:
    if len(payload) != 1 or payload[0] not in (0, 1):
        raise ValueError(f"operation {operation} response has an invalid boolean field")
    return payload[0] == 1


def _smithy_encode_f64(value: float) -> bytes:
    if not math.isfinite(value):
        raise ValueError("binary64 field must be finite")
    return struct.pack(">d", value)


def _smithy_decode_f64(payload: bytes, operation: int) -> float:
    if len(payload) != 8:
        raise ValueError(f"operation {operation} response has an invalid f64 field")
    value = struct.unpack(">d", payload)[0]
    if not math.isfinite(value):
        raise ValueError(f"operation {operation} response contains a non-finite f64 field")
    return value


def _smithy_encode_i32(value: int) -> bytes:
    if value < -(1 << 31) or value >= 1 << 31:
        raise ValueError("i32 field is outside the wire range")
    return value.to_bytes(4, "big", signed=True)


def _smithy_decode_i32(payload: bytes, operation: int) -> int:
    if len(payload) != 4:
        raise ValueError(f"operation {operation} response has an invalid i32 field")
    return int.from_bytes(payload, "big", signed=True)
`
}

export function render_python_container_helpers(max_value_bytes: number): string {
  return `def _smithy_encode_varuint(value: int) -> bytes:
    if value < 0:
        raise ValueError("container count is negative")
    if value < 0x80:
        return bytes((value,))
    if value < 0x4000:
        return bytes((0x80 | (value & 0x3f), value >> 6))
    if value < 0x200000:
        return bytes((0xc0 | (value & 0x1f), value >> 5, value >> 13))
    if value < 0x10000000:
        return bytes((0xe0 | (value & 0x0f), value >> 4, value >> 12, value >> 20))
    width = 8
    while width > 1 and value >> ((width - 1) * 8) == 0:
        width -= 1
    return bytes((0xf0 | (width - 1),)) + value.to_bytes(width, "little")


def _smithy_decode_varuint(payload: bytes, offset: int, operation: int) -> tuple[int, int]:
    if offset >= len(payload):
        raise ValueError(f"operation {operation} container count is truncated")
    first = payload[offset]
    width = (
        1 if first < 0x80 else
        2 if first < 0xc0 else
        3 if first < 0xe0 else
        4 if first < 0xf0 else (first & 0x0F) + 2
    )
    if width > 9 or offset + width > len(payload):
        raise ValueError(f"operation {operation} container count is truncated")
    if width == 1:
        value = first
    elif width == 2:
        value = (first & 0x3f) | (payload[offset + 1] << 6)
    elif width == 3:
        value = (first & 0x1f) | (payload[offset + 1] << 5) | (payload[offset + 2] << 13)
    elif width == 4:
        value = (first & 0x0f) | (payload[offset + 1] << 4) | (payload[offset + 2] << 12) | (payload[offset + 3] << 20)
    else:
        value = int.from_bytes(payload[offset + 1:offset + width], "little")
    if len(_smithy_encode_varuint(value)) != width:
        raise ValueError(f"operation {operation} container count is non-canonical")
    return value, offset + width


def _smithy_encode_length_delimited(value: bytes) -> bytes:
    if len(value) > ${max_value_bytes}:
        raise ValueError("container entry exceeds the maximum value size")
    return _smithy_encode_varuint(len(value)) + value


def _smithy_read_length_delimited(payload: bytes, offset: int, operation: int) -> tuple[bytes, int]:
    length, value_start = _smithy_decode_varuint(payload, offset, operation)
    if length > ${max_value_bytes} or value_start + length > len(payload):
        raise ValueError(f"operation {operation} container entry is malformed")
    return payload[value_start:value_start + length], value_start + length


def _smithy_encode_list(values: list[bytes]) -> bytes:
    return _smithy_encode_varuint(len(values)) + b"".join(
        _smithy_encode_length_delimited(value) for value in values
    )


def _smithy_decode_list(payload: bytes, operation: int) -> list[bytes]:
    count, offset = _smithy_decode_varuint(payload, 0, operation)
    values: list[bytes] = []
    for _ in range(count):
        value, offset = _smithy_read_length_delimited(payload, offset, operation)
        values.append(value)
    if offset != len(payload):
        raise ValueError(f"operation {operation} list has trailing bytes")
    return values


def _smithy_encode_map(values: list[tuple[bytes, bytes]]) -> bytes:
    return _smithy_encode_varuint(len(values)) + b"".join(
        _smithy_encode_length_delimited(part)
        for key, value in values
        for part in (key, value)
    )


def _smithy_decode_map(payload: bytes, operation: int) -> list[tuple[bytes, bytes]]:
    count, offset = _smithy_decode_varuint(payload, 0, operation)
    values: list[tuple[bytes, bytes]] = []
    for _ in range(count):
        key, offset = _smithy_read_length_delimited(payload, offset, operation)
        value, offset = _smithy_read_length_delimited(payload, offset, operation)
        values.append((key, value))
    if offset != len(payload):
        raise ValueError(f"operation {operation} map has trailing bytes")
    return values


def _smithy_encode_union(payload: bytes, operation: int) -> bytes:
    return _smithy_decode_union(payload, operation)


def _smithy_decode_union(payload: bytes, operation: int) -> bytes:
    if len(payload) < 2:
        raise ValueError(f"operation {operation} union payload is truncated")
    _, end = _smithy_read_length_delimited(payload, 1, operation)
    if end != len(payload):
        raise ValueError(f"operation {operation} union payload has trailing bytes")
    return payload
`
}

/** Shared Swift helpers for ordered field-sequence framing and scalars. */
