// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Net;
using System.Net.Quic;
using System.Net.Security;
using System.Security.Authentication;
using System.Security.Cryptography;
using System.Security.Cryptography.X509Certificates;
using System.Threading.Channels;

namespace OpenKache;

// Construction is guarded by QuicConnection.IsSupported. The platform analyzer cannot carry
// that runtime fact from ConnectAsync into methods invoked on the successfully created instance.
#pragma warning disable CA1416

internal sealed class QuicTransport : IAsyncDisposable
{
    private readonly QuicConnection _connection;
    private readonly Channel<LaneAvailability> _availableLanes;
    private readonly int _maximumStreamLanes;
    private int _openLanes;
    private int _disposed;

    private QuicTransport(
        QuicConnection connection,
        int maximumStreamLanes)
    {
        _connection = connection;
        _maximumStreamLanes = maximumStreamLanes;
        _availableLanes = Channel.CreateBounded<LaneAvailability>(
            new BoundedChannelOptions(maximumStreamLanes * 2)
            {
                FullMode = BoundedChannelFullMode.Wait,
                SingleReader = false,
                SingleWriter = false,
            });
    }

    internal static async ValueTask<QuicTransport> ConnectAsync(
        string host,
        int port,
        string serverName,
        byte[] trustedCertificateDer,
        int maximumStreamLanes,
        CancellationToken cancellationToken)
    {
        if (!QuicConnection.IsSupported)
        {
            throw new PlatformNotSupportedException(
                "System.Net.Quic is unavailable. Install a supported MsQuic runtime.");
        }

        var applicationProtocol = new SslApplicationProtocol(
            Protocol.ApplicationProtocol);
        EndPoint endpoint = IPAddress.TryParse(host, out var address)
            ? new IPEndPoint(address, port)
            : new DnsEndPoint(host, port);
        var connection = await QuicConnection.ConnectAsync(
            new QuicClientConnectionOptions
            {
                RemoteEndPoint = endpoint,
                DefaultCloseErrorCode = 0,
                DefaultStreamErrorCode = 0,
                ClientAuthenticationOptions = new SslClientAuthenticationOptions
                {
                    TargetHost = serverName,
                    EnabledSslProtocols = SslProtocols.Tls13,
                    ApplicationProtocols = [applicationProtocol],
                    RemoteCertificateValidationCallback =
                        (_, certificate, _, errors) => ValidateCertificate(
                            certificate,
                            errors,
                            trustedCertificateDer),
                },
            },
            cancellationToken).ConfigureAwait(false);

        if (!connection.NegotiatedApplicationProtocol.Equals(applicationProtocol))
        {
            await connection.DisposeAsync().ConfigureAwait(false);
            throw new AuthenticationException(
                $"Server did not negotiate {Protocol.ApplicationProtocol}.");
        }

        return new QuicTransport(connection, maximumStreamLanes);
    }

    internal async ValueTask<Protocol.Response> RequestAsync(
        ReadOnlyMemory<byte> frame,
        CancellationToken cancellationToken)
    {
        var stream = await AcquireLaneAsync(cancellationToken).ConfigureAwait(false);
        var reusable = false;
        try
        {
            await stream.WriteAsync(frame, cancellationToken).ConfigureAwait(false);
            var firstHeaderByte = new byte[1];
            await stream.ReadExactlyAsync(firstHeaderByte, cancellationToken)
                .ConfigureAwait(false);
            var payloadLengthBytes = await ReadVarUIntAsync(stream, cancellationToken)
                .ConfigureAwait(false);
            var headerBytes = new byte[1 + payloadLengthBytes.Length];
            firstHeaderByte.CopyTo(headerBytes, 0);
            payloadLengthBytes.CopyTo(headerBytes, 1);
            var header = Protocol.DecodeResponseHeader(headerBytes);
            var payload = GC.AllocateUninitializedArray<byte>(header.PayloadLength);
            if (payload.Length > 0)
            {
                await stream.ReadExactlyAsync(
                    payload,
                    cancellationToken).ConfigureAwait(false);
            }

            reusable = !Protocol.IsError(header.Status);
            return new Protocol.Response(header.Status, payload);
        }
        finally
        {
            if (reusable)
            {
                await ReleaseLaneAsync(stream).ConfigureAwait(false);
            }
            else
            {
                await DiscardLaneAsync(stream).ConfigureAwait(false);
            }
        }
    }

