// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls request timeouts and concurrent QUIC stream reuse.
/// </summary>
public sealed class ClientOptions
{
    /// <summary>
    /// Maximum reusable bidirectional stream lanes opened on one connection.
    /// </summary>
    public int MaximumStreamLanes { get; init; } = 256;

    /// <summary>
    /// Maximum duration for connection establishment and each cache operation.
    /// </summary>
    public TimeSpan OperationTimeout { get; init; } = TimeSpan.FromSeconds(10);

    internal void Validate()
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(MaximumStreamLanes, 1);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(MaximumStreamLanes, 65_535);
        if (OperationTimeout <= TimeSpan.Zero
            || OperationTimeout == Timeout.InfiniteTimeSpan)
        {
            throw new ArgumentOutOfRangeException(
                nameof(OperationTimeout),
                "The operation timeout must be finite and positive.");
        }
    }
}
