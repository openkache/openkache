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
    public SetCondition Condition { get; init; }

    /// <summary>
    /// Relative lifetime of the stored value. A missing value stores it without expiration.
    /// </summary>
    public TimeSpan? TimeToLive { get; init; }

    internal ulong? ValidateAndGetTtlMilliseconds()
    {
        if (Condition is not SetCondition.None
            and not SetCondition.Nx
            and not SetCondition.Xx)
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
}
