// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls expiration and atomic existence checks for one set operation.
/// </summary>
public sealed class SetOptions
{
    /// <summary>
    /// Atomic existence condition applied by the server.
    /// </summary>
    public Smithy.SetCondition? Condition { get; init; }

    /// <summary>
    /// Item expiration selection. The default inherits the namespace policy.
    /// </summary>
    public Smithy.ExpirationMode? ExpirationMode { get; init; }

    /// <summary>
    /// Item capacity-eviction selection. The default inherits the namespace policy.
    /// </summary>
    public Smithy.EvictionMode? EvictionMode { get; init; }

    /// <summary>
    /// Relative lifetime of the stored value. A missing value inherits the namespace expiration
    /// policy.
    /// </summary>
    public TimeSpan? TimeToLive { get; init; }

    internal ulong? ValidateAndGetTtlMilliseconds()
    {
        if (Condition is not null
            and not Smithy.SetCondition.Any
            and not Smithy.SetCondition.IfAbsent
            and not Smithy.SetCondition.IfPresent)
        {
            throw new ArgumentOutOfRangeException(
                nameof(Condition),
                "The set condition is not supported.");
        }

        if (TimeToLive is not { } timeToLive)
        {
            return null;
        }

        if (timeToLive <= TimeSpan.Zero)
        {
            throw new ArgumentOutOfRangeException(
                nameof(TimeToLive),
                "The time to live must be positive.");
        }

        var milliseconds = timeToLive.Ticks / TimeSpan.TicksPerMillisecond;
        if (timeToLive.Ticks % TimeSpan.TicksPerMillisecond != 0)
        {
            milliseconds += 1;
        }

        return checked((ulong)milliseconds);
    }

    internal (byte Flags, ulong TtlMilliseconds) ValidateAndGetWireOptions()
    {
        if (Condition is not null
            and not Smithy.SetCondition.Any
            and not Smithy.SetCondition.IfAbsent
            and not Smithy.SetCondition.IfPresent)
        {
            throw new ArgumentOutOfRangeException(nameof(Condition));
        }
        var flags = Condition switch
        {
            null or Smithy.SetCondition.Any => Protocol.SetConditionAnyBits,
            Smithy.SetCondition.IfAbsent => Protocol.SetIfAbsentBits,
            Smithy.SetCondition.IfPresent => Protocol.SetIfPresentBits,
            _ => throw new ArgumentOutOfRangeException(nameof(Condition)),
        };
        var ttl = ValidateAndGetTtlMilliseconds();
        flags |= ExpirationMode switch
        {
            null when ttl is > 0 => Protocol.SetExplicitTtlBits,
            null or Smithy.ExpirationMode.Inherit when ttl is null =>
                Protocol.SetInheritExpirationBits,
            Smithy.ExpirationMode.NoExpiry when ttl is null =>
                Protocol.SetNoExpiryBits,
            Smithy.ExpirationMode.ExplicitTtl when ttl is > 0 =>
                Protocol.SetExplicitTtlBits,
            null or Smithy.ExpirationMode.Inherit or Smithy.ExpirationMode.NoExpiry =>
                throw new ArgumentException(
                    "TimeToLive is only valid with ExplicitTtl expiration mode.",
                    nameof(TimeToLive)),
            Smithy.ExpirationMode.ExplicitTtl =>
                throw new ArgumentException(
                    "TimeToLive must be positive with ExplicitTtl expiration mode.",
                    nameof(TimeToLive)),
            _ => throw new ArgumentOutOfRangeException(nameof(ExpirationMode)),
        };
        flags |= EvictionMode switch
        {
            null or Smithy.EvictionMode.Inherit => Protocol.SetInheritEvictionBits,
            Smithy.EvictionMode.Evictable => Protocol.SetEvictableBits,
            Smithy.EvictionMode.EvictionProtected => Protocol.SetEvictionProtectedBits,
            _ => throw new ArgumentOutOfRangeException(nameof(EvictionMode)),
        };
        return (flags, ttl.GetValueOrDefault());
    }
}
