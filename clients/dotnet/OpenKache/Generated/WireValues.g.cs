// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

// Generated from the OpenKache Smithy contract. Do not edit.

namespace OpenKache;

internal static partial class Protocol
{
    internal const string ApplicationProtocol = "openkache/2";
    internal const int ResponseHeaderBytes = 5;
    internal const int MaximumValueBytes = 67_108_864;

    private const int RequestHeaderBytes = 9;
    private const int ItemKeyBytes = 32;
    private const int SetTtlBytes = 8;
    private const uint ResponseValueLengthMask = 1_073_741_823u;
    private const uint ValueCompressedBit = 2_147_483_648u;
    private const uint ValueEncryptedBit = 1_073_741_824u;
    private const uint SetTtlBit = 536_870_912u;
    private const uint SetIfAbsentBit = 268_435_456u;
    private const uint SetIfPresentBit = 134_217_728u;

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
}
