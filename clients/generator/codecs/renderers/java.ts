/** java field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_java_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `    private static byte[] smithyEncodeFieldVarUInt(long value) {
        if (value < 0) throw new IllegalArgumentException("field length is negative");
        if (value < 0x80L) return new byte[] { (byte) value };
        if (value < 0x4000L) return new byte[] {
            (byte) (0x80 | (value & 0x3f)), (byte) (value >>> 6)
        };
        if (value < 0x200000L) return new byte[] {
            (byte) (0xc0 | (value & 0x1f)), (byte) (value >>> 5),
            (byte) (value >>> 13)
        };
        if (value < 0x10000000L) return new byte[] {
            (byte) (0xe0 | (value & 0x0f)), (byte) (value >>> 4),
            (byte) (value >>> 12), (byte) (value >>> 20)
        };
        int width = 8;
        while (width > 1 && (value >>> ((width - 1) * 8)) == 0) width--;
        byte[] result = new byte[width + 1];
        result[0] = (byte) (0xf0 | (width - 1));
        for (int index = 0; index < width; index++) {
            result[index + 1] = (byte) (value >>> (index * 8));
        }
        return result;
    }

    private static long smithyDecodeFieldVarUInt(byte[] payload, int[] cursor, String operation) {
        int start = cursor[0];
        if (start >= payload.length) throw new OpenKacheClientException(operation + " field length is truncated");
        int first = payload[start] & 0xff;
        int width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3
            : first < 0xf0 ? 4 : (first & 0x0f) + 2;
        if (width > 9 || start + width > payload.length) {
            throw new OpenKacheClientException(operation + " field length is truncated");
        }
        long value;
        if (width == 1) value = first;
        else if (width == 2) value = (first & 0x3fL) | ((payload[start + 1] & 0xffL) << 6);
        else if (width == 3) value = (first & 0x1fL)
            | ((payload[start + 1] & 0xffL) << 5)
            | ((payload[start + 2] & 0xffL) << 13);
        else if (width == 4) value = (first & 0x0fL)
            | ((payload[start + 1] & 0xffL) << 4)
            | ((payload[start + 2] & 0xffL) << 12)
            | ((payload[start + 3] & 0xffL) << 20);
        else {
            value = 0;
            for (int index = 1; index < width; index++) {
                value |= (payload[start + index] & 0xffL) << ((index - 1) * 8);
            }
        }
        if (value < 0 || smithyEncodeFieldVarUInt(value).length != width) {
            throw new OpenKacheClientException(operation + " field length is non-canonical");
        }
        cursor[0] = start + width;
        return value;
    }

    private static byte[] smithyEncodeFieldSequence(byte[]... values) {
        int maskBytes = (values.length + 7) / 8;
        int total = maskBytes;
        int lastPresent = -1;
        for (int index = values.length - 1; index >= 0; index--) {
            if (values[index] != null) {
                lastPresent = index;
                break;
            }
        }
        for (int index = 0; index < values.length; index++) {
            byte[] value = values[index];
            if (value != null && value.length > ${framing.max_value_bytes}) {
                throw new OpenKacheClientException("field-sequence entry exceeds the maximum value size");
            }
            if (value != null) {
                if (index != lastPresent) {
                    total = Math.addExact(total, smithyEncodeFieldVarUInt(value.length).length);
                }
                total = Math.addExact(total, value.length);
            }
        }
        if (total > ${framing.max_value_bytes}) {
            throw new OpenKacheClientException("field-sequence payload exceeds the maximum value size");
        }
        byte[] payload = new byte[total];
        int offset = maskBytes;
        for (int index = 0; index < values.length; index++) {
            byte[] value = values[index];
            if (value != null) {
                payload[index / 8] |= (byte) (1 << (index % 8));
                if (index != lastPresent) {
                    byte[] encodedLength = smithyEncodeFieldVarUInt(value.length);
                    System.arraycopy(encodedLength, 0, payload, offset, encodedLength.length);
                    offset += encodedLength.length;
                }
                System.arraycopy(value, 0, payload, offset, value.length);
                offset += value.length;
            }
        }
        return payload;
    }

    private static byte[][] smithyDecodeFieldSequence(
        byte[] payload,
        int fieldCount,
        String operation) {
        byte[][] values = new byte[fieldCount][];
        int maskBytes = (fieldCount + 7) / 8;
        if (payload.length < maskBytes) {
            throw new OpenKacheClientException(operation + " field sequence is missing its presence mask");
        }
        if (maskBytes > 0 && fieldCount % 8 != 0 &&
            (payload[maskBytes - 1] & 0xff & ~((1 << (fieldCount % 8)) - 1)) != 0) {
            throw new OpenKacheClientException(operation + " field sequence presence mask has unused bits set");
        }
        int lastPresent = -1;
        for (int index = fieldCount - 1; index >= 0; index--) {
            if ((payload[index / 8] & (1 << (index % 8))) != 0) {
                lastPresent = index;
                break;
            }
        }
        int[] cursor = { maskBytes };
        for (int index = 0; index < fieldCount; index++) {
            if ((payload[index / 8] & (1 << (index % 8))) == 0) continue;
            long length = index == lastPresent
                ? payload.length - cursor[0]
                : smithyDecodeFieldVarUInt(payload, cursor, operation);
            if (length > ${framing.max_value_bytes}L || length > Integer.MAX_VALUE) {
                throw new OpenKacheClientException(operation + " field sequence entry exceeds the maximum value size");
            }
            int end = Math.addExact(cursor[0], (int) length);
            if (end > payload.length) {
                throw new OpenKacheClientException(operation + " field sequence entry is truncated");
            }
            values[index] = Arrays.copyOfRange(payload, cursor[0], end);
            cursor[0] = end;
        }
        if (cursor[0] != payload.length) {
            throw new OpenKacheClientException(operation + " field sequence contains trailing bytes");
        }
        return values;
    }

    private static byte[] smithyEncodeDenseFields(byte[]... values) {
        int total = 0;
        for (byte[] value : values) {
            Objects.requireNonNull(value, "dense field");
            total = Math.addExact(total, value.length);
        }
        if (total > ${framing.max_value_bytes}) {
            throw new OpenKacheClientException("dense field payload exceeds the maximum value size");
        }
        byte[] payload = new byte[total];
        int offset = 0;
        for (byte[] value : values) {
            System.arraycopy(value, 0, payload, offset, value.length);
            offset += value.length;
        }
        return payload;
    }

    private static byte[][] smithyDecodeDenseFields(
        byte[] payload,
        int[] widths,
        String operation) {
        byte[][] values = new byte[widths.length][];
        int offset = 0;
        for (int index = 0; index < widths.length; index++) {
            int width = widths[index];
            if (width < 0 || width > payload.length - offset) {
                throw new OpenKacheClientException(operation + " dense field payload is truncated");
            }
            values[index] = Arrays.copyOfRange(payload, offset, offset + width);
            offset += width;
        }
        if (offset != payload.length) {
            throw new OpenKacheClientException(operation + " dense field payload has trailing bytes");
        }
        return values;
    }

    private static byte[] smithyEncodeU64(long value) {
        return ByteBuffer.allocate(Long.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putLong(value).array();
    }

    private static long smithyDecodeU64(byte[] payload, String operation) {
        if (payload.length != Long.BYTES) {
            throw new OpenKacheClientException(operation + " response has an invalid u64 field");
        }
        return ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).getLong();
    }

    private static byte[] smithyEncodeBool(boolean value) {
        return new byte[] { (byte) (value ? 1 : 0) };
    }

    private static boolean smithyDecodeBool(byte[] payload, String operation) {
        if (payload.length != 1 || (payload[0] != 0 && payload[0] != 1)) {
            throw new OpenKacheClientException(operation + " response has an invalid boolean field");
        }
        return payload[0] == 1;
    }

    private static byte[] smithyEncodeF64(double value) {
        if (!Double.isFinite(value)) {
            throw new IllegalArgumentException("binary64 field must be finite");
        }
        return ByteBuffer.allocate(Double.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putDouble(value).array();
    }

    private static double smithyDecodeF64(byte[] payload, String operation) {
        if (payload.length != Double.BYTES) {
            throw new OpenKacheClientException(operation + " response has an invalid f64 field");
        }
        double value = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).getDouble();
        if (!Double.isFinite(value)) {
            throw new OpenKacheClientException(operation + " response contains a non-finite f64 field");
        }
        return value;
    }

    private static byte[] smithyEncodeI32(int value) {
        return ByteBuffer.allocate(Integer.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putInt(value).array();
    }

    private static int smithyDecodeI32(byte[] payload, String operation) {
        if (payload.length != Integer.BYTES) {
            throw new OpenKacheClientException(operation + " response has an invalid i32 field");
        }
        return ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).getInt();
    }
`
}

export function render_java_container_helpers(max_value_bytes: number): string {
  return `    private static byte[] smithyEncodeVarUInt(long value) {
        if (value < 0) throw new IllegalArgumentException("container count is negative");
        if (value < 0x80L) return new byte[] { (byte) value };
        if (value < 0x4000L) return new byte[] {
            (byte) (0x80 | (value & 0x3f)), (byte) (value >>> 6)
        };
        if (value < 0x200000L) return new byte[] {
            (byte) (0xc0 | (value & 0x1f)), (byte) (value >>> 5),
            (byte) (value >>> 13)
        };
        if (value < 0x10000000L) return new byte[] {
            (byte) (0xe0 | (value & 0x0f)), (byte) (value >>> 4),
            (byte) (value >>> 12), (byte) (value >>> 20)
        };
        int width = 8;
        while (width > 1 && (value >>> ((width - 1) * 8)) == 0) width--;
        byte[] result = new byte[width + 1];
        result[0] = (byte) (0xf0 | (width - 1));
        for (int index = 0; index < width; index++) {
            result[index + 1] = (byte) (value >>> (index * 8));
        }
        return result;
    }

    private static long smithyDecodeVarUInt(byte[] payload, int[] cursor, String operation) {
        int start = cursor[0];
        if (start >= payload.length) throw new OpenKacheClientException(operation + " container count is truncated");
        int first = payload[start] & 0xff;
        int width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3
            : first < 0xf0 ? 4 : (first & 0x0f) + 2;
        if (width > 9 || start + width > payload.length) {
            throw new OpenKacheClientException(operation + " container count is truncated");
        }
        long value;
        if (width == 1) value = first;
        else if (width == 2) value = (first & 0x3fL) | ((payload[start + 1] & 0xffL) << 6);
        else if (width == 3) value = (first & 0x1fL)
            | ((payload[start + 1] & 0xffL) << 5)
            | ((payload[start + 2] & 0xffL) << 13);
        else if (width == 4) value = (first & 0x0fL)
            | ((payload[start + 1] & 0xffL) << 4)
            | ((payload[start + 2] & 0xffL) << 12)
            | ((payload[start + 3] & 0xffL) << 20);
        else {
            value = 0;
            for (int index = 1; index < width; index++) {
                value |= (payload[start + index] & 0xffL) << ((index - 1) * 8);
            }
        }
        if (smithyEncodeVarUInt(value).length != width) {
            throw new OpenKacheClientException(operation + " container count is non-canonical");
        }
        cursor[0] = start + width;
        return value;
    }

    private static byte[] smithyEncodeLengthDelimited(byte[] value) {
        Objects.requireNonNull(value, "container value");
        if (value.length > ${max_value_bytes}) {
            throw new OpenKacheClientException("container entry exceeds the maximum value size");
        }
        ByteBuffer buffer = ByteBuffer.allocate(Math.addExact(Integer.BYTES, value.length))
            .order(ByteOrder.BIG_ENDIAN);
        buffer.putInt(value.length).put(value);
        return buffer.array();
    }

    private static byte[] smithyReadLengthDelimited(
        byte[] payload,
        int[] cursor,
        String operation
    ) {
        int start = cursor[0];
        if (start > payload.length - Integer.BYTES) {
            throw new OpenKacheClientException(operation + " container entry length is truncated");
        }
        int length = ByteBuffer.wrap(payload, start, Integer.BYTES)
            .order(ByteOrder.BIG_ENDIAN).getInt();
        if (length < 0 || length == 0xffff_ffff || length > ${max_value_bytes}
            || length > payload.length - start - Integer.BYTES) {
            throw new OpenKacheClientException(operation + " container entry is malformed");
        }
        cursor[0] = start + Integer.BYTES + length;
        return java.util.Arrays.copyOfRange(payload, start + Integer.BYTES, cursor[0]);
    }

    private static byte[] smithyJoinContainer(java.util.List<byte[]> chunks) {
        int total = 0;
        for (byte[] chunk : chunks) total = Math.addExact(total, chunk.length);
        byte[] result = new byte[total];
        int offset = 0;
        for (byte[] chunk : chunks) {
            System.arraycopy(chunk, 0, result, offset, chunk.length);
            offset += chunk.length;
        }
        return result;
    }

    private static byte[] smithyEncodeList(byte[][] values) {
        java.util.List<byte[]> chunks = new java.util.ArrayList<>();
        chunks.add(smithyEncodeVarUInt(values.length));
        for (byte[] value : values) chunks.add(smithyEncodeLengthDelimited(value));
        return smithyJoinContainer(chunks);
    }

    private static byte[][] smithyDecodeList(byte[] payload, String operation) {
        int[] cursor = { 0 };
        long count = smithyDecodeVarUInt(payload, cursor, operation);
        if (count > Integer.MAX_VALUE) throw new OpenKacheClientException(operation + " list is too large");
        byte[][] values = new byte[(int) count][];
        for (int index = 0; index < values.length; index++) {
            values[index] = smithyReadLengthDelimited(payload, cursor, operation);
        }
        if (cursor[0] != payload.length) throw new OpenKacheClientException(operation + " list has trailing bytes");
        return values;
    }

    private static byte[] smithyEncodeMap(byte[][][] entries) {
        java.util.List<byte[]> chunks = new java.util.ArrayList<>();
        chunks.add(smithyEncodeVarUInt(entries.length));
        for (byte[][] entry : entries) {
            if (entry.length != 2) throw new IllegalArgumentException("map entry must contain key and value");
            chunks.add(smithyEncodeLengthDelimited(entry[0]));
            chunks.add(smithyEncodeLengthDelimited(entry[1]));
        }
        return smithyJoinContainer(chunks);
    }

    private static java.util.List<byte[][]> smithyDecodeMap(byte[] payload, String operation) {
        int[] cursor = { 0 };
        long count = smithyDecodeVarUInt(payload, cursor, operation);
        if (count > Integer.MAX_VALUE) throw new OpenKacheClientException(operation + " map is too large");
        java.util.List<byte[][]> entries = new java.util.ArrayList<>((int) count);
        for (int index = 0; index < count; index++) {
            entries.add(new byte[][] {
                smithyReadLengthDelimited(payload, cursor, operation),
                smithyReadLengthDelimited(payload, cursor, operation)
            });
        }
        if (cursor[0] != payload.length) throw new OpenKacheClientException(operation + " map has trailing bytes");
        return entries;
    }

    private static byte[] smithyEncodeUnion(byte[] payload, String operation) {
        return smithyDecodeUnion(payload, operation);
    }

    private static byte[] smithyDecodeUnion(byte[] payload, String operation) {
        if (payload.length < 5) throw new OpenKacheClientException(operation + " union payload is truncated");
        int[] cursor = { 1 };
        smithyReadLengthDelimited(payload, cursor, operation);
        if (cursor[0] != payload.length) throw new OpenKacheClientException(operation + " union payload has trailing bytes");
        return payload;
    }
`
}

/** Shared Kotlin helpers for ordered field-sequence framing and scalar codecs. */
