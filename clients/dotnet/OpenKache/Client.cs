// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Net.Quic;
using System.Net.Sockets;
using System.Text;

namespace OpenKache;

/// <summary>
/// An asynchronous, thread-safe client for the OpenKache QUIC protocol.
/// </summary>
public sealed class Client : IAsyncDisposable
{
    private static readonly UTF8Encoding StrictUtf8 = new(false, true);
    private static readonly SetOptions DefaultSetOptions = new();

    private readonly QuicTransport _transport;
    private readonly TimeSpan _operationTimeout;
    private int _disposed;

    private Client(QuicTransport transport, TimeSpan operationTimeout)
    {
        _transport = transport;
        _operationTimeout = operationTimeout;
    }

    /// <summary>
    /// Connects to an OpenKache server and authenticates its TLS certificate.
    /// </summary>
    /// <param name="host">Server host or IP address.</param>
    /// <param name="port">Server UDP port.</param>
    /// <param name="serverName">DNS name required by the server certificate.</param>
    /// <param name="trustedCertificateDer">Exact DER certificate trusted for this connection.</param>
    /// <param name="options">Optional stream-pool and timeout settings.</param>
    /// <param name="cancellationToken">Cancels connection establishment.</param>
    /// <returns>A connected client that owns one reusable QUIC connection.</returns>
    /// <exception cref="OpenKacheException">
    /// Thrown when QUIC is unavailable, configuration is invalid, certificate validation fails,
    /// or the connection cannot be established.
    /// </exception>
    public static async ValueTask<Client> ConnectAsync(
        string host,
        int port,
        string serverName,
        ReadOnlyMemory<byte> trustedCertificateDer,
        ClientOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentException.ThrowIfNullOrWhiteSpace(host);
        ArgumentException.ThrowIfNullOrWhiteSpace(serverName);
        ArgumentOutOfRangeException.ThrowIfLessThan(port, 1);
        ArgumentOutOfRangeException.ThrowIfGreaterThan(port, 65_535);
        if (trustedCertificateDer.IsEmpty)
        {
            throw new ArgumentException(
                "A trusted DER certificate is required.",
                nameof(trustedCertificateDer));
        }

        options ??= new ClientOptions();
        options.Validate();

        using var timeout = CreateTimeout(
            cancellationToken,
            options.OperationTimeout);
        try
        {
            var transport = await QuicTransport.ConnectAsync(
                host,
                port,
                serverName,
                trustedCertificateDer.ToArray(),
                options.MaximumStreamLanes,
                timeout.Token).ConfigureAwait(false);
            return new Client(transport, options.OperationTimeout);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException(
                "TIMEOUT",
                $"Connection exceeded {options.OperationTimeout}.");
        }
        catch (PlatformNotSupportedException error)
        {
            throw new OpenKacheException("QUIC_UNAVAILABLE", error.Message, error);
        }
        catch (Exception error) when (
            error is QuicException
                or SocketException
                or System.Security.Authentication.AuthenticationException)
        {
            throw new OpenKacheException("CONNECTION_FAILED", error.Message, error);
        }
    }

