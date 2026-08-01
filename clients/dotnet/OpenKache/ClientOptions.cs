// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls shared-core request timeouts and concurrent request lanes.
/// </summary>
public sealed class ClientOptions
{
    /// <summary>
    /// Active 32-byte data-protection key. Use <see cref="KeyRing"/> when rotating keys.
    /// </summary>
    public byte[] DataProtectionKey { get; init; } = new byte[Protocol.ValueFormatDataProtectionKeyBytes];

    /// <summary>
    /// Active key and a bounded retired-key window used for read/delete rotation.
    /// </summary>
    public DataProtectionKeyRing? KeyRing { get; init; }

    /// <summary>
    /// Maximum reusable bidirectional stream lanes opened on one connection.
    /// </summary>
    public int MaximumStreamLanes { get; init; } = Protocol.DefaultMaxInFlight;

    /// <summary>
    /// Maximum duration for connection establishment.
    /// </summary>
    public TimeSpan ConnectTimeout { get; init; } =
        TimeSpan.FromMilliseconds(Protocol.DefaultConnectTimeoutMilliseconds);

    /// <summary>
    /// Maximum duration for one complete cache operation.
    /// </summary>
    public TimeSpan RequestTimeout { get; init; } =
        TimeSpan.FromMilliseconds(Protocol.DefaultRequestTimeoutMilliseconds);

    /// <summary>
    /// Legacy alias that applies one timeout to both connection setup and requests.
    /// </summary>
    public TimeSpan? OperationTimeout { get; init; }

    internal TimeSpan EffectiveConnectTimeout =>
        OperationTimeout ?? ConnectTimeout;

    internal TimeSpan EffectiveRequestTimeout =>
        OperationTimeout ?? RequestTimeout;

    internal void Validate()
    {
        if (KeyRing is null)
        {
            ValidateKey(DataProtectionKey, nameof(DataProtectionKey));
        }
        else
        {
            KeyRing.Validate();
        }
        ArgumentOutOfRangeException.ThrowIfLessThan(MaximumStreamLanes, 1);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(MaximumStreamLanes, 65_535);
        ValidateTimeout(nameof(ConnectTimeout), EffectiveConnectTimeout);
        ValidateTimeout(nameof(RequestTimeout), EffectiveRequestTimeout);
    }

    private static void ValidateKey(byte[] key, string name)
    {
        ArgumentNullException.ThrowIfNull(key, name);
        if (key.Length != Protocol.ValueFormatDataProtectionKeyBytes)
        {
            throw new ArgumentException(
                $"The key must contain exactly {Protocol.ValueFormatDataProtectionKeyBytes} bytes.",
                name);
        }
    }

    private static void ValidateTimeout(string name, TimeSpan timeout)
    {
        if (timeout <= TimeSpan.Zero
            || timeout == Timeout.InfiniteTimeSpan)
        {
            throw new ArgumentOutOfRangeException(
                name,
                "The timeout must be finite and positive.");
        }
    }
}

/// <summary>Active data-protection key plus a bounded set of retired keys.</summary>
public sealed class DataProtectionKeyRing
{
    /// <summary>Key used for new writes.</summary>
    public required byte[] Active { get; init; }

    /// <summary>Newest retired key first; the Smithy contract bounds the window.</summary>
    public IReadOnlyList<byte[]> Previous { get; init; } = Array.Empty<byte[]>();

    internal void Validate()
    {
        ValidateKey(Active, nameof(Active));
        if (Previous.Count > Protocol.MaxPreviousDataProtectionKeys)
        {
            throw new ArgumentException(
                $"At most {Protocol.MaxPreviousDataProtectionKeys} previous keys may be retained.",
                nameof(Previous));
        }
        for (var index = 0; index < Previous.Count; index++)
        {
            ValidateKey(Previous[index], $"{nameof(Previous)}[{index}]");
        }
    }

    private static void ValidateKey(byte[] key, string name)
    {
        ArgumentNullException.ThrowIfNull(key, name);
        if (key.Length != Protocol.ValueFormatDataProtectionKeyBytes)
        {
            throw new ArgumentException(
                $"The key must contain exactly {Protocol.ValueFormatDataProtectionKeyBytes} bytes.",
                name);
        }
    }
}
