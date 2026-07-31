// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

internal static partial class Protocol
{
    internal readonly record struct Response(
        Status Status,
        byte[] Payload);

    internal readonly record struct ResponseHeader(
        Status Status,
        int PayloadLength);

    internal static byte[] EncodeRequest(
        Opcode opcode,
        ReadOnlySpan<byte> itemId,
        ReadOnlySpan<byte> value,
        SetCondition setCondition = SetCondition.None,
        ulong? ttlMilliseconds = null)
    {
        if (value.Length > MaximumValueBytes)
        {
            throw new OpenKacheException(
                "VALUE_TOO_LARGE",
                $"Value size {value.Length} exceeds {MaximumValueBytes} bytes.");
        }

        var usesItemId = opcode is Opcode.Get or Opcode.Set or Opcode.Delete;
        var acceptsValue = opcode is Opcode.Set;
        if (!acceptsValue && !value.IsEmpty)
        {
            throw ProtocolError($"{opcode} does not accept a value.");
        }

        if (!usesItemId && !itemId.IsEmpty)
        {
            throw ProtocolError($"{opcode} does not accept an item ID.");
        }

        if (usesItemId && itemId.Length != ItemIdBytes)
        {
            throw ProtocolError(
                $"{opcode} item ID must contain exactly {ItemIdBytes} bytes.");
        }

        if (opcode is not Opcode.Set
            && (setCondition is not SetCondition.None || ttlMilliseconds.HasValue))
        {
            throw ProtocolError($"{opcode} does not accept set options.");
        }

        if (ttlMilliseconds is 0)
        {
            throw ProtocolError("SET TTL must be greater than zero milliseconds.");
        }

        var flags = setCondition switch
        {
            SetCondition.None => 0u,
            SetCondition.IfAbsent => SetIfAbsentBit,
            SetCondition.IfPresent => SetIfPresentBit,
            _ => throw ProtocolError($"Unknown set condition {setCondition}."),
        };
        if (ttlMilliseconds.HasValue)
        {
            flags |= SetTtlBit;
        }

        var itemIdLength = usesItemId ? ItemIdBytes : 0;
        var itemIdLengthBytes = EncodeVarUInt((ulong)itemIdLength);
        var valueLengthBytes = EncodeVarUInt((ulong)value.Length);
        var ttlBytes = ttlMilliseconds.HasValue
            ? EncodeVarUInt(ttlMilliseconds.Value)
            : [];
        var requestHeaderBytes =
            2 + itemIdLengthBytes.Length + valueLengthBytes.Length;
        var frame = GC.AllocateUninitializedArray<byte>(
            checked(requestHeaderBytes + itemIdLength + ttlBytes.Length + value.Length));
        frame[0] = (byte)opcode;
        frame[1] = (byte)flags;
        var offset = 2;
        itemIdLengthBytes.CopyTo(frame.AsSpan(offset));
        offset += itemIdLengthBytes.Length;
        valueLengthBytes.CopyTo(frame.AsSpan(offset));
        offset += valueLengthBytes.Length;
        if (usesItemId)
        {
            itemId.CopyTo(frame.AsSpan(offset, ItemIdBytes));
            offset += ItemIdBytes;
        }

        if (ttlBytes.Length > 0)
        {
            ttlBytes.CopyTo(frame.AsSpan(offset));
            offset += ttlBytes.Length;
        }

        value.CopyTo(frame.AsSpan(offset));
        return frame;
    }

    internal static ResponseHeader DecodeResponseHeader(ReadOnlySpan<byte> header)
    {
        if (header.Length < 2)
        {
            throw ProtocolError("Response header is truncated.");
        }

        var status = DecodeStatus(header[0]);
        var payloadLength = DecodeVarUInt(header[1..]);
        if (payloadLength > (ulong)MaximumValueBytes)
        {
            throw ProtocolError(
                $"Response payload exceeds {MaximumValueBytes} bytes.");
        }

        return new ResponseHeader(status, checked((int)payloadLength));
    }