    /// <summary>
    /// Verifies that the peer speaks the expected OpenKache protocol.
    /// </summary>
    public async ValueTask PingAsync(CancellationToken cancellationToken = default)
    {
        var response = await RequestAsync(
            Protocol.Opcode.Ping,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken).ConfigureAwait(false);
        ExpectStatus("PING", response.Status, Protocol.Status.Ok);
        if (!response.Payload.AsSpan().SequenceEqual("PONG"u8))
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "PING returned an unexpected payload.");
        }
    }

    /// <summary>
    /// Retrieves the bytes stored for an exact binary key.
    /// </summary>
    /// <returns>The stored bytes, or <see langword="null"/> when the key is absent.</returns>
    public async ValueTask<byte[]?> GetAsync(
        ReadOnlyMemory<byte> key,
        CancellationToken cancellationToken = default)
    {
        var response = await RequestAsync(
            Protocol.Opcode.Get,
            key,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken).ConfigureAwait(false);
        return response.Status switch
        {
            Protocol.Status.Ok => DecodePlaintextValue(response),
            Protocol.Status.NotFound => null,
            _ => throw UnexpectedStatus("GET", response.Status),
        };
    }

    /// <summary>
    /// Stores exact bytes under an exact binary key.
    /// </summary>
    /// <returns>Whether the operation created or replaced the key.</returns>
    public async ValueTask<SetOutcome> SetAsync(
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        CancellationToken cancellationToken = default)
    {
        return await SetAsync(
            key,
            value,
            DefaultSetOptions,
            cancellationToken).ConfigureAwait(false);
    }

    /// <summary>
    /// Stores exact bytes under an exact binary key with optional expiration and an atomic
    /// existence condition.
    /// </summary>
    /// <returns>
    /// Whether the operation created, replaced, or did not store the key because its condition
    /// failed.
    /// </returns>
    public async ValueTask<SetOutcome> SetAsync(
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        SetOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var ttlMilliseconds = options.ValidateAndGetTtlMilliseconds();
        var response = await RequestAsync(
            Protocol.Opcode.Set,
            key,
            value,
            cancellationToken,
            options.Condition,
            ttlMilliseconds).ConfigureAwait(false);
        return response.Status switch
        {
            Protocol.Status.Created => SetOutcome.Created,
            Protocol.Status.Replaced => SetOutcome.Replaced,
            Protocol.Status.NotStored => SetOutcome.NotStored,
            _ => throw UnexpectedStatus("SET", response.Status),
        };
    }

    /// <summary>
    /// Deletes an exact binary key.
    /// </summary>
    /// <returns><see langword="true"/> when the key existed.</returns>
    public async ValueTask<bool> DeleteAsync(
        ReadOnlyMemory<byte> key,
        CancellationToken cancellationToken = default)
    {
        var response = await RequestAsync(
            Protocol.Opcode.Delete,
            key,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken).ConfigureAwait(false);
        return response.Status switch
        {
            Protocol.Status.Deleted => true,
            Protocol.Status.NotFound => false,
            _ => throw UnexpectedStatus("DELETE", response.Status),
        };
    }

    /// <summary>
    /// Returns the server statistics payload as UTF-8 JSON.
    /// </summary>
    public async ValueTask<string> StatsAsync(
        CancellationToken cancellationToken = default)
    {
        var response = await RequestAsync(
            Protocol.Opcode.Stats,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken).ConfigureAwait(false);
        ExpectStatus("STATS", response.Status, Protocol.Status.Ok);
        try
        {
            return StrictUtf8.GetString(response.Payload);
        }
        catch (DecoderFallbackException error)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "STATS returned invalid UTF-8.",
                error);
        }
    }

    /// <summary>
    /// Requests a durability barrier from the server.
    /// </summary>
    public async ValueTask SyncAsync(CancellationToken cancellationToken = default)
    {
        var response = await RequestAsync(
            Protocol.Opcode.Sync,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken).ConfigureAwait(false);
        ExpectStatus("SYNC", response.Status, Protocol.Status.Ok);
    }

    /// <summary>
    /// Closes the QUIC connection and releases every idle stream lane.
    /// </summary>
    public async ValueTask DisposeAsync()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        await _transport.DisposeAsync().ConfigureAwait(false);
    }

    private static CancellationTokenSource CreateTimeout(
        CancellationToken cancellationToken,
        TimeSpan timeout)
    {
        var source = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
        source.CancelAfter(timeout);
        return source;
    }

    private async ValueTask<Protocol.Response> RequestAsync(
        Protocol.Opcode opcode,
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        CancellationToken cancellationToken,
        SetCondition setCondition = SetCondition.None,
        ulong? ttlMilliseconds = null)
    {
        ObjectDisposedException.ThrowIf(
            Volatile.Read(ref _disposed) != 0,
            this);
        var frame = Protocol.EncodeRequest(
            opcode,
            key.Span,
            value.Span,
            setCondition,
            ttlMilliseconds);
        using var timeout = CreateTimeout(cancellationToken, _operationTimeout);
        try
        {
            var response = await _transport.RequestAsync(
                frame,
                timeout.Token).ConfigureAwait(false);
            if (Protocol.IsError(response.Status))
            {
                throw new OpenKacheException(
                    Protocol.ErrorCode(response.Status),
                    Encoding.UTF8.GetString(response.Payload));
            }

            return response;
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException(
                "TIMEOUT",
                $"{opcode.ToString().ToUpperInvariant()} exceeded {_operationTimeout}.");
        }
        catch (OpenKacheException)
        {
            throw;
        }
        catch (Exception error) when (
            error is QuicException
                or IOException
                or ObjectDisposedException)
        {
            throw new OpenKacheException("CONNECTION_FAILED", error.Message, error);
        }
    }

    private static byte[] DecodePlaintextValue(Protocol.Response response)
    {
        if (response.ValueFlags != Protocol.ValueFlags.None)
        {
            throw new OpenKacheException(
                "UNSUPPORTED_VALUE_ENCODING",
                "The value is compressed or encrypted. This .NET client currently accepts plaintext values only.");
        }

        return response.Payload;
    }

    private static void ExpectStatus(
        string operation,
        Protocol.Status actual,
        Protocol.Status expected)
    {
        if (actual != expected)
        {
            throw UnexpectedStatus(operation, actual);
        }
    }

    private static OpenKacheException UnexpectedStatus(
        string operation,
        Protocol.Status status)
    {
        return new OpenKacheException(
            "PROTOCOL_ERROR",
            $"{operation} returned unexpected status {status}.");
    }
}
