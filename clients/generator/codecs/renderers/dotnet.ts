/** dotnet field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_csharp_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `    private static byte[] EncodeFieldVarUInt(ulong value)
    {
        if (value < 0x80) return [(byte)value];
        if (value < 0x4000) return [(byte)(0x80 | (value & 0x3f)), (byte)(value >> 6)];
        if (value < 0x200000) return [
            (byte)(0xc0 | (value & 0x1f)), (byte)(value >> 5), (byte)(value >> 13)];
        if (value < 0x10000000) return [
            (byte)(0xe0 | (value & 0x0f)), (byte)(value >> 4),
            (byte)(value >> 12), (byte)(value >> 20)];
        var width = 8;
        while (width > 1 && (value >> ((width - 1) * 8)) == 0) width--;
        var output = new byte[width + 1];
        output[0] = (byte)(0xf0 | (width - 1));
        for (var index = 0; index < width; index++) {
            output[index + 1] = (byte)(value >> (index * 8));
        }
        return output;
    }

    private static ulong DecodeFieldVarUInt(byte[] payload, ref int offset, string operation)
    {
        if (offset >= payload.Length) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field length is truncated.");
        }
        var start = offset;
        var first = payload[start];
        var width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3 :
            first < 0xf0 ? 4 : (first & 0x0f) + 2;
        if (width > 9 || start + width > payload.Length) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field length is truncated.");
        }
        ulong value = width switch
        {
            1 => first,
            2 => ((ulong)first & 0x3f) | ((ulong)payload[start + 1] << 6),
            3 => ((ulong)first & 0x1f) |
                ((ulong)payload[start + 1] << 5) |
                ((ulong)payload[start + 2] << 13),
            4 => ((ulong)first & 0x0f) |
                ((ulong)payload[start + 1] << 4) |
                ((ulong)payload[start + 2] << 12) |
                ((ulong)payload[start + 3] << 20),
            _ => Enumerable.Range(1, width - 1)
                .Aggregate(0UL, (result, index) =>
                    result | ((ulong)payload[start + index] << ((index - 1) * 8))),
        };
        if (EncodeFieldVarUInt(value).Length != width) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field length is non-canonical.");
        }
        offset = start + width;
        return value;
    }

    private static byte[] EncodeFieldSequence(ReadOnlyMemory<byte>?[] values)
    {
        var maskBytes = (values.Length + 7) / 8;
        var lastPresent = Array.FindLastIndex(values, value => value.HasValue);
        var total = maskBytes;
        for (var index = 0; index < values.Length; index++)
        {
            if (values[index] is not { } value) continue;
            if (value.Length > ${framing.max_value_bytes})
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    "field-sequence entry exceeds the maximum value size.");
            }
            if (index != lastPresent) {
                total = checked(total + EncodeFieldVarUInt((ulong)value.Length).Length);
            }
            total = checked(total + value.Length);
        }
        if (total > ${framing.max_value_bytes})
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "field-sequence payload exceeds the maximum value size.");
        }
        var payload = new byte[total];
        var offset = maskBytes;
        for (var index = 0; index < values.Length; index++)
        {
            if (values[index] is not { } value) continue;
            payload[index / 8] |= (byte)(1 << (index % 8));
            if (index != lastPresent) {
                var encodedLength = EncodeFieldVarUInt((ulong)value.Length);
                encodedLength.CopyTo(payload, offset);
                offset += encodedLength.Length;
            }
            value.Span.CopyTo(payload.AsSpan(offset));
            offset += value.Length;
        }
        return payload;
    }

    private static byte[]?[] DecodeFieldSequence(byte[] payload, int fieldCount, string operation)
    {
        var values = new byte[]?[fieldCount];
        var maskBytes = (fieldCount + 7) / 8;
        if (payload.Length < maskBytes) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field sequence is missing its presence mask.");
        }
        if (maskBytes > 0 && fieldCount % 8 != 0 &&
            (payload[maskBytes - 1] & ~((1 << (fieldCount % 8)) - 1)) != 0) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field sequence presence mask has unused bits set.");
        }
        var lastPresent = -1;
        for (var index = fieldCount - 1; index >= 0; index--) {
            if ((payload[index / 8] & (1 << (index % 8))) != 0) {
                lastPresent = index;
                break;
            }
        }
        var offset = maskBytes;
        for (var index = 0; index < fieldCount; index++)
        {
            if ((payload[index / 8] & (1 << (index % 8))) == 0) continue;
            var length = index == lastPresent
                ? (ulong)(payload.Length - offset)
                : DecodeFieldVarUInt(payload, ref offset, operation);
            if (length > ${framing.max_value_bytes}UL || length > int.MaxValue) {
                throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field sequence entry exceeds the maximum value size.");
            }
            var end = checked(offset + (int)length);
            if (end > payload.Length) {
                throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field sequence entry is truncated.");
            }
            values[index] = payload[offset..end];
            offset = end;
        }
        if (offset != payload.Length) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} field sequence contains trailing bytes.");
        }
        return values;
    }

    private static byte[] EncodeDenseFields(ReadOnlyMemory<byte>[] values)
    {
        var total = values.Sum(value => value.Length);
        if (total > ${framing.max_value_bytes})
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "dense field payload exceeds the maximum value size.");
        }
        var payload = new byte[total];
        var offset = 0;
        foreach (var value in values)
        {
            value.Span.CopyTo(payload.AsSpan(offset));
            offset += value.Length;
        }
        return payload;
    }

    private static byte[]?[] DecodeDenseFields(
        byte[] payload,
        int[] widths,
        string operation)
    {
        var values = new byte[]?[widths.Length];
        var offset = 0;
        for (var index = 0; index < widths.Length; index++)
        {
            var width = widths[index];
            if (width < 0 || width > payload.Length - offset)
            {
                throw new OpenKacheException(
                    "PROTOCOL_ERROR",
                    $"{operation} dense field payload is truncated.");
            }
            values[index] = payload[offset..(offset + width)];
            offset += width;
        }
        if (offset != payload.Length)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} dense field payload has trailing bytes.");
        }
        return values;
    }

    private static byte[] EncodeU64(ulong value)
    {
        var payload = new byte[sizeof(ulong)];
        BinaryPrimitives.WriteUInt64BigEndian(payload, value);
        return payload;
    }

    private static ulong DecodeU64(byte[] payload, string operation)
    {
        if (payload.Length != sizeof(ulong))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response has an invalid u64 field.");
        }
        return BinaryPrimitives.ReadUInt64BigEndian(payload);
    }

    private static byte[] EncodeBool(bool value) => [value ? (byte)1 : (byte)0];

    private static bool DecodeBool(byte[] payload, string operation)
    {
        if (payload.Length != 1 || (payload[0] != 0 && payload[0] != 1))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response has an invalid boolean field.");
        }
        return payload[0] == 1;
    }

    private static byte[] EncodeF64(double value)
    {
        if (!double.IsFinite(value))
        {
            throw new OpenKacheException("PROTOCOL_ERROR", "binary64 field must be finite.");
        }
        var payload = new byte[sizeof(double)];
        BinaryPrimitives.WriteInt64BigEndian(
            payload,
            BitConverter.DoubleToInt64Bits(value));
        return payload;
    }

    private static double DecodeF64(byte[] payload, string operation)
    {
        if (payload.Length != sizeof(double))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response has an invalid f64 field.");
        }
        var value = BitConverter.Int64BitsToDouble(
            BinaryPrimitives.ReadInt64BigEndian(payload));
        if (!double.IsFinite(value))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response contains a non-finite f64 field.");
        }
        return value;
    }

    private static byte[] EncodeI32(int value)
    {
        var payload = new byte[sizeof(int)];
        BinaryPrimitives.WriteInt32BigEndian(payload, value);
        return payload;
    }

    private static int DecodeI32(byte[] payload, string operation)
    {
        if (payload.Length != sizeof(int))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"{operation} response has an invalid i32 field.");
        }
        return BinaryPrimitives.ReadInt32BigEndian(payload);
    }
`
}

export function render_csharp_container_helpers(max_value_bytes: number): string {
  return `    private static byte[] EncodeVarUInt(ulong value)
    {
        if (value < 0x80) return [(byte)value];
        if (value < 0x4000) return [(byte)(0x80 | (value & 0x3f)), (byte)(value >> 6)];
        if (value < 0x200000) return [(byte)(0xc0 | (value & 0x1f)), (byte)(value >> 5), (byte)(value >> 13)];
        if (value < 0x10000000) return [(byte)(0xe0 | (value & 0x0f)), (byte)(value >> 4), (byte)(value >> 12), (byte)(value >> 20)];
        var width = 8;
        while (width > 1 && (value >> ((width - 1) * 8)) == 0) width--;
        var result = new byte[width + 1];
        result[0] = (byte)(0xf0 | (width - 1));
        for (var index = 0; index < width; index++) result[index + 1] = (byte)(value >> (index * 8));
        return result;
    }

    private static ulong DecodeVarUInt(byte[] payload, ref int offset, string operation)
    {
        if (offset >= payload.Length) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} container count is truncated.");
        var first = payload[offset];
        var width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3 : first < 0xf0 ? 4 : (first & 0x0f) + 2;
        if (width > 9 || offset + width > payload.Length) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} container count is truncated.");
        ulong value;
        if (width == 1) {
            value = first;
        } else if (width == 2) {
            value = ((ulong)first & 0x3f) | ((ulong)payload[offset + 1] << 6);
        } else if (width == 3) {
            value = ((ulong)first & 0x1f)
                | ((ulong)payload[offset + 1] << 5)
                | ((ulong)payload[offset + 2] << 13);
        } else if (width == 4) {
            value = ((ulong)first & 0x0f)
                | ((ulong)payload[offset + 1] << 4)
                | ((ulong)payload[offset + 2] << 12)
                | ((ulong)payload[offset + 3] << 20);
        } else {
            value = 0;
            for (var index = 1; index < width; index++) {
                value |= (ulong)payload[offset + index] << ((index - 1) * 8);
            }
        }
        if (EncodeVarUInt(value).Length != width) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} container count is non-canonical.");
        offset += width;
        return value;
    }

    private static byte[] EncodeLengthDelimited(byte[] value)
    {
        if (value.Length > ${max_value_bytes}) throw new OpenKacheException("PROTOCOL_ERROR", "container entry exceeds the maximum value size.");
        var encodedLength = EncodeVarUInt((ulong)value.Length);
        var result = new byte[checked(encodedLength.Length + value.Length)];
        encodedLength.CopyTo(result, 0);
        value.CopyTo(result, encodedLength.Length);
        return result;
    }

    private static byte[] ReadLengthDelimited(byte[] payload, ref int offset, string operation)
    {
        var length = DecodeVarUInt(payload, ref offset, operation);
        if (length > ${max_value_bytes} || length > (ulong)(payload.Length - offset)) {
            throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} container entry is malformed.");
        }
        var start = offset;
        offset = checked(start + (int)length);
        return payload.AsSpan(start, (int)length).ToArray();
    }

    private static byte[] JoinContainer(IEnumerable<byte[]> chunks)
    {
        var result = new List<byte>();
        foreach (var chunk in chunks) result.AddRange(chunk);
        return result.ToArray();
    }

    private static byte[] EncodeList(byte[][] values) =>
        JoinContainer(new[] { EncodeVarUInt((ulong)values.Length) }.Concat(values.Select(EncodeLengthDelimited)));

    private static byte[][] DecodeList(byte[] payload, string operation)
    {
        var offset = 0;
        var count = DecodeVarUInt(payload, ref offset, operation);
        if (count > int.MaxValue) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} list is too large.");
        var values = new byte[(int)count][];
        for (var index = 0; index < values.Length; index++) values[index] = ReadLengthDelimited(payload, ref offset, operation);
        if (offset != payload.Length) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} list has trailing bytes.");
        return values;
    }

    private static byte[] EncodeMap(byte[][][] values) =>
        JoinContainer(new[] { EncodeVarUInt((ulong)values.Length) }.Concat(values.SelectMany(entry =>
        {
            if (entry.Length != 2) throw new OpenKacheException("PROTOCOL_ERROR", "map entry must contain key and value.");
            return new[] { EncodeLengthDelimited(entry[0]), EncodeLengthDelimited(entry[1]) };
        })));

    private static (byte[] Key, byte[] Value)[] DecodeMap(byte[] payload, string operation)
    {
        var offset = 0;
        var count = DecodeVarUInt(payload, ref offset, operation);
        if (count > int.MaxValue) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} map is too large.");
        var values = new (byte[] Key, byte[] Value)[(int)count];
        for (var index = 0; index < values.Length; index++) {
            values[index] = (ReadLengthDelimited(payload, ref offset, operation), ReadLengthDelimited(payload, ref offset, operation));
        }
        if (offset != payload.Length) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} map has trailing bytes.");
        return values;
    }

    private static byte[] EncodeUnion(byte[] payload, string operation) => DecodeUnion(payload, operation);

    private static byte[] DecodeUnion(byte[] payload, string operation)
    {
        if (payload.Length < 2) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} union payload is truncated.");
        var offset = 1;
        ReadLengthDelimited(payload, ref offset, operation);
        if (offset != payload.Length) throw new OpenKacheException("PROTOCOL_ERROR", $"{operation} union payload has trailing bytes.");
        return payload;
    }
`
}

/** Shared Go helpers for ordered field-sequence framing and scalar codecs. */