    internal static int EncodedVarUIntLength(byte first)
    {
        if (first <= 0x7f)
        {
            return 1;
        }

        if (first <= 0xbf)
        {
            return 2;
        }

        if (first <= 0xdf)
        {
            return 3;
        }

        if (first <= 0xef)
        {
            return 4;
        }

        if (first is >= 0xf3 and <= 0xf7)
        {
            return first - 0xf3 + 5;
        }

        throw ProtocolError($"Invalid variable integer prefix 0x{first:x2}.");
    }

    internal static bool IsError(Status status)
    {
        return (byte)status >= (byte)Status.InvalidRequest;
    }

    internal static string ErrorCode(Status status)
    {
        return status switch
        {
            Status.InvalidRequest => "INVALID_REQUEST",
            Status.UnsupportedOpcode => "UNSUPPORTED_OPCODE",
            Status.TooLarge => "TOO_LARGE",
            Status.Overloaded => "OVERLOADED",
            Status.Timeout => "TIMEOUT",
            Status.Forbidden => "FORBIDDEN",
            Status.InternalError => "INTERNAL_ERROR",
            _ => throw ProtocolError($"{status} is not an error status."),
        };
    }

    private static Status DecodeStatus(byte value)
    {
        var status = (Status)value;
        return Enum.IsDefined(status)
            ? status
            : throw ProtocolError($"Unknown response status 0x{value:x2}.");
    }

    private static byte[] EncodeVarUInt(ulong value)
    {
        var bytes = new byte[MaximumVarUIntBytes];
        var length = value switch
        {
            <= 0x7f => 1,
            <= 0x3fff => 2,
            <= 0x1f_ffff => 3,
            <= 0x0fff_ffff => 4,
            <= uint.MaxValue => 5,
            <= 0xff_ffff_ffff => 6,
            <= 0xffff_ffff_ffff => 7,
            <= 0xff_ffff_ffff_ffff => 8,
            _ => 9,
        };

        switch (length)
        {
            case 1:
                bytes[0] = (byte)value;
                break;
            case 2:
                bytes[0] = (byte)(0x80 | (value & 0x3f));
                bytes[1] = (byte)(value >> 6);
                break;
            case 3:
                bytes[0] = (byte)(0xc0 | (value & 0x1f));
                bytes[1] = (byte)(value >> 5);
                bytes[2] = (byte)(value >> 13);
                break;
            case 4:
                bytes[0] = (byte)(0xe0 | (value & 0x0f));
                bytes[1] = (byte)(value >> 4);
                bytes[2] = (byte)(value >> 12);
                bytes[3] = (byte)(value >> 20);
                break;
            default:
                bytes[0] = (byte)(0xf3 + length - 5);
                for (var index = 1; index < length; index++)
                {
                    bytes[index] = (byte)(value >> (8 * (index - 1)));
                }
                break;
        }

        Array.Resize(ref bytes, length);
        return bytes;
    }

    private static ulong DecodeVarUInt(ReadOnlySpan<byte> encoded)
    {
        var length = EncodedVarUIntLength(encoded[0]);
        if (encoded.Length != length)
        {
            throw ProtocolError(
                $"Variable integer requires {length} bytes, got {encoded.Length}.");
        }

        var value = length switch
        {
            1 => encoded[0],
            2 => ((ulong)encoded[0] & 0x3fUL) | ((ulong)encoded[1] << 6),
            3 => ((ulong)encoded[0] & 0x1fUL)
                | ((ulong)encoded[1] << 5)
                | ((ulong)encoded[2] << 13),
            4 => ((ulong)encoded[0] & 0x0fUL)
                | ((ulong)encoded[1] << 4)
                | ((ulong)encoded[2] << 12)
                | ((ulong)encoded[3] << 20),
            _ => DecodeWideVarUInt(encoded),
        };
        if (!EncodeVarUInt(value).AsSpan().SequenceEqual(encoded))
        {
            throw ProtocolError("Variable integer is not in canonical form.");
        }

        return value;
    }

    private static ulong DecodeWideVarUInt(ReadOnlySpan<byte> encoded)
    {
        var value = 0UL;
        for (var index = 1; index < encoded.Length; index++)
        {
            value |= (ulong)encoded[index] << (8 * (index - 1));
        }

        return value;
    }

    private static OpenKacheException ProtocolError(string message)
    {
        return new OpenKacheException("PROTOCOL_ERROR", message);
    }
}
