// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Controls shared-core request timeouts and concurrent request lanes.
/// </summary>
public sealed class ClientOptions
{
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
        ArgumentOutOfRangeException.ThrowIfLessThan(MaximumStreamLanes, 1);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(MaximumStreamLanes, 65_535);
        ValidateTimeout(nameof(ConnectTimeout), EffectiveConnectTimeout);
        ValidateTimeout(nameof(RequestTimeout), EffectiveRequestTimeout);
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
