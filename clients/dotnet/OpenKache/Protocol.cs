// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Buffers.Binary;

namespace OpenKache;

internal static class Protocol
{
    internal const string ApplicationProtocol = "openkache/2";
    internal const int ResponseHeaderBytes = 5;
    internal const int MaximumValueBytes = 64 * 1024 * 1024;

    private const int RequestHeaderBytes = 9;
    private const int ItemKeyBytes = 32;
    private const int SetTtlBytes = sizeof(ulong);
    private const uint ResponseValueLengthMask = (1u << 30) - 1;
    private const uint ValueCompressedBit = 1u << 31;
    private const uint ValueEncryptedBit = 1u << 30;
    private const uint SetTtlBit = 1u << 29;
    private const uint SetIfAbsentBit = 1u << 28;
    private const uint SetIfPresentBit = 1u << 27;

    internal enum Opcode : byte
    {
        Ping = 0x01,
        Get = 0x02,
        Set = 0x03,
        Delete = 0x04,
        Stats = 0x05,
        Sync = 0x06,
    }

    internal enum Status : byte
    {
        Ok = 0x00,
        NotFound = 0x01,
        Created = 0x02,
        Replaced = 0x03,
        Deleted = 0x04,
        NotStored = 0x05,
        InvalidRequest = 0x40,
        UnsupportedOpcode = 0x41,
        TooLarge = 0x42,
        Overloaded = 0x43,
        Timeout = 0x44,
        Forbidden = 0x45,
        InternalError = 0x7f,
    }

    [Flags]
    internal enum ValueFlags : byte
    {
        None = 0,
        Compressed = 1,
        Encrypted = 2,
    }

    internal readonly record struct Response(
        Status Status,
        ValueFlags ValueFlags,
        byte[] Payload);

    internal readonly record struct ResponseHeader(
        Status Status,
        ValueFlags ValueFlags,
        int PayloadLength);

    internal static byte[] EncodeRequest(
        Opcode opcode,
        ReadOnlySpan<byte> itemKey,
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

        var usesKey = opcode is Opcode.Get or Opcode.Set or Opcode.Delete;
        var acceptsValue = opcode is Opcode.Set;
        if (!acceptsValue && !value.IsEmpty)
        {
            throw ProtocolError($"{opcode} does not accept a value.");
        }

        if (!usesKey && !itemKey.IsEmpty)
        {
            throw ProtocolError($"{opcode} does not accept a key.");
        }

        if (usesKey && itemKey.Length != ItemKeyBytes)
        {
            throw ProtocolError(
                $"{opcode} key must contain exactly {ItemKeyBytes} bytes.");
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

        var optionBits = setCondition switch
        {
            SetCondition.None => 0u,
            SetCondition.IfAbsent => SetIfAbsentBit,
            SetCondition.IfPresent => SetIfPresentBit,
            _ => throw ProtocolError($"Unknown set condition {setCondition}."),
        };
        if (ttlMilliseconds.HasValue)
        {
            optionBits |= SetTtlBit;
        }

        var keyLength = usesKey ? ItemKeyBytes : 0;
        var ttlLength = ttlMilliseconds.HasValue ? SetTtlBytes : 0;
        var frame = GC.AllocateUninitializedArray<byte>(
            checked(RequestHeaderBytes + keyLength + ttlLength + value.Length));
        frame[0] = (byte)opcode;
        BinaryPrimitives.WriteUInt32BigEndian(
            frame.AsSpan(1, sizeof(uint)),
            (uint)keyLength);
        BinaryPrimitives.WriteUInt32BigEndian(
            frame.AsSpan(5, sizeof(uint)),
            (uint)value.Length | optionBits);
        if (usesKey)
        {
            itemKey.CopyTo(frame.AsSpan(RequestHeaderBytes, ItemKeyBytes));
        }

        var valueOffset = RequestHeaderBytes + keyLength;
        if (ttlMilliseconds is { } ttl)
        {
            BinaryPrimitives.WriteUInt64BigEndian(
                frame.AsSpan(valueOffset, SetTtlBytes),
                ttl);
            valueOffset += SetTtlBytes;
        }

        value.CopyTo(frame.AsSpan(valueOffset));
        return frame;
    }

    internal static ResponseHeader DecodeResponseHeader(ReadOnlySpan<byte> header)
    {
        if (header.Length != ResponseHeaderBytes)
        {
            throw ProtocolError(
                $"Response header must contain {ResponseHeaderBytes} bytes.");
        }

        var status = DecodeStatus(header[0]);
        var encodedLength = BinaryPrimitives.ReadUInt32BigEndian(header[1..]);
        var payloadLength = checked((int)(encodedLength & ResponseValueLengthMask));
        if (payloadLength > MaximumValueBytes)
        {
            throw ProtocolError(
                $"Response payload exceeds {MaximumValueBytes} bytes.");
        }

        var flags = ValueFlags.None;
        if ((encodedLength & ValueCompressedBit) != 0)
        {
            flags |= ValueFlags.Compressed;
        }

        if ((encodedLength & ValueEncryptedBit) != 0)
        {
            flags |= ValueFlags.Encrypted;
        }

        if (flags != ValueFlags.None
            && (status != Status.Ok || payloadLength == 0))
        {
            throw ProtocolError(
                "Value transformation flags require a non-empty OK response.");
        }

        return new ResponseHeader(status, flags, payloadLength);
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
        return value switch
        {
            0x00 => Status.Ok,
            0x01 => Status.NotFound,
            0x02 => Status.Created,
            0x03 => Status.Replaced,
            0x04 => Status.Deleted,
            0x05 => Status.NotStored,
            0x40 => Status.InvalidRequest,
            0x41 => Status.UnsupportedOpcode,
            0x42 => Status.TooLarge,
            0x43 => Status.Overloaded,
            0x44 => Status.Timeout,
            0x45 => Status.Forbidden,
            0x7f => Status.InternalError,
            _ => throw ProtocolError($"Unknown response status 0x{value:x2}."),
        };
    }

    private static OpenKacheException ProtocolError(string message)
    {
        return new OpenKacheException("PROTOCOL_ERROR", message);
    }
}
