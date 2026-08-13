/** typescript field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_typescript_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `function smithy_encode_field_var_uint(value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error("field length is invalid");
  }
  if (value < 0x80) return Uint8Array.of(value);
  if (value < 0x4000) return Uint8Array.of(0x80 | (value & 0x3f), value >> 6);
  if (value < 0x200000) {
    return Uint8Array.of(0xc0 | (value & 0x1f), value >> 5, value >> 13);
  }
  if (value < 0x10000000) {
    return Uint8Array.of(0xe0 | (value & 0x0f), value >> 4, value >> 12, value >> 20);
  }
  let width = 8;
  while (width > 1 && Math.floor(value / 2 ** ((width - 1) * 8)) === 0) width--;
  const result = new Uint8Array(width + 1);
  result[0] = 0xf0 | (width - 1);
  for (let index = 0; index < width; index++) {
    result[index + 1] = Math.floor(value / 2 ** (index * 8)) & 0xff;
  }
  return result;
}

function smithy_decode_field_var_uint(
  payload: Uint8Array,
  offset: number,
  operation: number,
): readonly [number, number] {
  if (offset >= payload.byteLength) {
    throw new Error(\`operation \${operation} field length is truncated\`);
  }
  const first = payload[offset]!;
  const width = first < 0x80
    ? 1
    : first < 0xc0
    ? 2
    : first < 0xe0
    ? 3
    : first < 0xf0
    ? 4
    : (first & 0x0f) + 2;
  if (width > 9 || offset + width > payload.byteLength) {
    throw new Error(\`operation \${operation} field length is truncated\`);
  }
  let value = 0;
  if (width === 1) value = first;
  else if (width === 2) value = (first & 0x3f) | (payload[offset + 1]! << 6);
  else if (width === 3) {
    value = (first & 0x1f) |
      (payload[offset + 1]! << 5) |
      (payload[offset + 2]! << 13);
  } else if (width === 4) {
    value = (first & 0x0f) |
      (payload[offset + 1]! << 4) |
      (payload[offset + 2]! << 12) |
      (payload[offset + 3]! << 20);
  } else {
    for (let index = 1; index < width; index++) {
      value += payload[offset + index]! * 2 ** ((index - 1) * 8);
    }
  }
  if (smithy_encode_field_var_uint(value).byteLength !== width) {
    throw new Error(\`operation \${operation} field length is non-canonical\`);
  }
  return [value, offset + width];
}

export function smithy_encode_field_sequence(
  values: readonly (Uint8Array | undefined)[],
): Uint8Array {
  const maskBytes = Math.ceil(values.length / 8);
  let lastPresent = -1;
  values.forEach((value, index) => {
    if (value !== undefined) lastPresent = index;
  });
  let total = maskBytes;
  values.forEach((value, index) => {
    if (value !== undefined && value.byteLength > ${framing.max_value_bytes}) {
      throw new Error("field-sequence entry exceeds the maximum value size");
    }
    if (value === undefined) return;
    const lengthBytes = index === lastPresent
      ? 0
      : smithy_encode_field_var_uint(value.byteLength).byteLength;
    if (
      total > ${framing.max_value_bytes} ||
      lengthBytes > ${framing.max_value_bytes} - total ||
      value.byteLength > ${framing.max_value_bytes} - total - lengthBytes
    ) {
      throw new Error("field-sequence payload exceeds the maximum value size");
    }
    total += lengthBytes + value.byteLength;
  });
  const payload = new Uint8Array(total);
  let offset = maskBytes;
  values.forEach((value, index) => {
    if (value !== undefined) {
      payload[index >> 3] = payload[index >> 3]! | (1 << (index & 7));
      if (index !== lastPresent) {
        const encodedLength = smithy_encode_field_var_uint(value.byteLength);
        payload.set(encodedLength, offset);
        offset += encodedLength.byteLength;
      }
      payload.set(value, offset);
      offset += value.byteLength;
    }
  });
  return payload;
}

export function smithy_decode_field_sequence(
  payload: Uint8Array,
  fieldCount: number,
  operation: number,
): (Uint8Array | undefined)[] {
  const values: (Uint8Array | undefined)[] = Array(fieldCount).fill(undefined);
  const maskBytes = Math.ceil(fieldCount / 8);
  if (payload.byteLength < maskBytes) {
    throw new Error(\`operation \${operation} field sequence is missing its presence mask\`);
  }
  if (
    maskBytes > 0 &&
    fieldCount % 8 !== 0 &&
    (payload[maskBytes - 1]! & ~((1 << (fieldCount % 8)) - 1)) !== 0
  ) {
    throw new Error(\`operation \${operation} field sequence presence mask has unused bits set\`);
  }
  let lastPresent = -1;
  for (let index = fieldCount - 1; index >= 0; index--) {
    if ((payload[index >> 3]! & (1 << (index & 7))) !== 0) {
      lastPresent = index;
      break;
    }
  }
  let offset = maskBytes;
  for (let index = 0; index < fieldCount; index++) {
    if ((payload[index >> 3]! & (1 << (index & 7))) === 0) continue;
    const [length, next] = index === lastPresent
      ? [payload.byteLength - offset, offset] as const
      : smithy_decode_field_var_uint(payload, offset, operation);
    if (length > ${framing.max_value_bytes} || next + length > payload.byteLength) {
      throw new Error(\`operation \${operation} field sequence entry is truncated\`);
    }
    values[index] = payload.slice(next, next + length);
    offset = next + length;
  }
  if (offset !== payload.byteLength) {
    throw new Error(\`operation \${operation} field sequence contains trailing bytes\`);
  }
  return values;
}

export function smithy_encode_dense_fields(values: readonly Uint8Array[]): Uint8Array {
  const total = values.reduce((sum, value) => sum + value.byteLength, 0);
  if (total > ${framing.max_value_bytes}) {
    throw new Error("dense field payload exceeds the maximum value size");
  }
  const payload = new Uint8Array(total);
  let offset = 0;
  for (const value of values) {
    payload.set(value, offset);
    offset += value.byteLength;
  }
  return payload;
}

export function smithy_decode_dense_fields(
  payload: Uint8Array,
  widths: readonly number[],
  operation: number,
): (Uint8Array | undefined)[] {
  const values: (Uint8Array | undefined)[] = Array(widths.length).fill(undefined);
  let offset = 0;
  for (let index = 0; index < widths.length; index++) {
    const width = widths[index]!;
    if (width < 0 || width > payload.byteLength - offset) {
      throw new Error(\`operation \${operation} dense field payload is truncated\`);
    }
    values[index] = payload.slice(offset, offset + width);
    offset += width;
  }
  if (offset !== payload.byteLength) {
    throw new Error(\`operation \${operation} dense field payload has trailing bytes\`);
  }
  return values;
}

function smithy_encode_u64(value: number | bigint): Uint8Array {
  const payload = new Uint8Array(8);
  new DataView(payload.buffer).setBigUint64(0, BigInt.asUintN(64, BigInt(value)), false);
  return payload;
}

function smithy_decode_u64(payload: Uint8Array, operation: number): bigint {
  if (payload.byteLength !== 8) {
    throw new Error(\`operation \${operation} response has an invalid u64 field\`);
  }
  return new DataView(payload.buffer, payload.byteOffset, payload.byteLength)
    .getBigUint64(0, false);
}

function smithy_encode_bool(value: boolean): Uint8Array {
  return Uint8Array.of(value ? 1 : 0);
}

function smithy_decode_bool(payload: Uint8Array, operation: number): boolean {
  if (payload.byteLength !== 1 || (payload[0] !== 0 && payload[0] !== 1)) {
    throw new Error(\`operation \${operation} response has an invalid boolean field\`);
  }
  return payload[0] === 1;
}

function smithy_encode_f64(value: number): Uint8Array {
  if (!Number.isFinite(value)) {
    throw new Error("binary64 field must be finite");
  }
  const payload = new Uint8Array(8);
  new DataView(payload.buffer).setFloat64(0, value, false);
  return payload;
}

function smithy_decode_f64(payload: Uint8Array, operation: number): number {
  if (payload.byteLength !== 8) {
    throw new Error(\`operation \${operation} response has an invalid f64 field\`);
  }
  const value = new DataView(payload.buffer, payload.byteOffset, payload.byteLength)
    .getFloat64(0, false);
  if (!Number.isFinite(value)) {
    throw new Error(\`operation \${operation} response contains a non-finite f64 field\`);
  }
  return value;
}

function smithy_encode_i32(value: number): Uint8Array {
  const payload = new Uint8Array(4);
  new DataView(payload.buffer).setInt32(0, value, false);
  return payload;
}

function smithy_decode_i32(payload: Uint8Array, operation: number): number {
  if (payload.byteLength !== 4) {
    throw new Error(\`operation \${operation} response has an invalid i32 field\`);
  }
  return new DataView(payload.buffer, payload.byteOffset, payload.byteLength)
    .getInt32(0, false);
}
`
}

export function render_typescript_container_helpers(max_value_bytes: number): string {
  return `function smithy_encode_varuint(value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error("container count is outside the supported vu128 range");
  }
  if (value < 0x80) return Uint8Array.of(value);
  if (value < 0x4000) {
    return Uint8Array.of(0x80 | (value & 0x3f), value >>> 6);
  }
  if (value < 0x200000) {
    return Uint8Array.of(0xc0 | (value & 0x1f), value >>> 5, value >>> 13);
  }
  if (value < 0x10000000) {
    return Uint8Array.of(
      0xe0 | (value & 0x0f),
      value >>> 4,
      value >>> 12,
      value >>> 20,
    );
  }
  let width = 8;
  while (width > 1 && Math.floor(value / 2 ** ((width - 1) * 8)) === 0) width--;
  const result = new Uint8Array(width + 1);
  result[0] = 0xf0 | (width - 1);
  let remaining = value;
  for (let index = 0; index < width; index++) {
    result[index + 1] = remaining & 0xff;
    remaining = Math.floor(remaining / 256);
  }
  return result;
}

function smithy_decode_varuint(
  payload: Uint8Array,
  offset: number,
  operation: number,
): readonly [number, number] {
  const first = payload[offset];
  if (first === undefined) throw new Error(\`operation \${operation} container count is truncated\`);
  const width = first < 0x80
    ? 1
    : first < 0xc0
    ? 2
    : first < 0xe0
    ? 3
    : first < 0xf0
    ? 4
    : (first & 0x0f) + 2;
  if (offset + width > payload.byteLength || width > 9) {
    throw new Error(\`operation \${operation} container count is truncated\`);
  }
  let value = 0;
  if (width === 1) value = first;
  else if (width === 2) value = (first & 0x3f) | (payload[offset + 1]! << 6);
  else if (width === 3) {
    value = (first & 0x1f) |
      (payload[offset + 1]! << 5) |
      (payload[offset + 2]! * 2 ** 13);
  } else if (width === 4) {
    value = (first & 0x0f) |
      (payload[offset + 1]! * 2 ** 4) |
      (payload[offset + 2]! * 2 ** 12) |
      (payload[offset + 3]! * 2 ** 20);
  } else {
    for (let index = 1; index < width; index++) {
      value += payload[offset + index]! * 2 ** ((index - 1) * 8);
    }
  }
  if (!Number.isSafeInteger(value) || smithy_encode_varuint(value).byteLength !== width) {
    throw new Error(\`operation \${operation} container count is non-canonical\`);
  }
  return [value, offset + width];
}

function smithy_encode_length_delimited(value: Uint8Array): Uint8Array {
  if (value.byteLength > ${max_value_bytes}) {
    throw new Error("container entry exceeds the maximum value size");
  }
  const encoded_length = smithy_encode_varuint(value.byteLength);
  const result = new Uint8Array(encoded_length.byteLength + value.byteLength);
  result.set(encoded_length);
  result.set(value, encoded_length.byteLength);
  return result;
}

function smithy_read_length_delimited(
  payload: Uint8Array,
  offset: number,
  operation: number,
): readonly [Uint8Array, number] {
  const [length, value_start] = smithy_decode_varuint(payload, offset, operation);
  if (length > ${max_value_bytes} || value_start + length > payload.byteLength) {
    throw new Error(\`operation \${operation} container entry is malformed\`);
  }
  return [payload.slice(value_start, value_start + length), value_start + length];
}

function smithy_encode_list(values: readonly Uint8Array[]): Uint8Array {
  const chunks = [smithy_encode_varuint(values.length), ...values.map(smithy_encode_length_delimited)];
  const result = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.byteLength, 0));
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

function smithy_decode_list(payload: Uint8Array, operation: number): Uint8Array[] {
  const [count, start] = smithy_decode_varuint(payload, 0, operation);
  const values: Uint8Array[] = [];
  let offset = start;
  for (let index = 0; index < count; index++) {
    const [value, next] = smithy_read_length_delimited(payload, offset, operation);
    values.push(value);
    offset = next;
  }
  if (offset !== payload.byteLength) {
    throw new Error(\`operation \${operation} list has trailing bytes\`);
  }
  return values;
}

function smithy_encode_map(
  entries: readonly (readonly [Uint8Array, Uint8Array])[],
): Uint8Array {
  const chunks = [
    smithy_encode_varuint(entries.length),
    ...entries.flatMap(([key, value]) => [
      smithy_encode_length_delimited(key),
      smithy_encode_length_delimited(value),
    ]),
  ];
  const result = new Uint8Array(chunks.reduce((total, chunk) => total + chunk.byteLength, 0));
  let offset = 0;
  for (const chunk of chunks) {
    result.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return result;
}

function smithy_decode_map(
  payload: Uint8Array,
  operation: number,
): Array<readonly [Uint8Array, Uint8Array]> {
  const [count, start] = smithy_decode_varuint(payload, 0, operation);
  const entries: Array<readonly [Uint8Array, Uint8Array]> = [];
  let offset = start;
  for (let index = 0; index < count; index++) {
    const [key, nextKey] = smithy_read_length_delimited(payload, offset, operation);
    const [value, nextValue] = smithy_read_length_delimited(payload, nextKey, operation);
    entries.push([key, value]);
    offset = nextValue;
  }
  if (offset !== payload.byteLength) {
    throw new Error(\`operation \${operation} map has trailing bytes\`);
  }
  return entries;
}

function smithy_encode_union(payload: Uint8Array, operation: number): Uint8Array {
  smithy_decode_union(payload, operation);
  return payload;
}

function smithy_decode_union(payload: Uint8Array, operation: number): Uint8Array {
  if (payload.byteLength < 2) {
    throw new Error(\`operation \${operation} union payload is truncated\`);
  }
  const [, end] = smithy_read_length_delimited(payload, 1, operation);
  if (end !== payload.byteLength) {
    throw new Error(\`operation \${operation} union payload has trailing bytes\`);
  }
  return payload;
}

function smithy_decode_enum(
  payload: Uint8Array,
  allowed: readonly string[],
  operation: number,
): string {
  const value = new TextDecoder().decode(payload);
  if (!allowed.includes(value)) {
    throw new Error(\`operation \${operation} response contains an unknown enum value\`);
  }
  return value;
}
`
}

/** Shared Python helpers for ordered field-sequence framing and scalars. */