    public async ValueTask DisposeAsync()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        _availableLanes.Writer.TryComplete();
        while (_availableLanes.Reader.TryRead(out var available))
        {
            if (available.Stream is not null)
            {
                await available.Stream.DisposeAsync().ConfigureAwait(false);
                RemoveLane();
            }
        }

        try
        {
            await _connection.CloseAsync(
                0,
                CancellationToken.None).ConfigureAwait(false);
        }
        catch (QuicException)
        {
            // The peer may have already closed the connection.
        }
        finally
        {
            await _connection.DisposeAsync().ConfigureAwait(false);
        }
    }

    private static bool ValidateCertificate(
        X509Certificate? certificate,
        SslPolicyErrors errors,
        byte[] trustedCertificateDer)
    {
        if (certificate is null
            || (errors & ~SslPolicyErrors.RemoteCertificateChainErrors)
                != SslPolicyErrors.None)
        {
            return false;
        }

        return CryptographicOperations.FixedTimeEquals(
            certificate.GetRawCertData(),
            trustedCertificateDer);
    }

    private static async ValueTask<byte[]> ReadVarUIntAsync(
        QuicStream stream,
        CancellationToken cancellationToken)
    {
        var first = new byte[1];
        await stream.ReadExactlyAsync(first, cancellationToken).ConfigureAwait(false);
        var length = Protocol.EncodedVarUIntLength(first[0]);
        var encoded = new byte[length];
        encoded[0] = first[0];
        if (length > 1)
        {
            await stream.ReadExactlyAsync(
                encoded.AsMemory(1),
                cancellationToken).ConfigureAwait(false);
        }

        return encoded;
    }

    private async ValueTask<QuicStream> AcquireLaneAsync(
        CancellationToken cancellationToken)
    {
        while (true)
        {
            ObjectDisposedException.ThrowIf(
                Volatile.Read(ref _disposed) != 0,
                this);
            if (_availableLanes.Reader.TryRead(out var available)
                && available.Stream is not null)
            {
                return available.Stream;
            }

            if (TryReserveLane())
            {
                try
                {
                    return await _connection.OpenOutboundStreamAsync(
                        QuicStreamType.Bidirectional,
                        cancellationToken).ConfigureAwait(false);
                }
                catch
                {
                    RemoveLane();
                    SignalCapacity();
                    throw;
                }
            }

            try
            {
                available = await _availableLanes.Reader.ReadAsync(
                    cancellationToken).ConfigureAwait(false);
                if (available.Stream is not null)
                {
                    return available.Stream;
                }
            }
            catch (ChannelClosedException error)
            {
                throw new ObjectDisposedException(
                    nameof(QuicTransport),
                    error);
            }
        }
    }

    private ValueTask ReleaseLaneAsync(QuicStream stream)
    {
        if (Volatile.Read(ref _disposed) == 0
            && _availableLanes.Writer.TryWrite(new LaneAvailability(stream)))
        {
            return ValueTask.CompletedTask;
        }

        return DiscardLaneAsync(stream);
    }

    private async ValueTask DiscardLaneAsync(QuicStream stream)
    {
        await stream.DisposeAsync().ConfigureAwait(false);
        RemoveLane();
        SignalCapacity();
    }

    private bool TryReserveLane()
    {
        while (true)
        {
            var open = Volatile.Read(ref _openLanes);
            if (open >= _maximumStreamLanes)
            {
                return false;
            }

            if (Interlocked.CompareExchange(
                    ref _openLanes,
                    open + 1,
                    open) == open)
            {
                return true;
            }
        }
    }

    private void RemoveLane()
    {
        Interlocked.Decrement(ref _openLanes);
    }

    private void SignalCapacity()
    {
        if (Volatile.Read(ref _disposed) == 0)
        {
            _availableLanes.Writer.TryWrite(default);
        }
    }

    private readonly record struct LaneAvailability(QuicStream? Stream);
}
