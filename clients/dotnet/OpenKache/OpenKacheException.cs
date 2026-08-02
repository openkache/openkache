// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

namespace OpenKache;

/// <summary>
/// Reports OpenKache connection, protocol, timeout, and server failures.
/// </summary>
public sealed class OpenKacheException : Exception
{
    /// <summary>
    /// Stable machine-readable error identifier.
    /// </summary>
    public string Code { get; }

    /// <summary>Structured native error metadata, when supplied by the core.</summary>
    public ErrorMetadata? Metadata { get; }

    internal OpenKacheException(
        string code,
        string message,
        Exception? innerException = null,
        ErrorMetadata? metadata = null)
        : base($"[{code}] {message}", innerException)
    {
        Code = code;
        Metadata = metadata;
    }
}

/// <summary>Stable structured metadata attached to native operation failures.</summary>
public sealed record ErrorMetadata(
    uint Code,
    uint Operation,
    uint Phase,
    uint Backend,
    bool Retryable,
    bool Ambiguous,
    byte[]? MutationId);

/// <summary>Point-in-time native request, retry, transport, and lane counters.</summary>
public sealed record MetricsSnapshot(
    ulong Requests,
    ulong Hits,
    ulong Misses,
    ulong Retries,
    ulong Reconnects,
    ulong Cancellations,
    ulong TransportErrors,
    ulong ProtocolErrors,
    ulong BytesSent,
    ulong BytesReceived,
    ulong ActiveLanes);
