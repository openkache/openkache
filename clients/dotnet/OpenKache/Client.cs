// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Text;

namespace OpenKache;

/// <summary>
/// An asynchronous, thread-safe client for the OpenKache protocol.
/// </summary>
public sealed class Client : IAsyncDisposable, Smithy.IOpenKacheApi
{
    private readonly NativeClient _nativeClient;
    private int _disposed;

    private Client(NativeClient nativeClient)
    {
        _nativeClient = nativeClient;
    }

    /// <summary>
    /// Connects to an OpenKache server through the shared Rust client core.
    /// </summary>
    /// <param name="host">Server host or IP address.</param>
    /// <param name="port">Server UDP port.</param>
    /// <param name="serverName">DNS name required by the server certificate.</param>
    /// <param name="trustedCertificateDer">Exact DER certificate trusted for this connection.</param>
    /// <param name="options">Optional shared-core timeout and lane settings.</param>
    /// <param name="cancellationToken">Cancels connection establishment.</param>
    /// <returns>A connected client that owns one shared-core worker.</returns>
    /// <exception cref="OpenKacheException">
    /// Thrown when the native core is unavailable, configuration is invalid, certificate
    /// validation fails, or the connection cannot be established.
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
        try
        {
            var address = host.Contains(':', StringComparison.Ordinal)
                && !host.StartsWith("[", StringComparison.Ordinal)
                ? $"[{host}]:{port}"
                : $"{host}:{port}";
            var nativeClient = await NativeClient.ConnectAsync(
                address,
                serverName,
                trustedCertificateDer,
                options.KeyRing?.Active ?? options.DataProtectionKey,
                options.KeyRing?.Previous
                    .Select(static key => (ReadOnlyMemory<byte>)key)
                    .ToArray()
                    ?? Array.Empty<ReadOnlyMemory<byte>>(),
                options.EffectiveConnectTimeout,
                options.EffectiveRequestTimeout,
                options.MaximumStreamLanes,
                cancellationToken).ConfigureAwait(false);
            return new Client(nativeClient);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException(
                "TIMEOUT",
                $"Connection exceeded {options.EffectiveConnectTimeout}.");
        }
        catch (NativeException error)
        {
            throw MapNativeError(error, "CONNECTION_FAILED");
        }
        catch (DllNotFoundException error)
        {
            throw new OpenKacheException(
                "NATIVE_UNAVAILABLE",
                "The shared OpenKache client core could not be loaded.",
                error);
        }
        catch (EntryPointNotFoundException error)
        {
            throw new OpenKacheException(
                "NATIVE_UNAVAILABLE",
                "The shared OpenKache client core does not expose the required ABI.",
                error);
        }
    }

    /// <summary>
    /// Verifies that the peer speaks the expected OpenKache protocol.
    /// </summary>
    public async ValueTask PingAsync(CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Ping,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("PING", result, Protocol.FfiResultOk);
    }

    /// <summary>
    /// Retrieves the bytes stored for an exact binary item ID.
    /// </summary>
    /// <returns>The stored bytes, or <see langword="null"/> when the item ID is absent.</returns>
    public async ValueTask<byte[]?> GetAsync(
        ReadOnlyMemory<byte> itemId,
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Get,
            ValidateItemId(itemId),
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultValue => result.Payload,
            var kind when kind == Protocol.FfiResultNotFound => null,
            _ => throw UnexpectedKind("GET", result.Kind),
        };
    }

    /// <summary>
    /// Retrieves opaque bytes stored under an application key through the protected value API.
    /// </summary>
    /// <param name="key">Non-empty application key. The core derives the protocol item ID.</param>
    /// <param name="cancellationToken">Stops the pending native operation.</param>
    /// <returns>The stored bytes, or <see langword="null"/> when the key is absent.</returns>
    public async ValueTask<byte[]?> GetRawAsync(
        ReadOnlyMemory<byte> key,
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Get,
            ValidateApplicationKey(key),
            ReadOnlyMemory<byte>.Empty,
            raw: false,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultValue => result.Payload,
            var kind when kind == Protocol.FfiResultNotFound => null,
            _ => throw UnexpectedKind("GET_RAW", result.Kind),
        };
    }

    /// <summary>Retrieves a canonical JSON document through the protected value API.</summary>
    public async ValueTask<string?> GetJsonAsync(
        ReadOnlyMemory<byte> key,
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            Protocol.FfiOperationGetJson,
            ValidateApplicationKey(key),
            ReadOnlyMemory<byte>.Empty,
            raw: false,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        if (result.Kind == Protocol.FfiResultNotFound) return null;
        ExpectKind("GET_JSON", result, Protocol.FfiResultValue);
        try
        {
            return new UTF8Encoding(false, true).GetString(result.Payload);
        }
        catch (DecoderFallbackException error)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "GET_JSON returned invalid UTF-8.",
                error);
        }
    }

    /// <summary>
    /// Stores exact bytes under an exact binary item ID.
    /// </summary>
    /// <returns>Whether the operation created or replaced the item ID.</returns>
    public ValueTask<Smithy.SetOutcome> SetAsync(
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        CancellationToken cancellationToken = default)
    {
        return SetAsync(itemId, value, new SetOptions(), cancellationToken);
    }

    /// <summary>
    /// Stores exact bytes under an exact binary item ID with optional expiration and an atomic
    /// existence condition.
    /// </summary>
    /// <returns>
    /// Whether the operation created, replaced, or did not store the item ID because its condition
    /// failed.
    /// </returns>
    public async ValueTask<Smithy.SetOutcome> SetAsync(
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        SetOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var ttlMilliseconds = options.ValidateAndGetTtlMilliseconds();
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Set,
            ValidateItemId(itemId),
            ValidateValue(value),
            options.Condition,
            ttlMilliseconds,
            options.MutationId ?? CreateMutationId(),
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultCreated => Smithy.SetOutcome.Created,
            var kind when kind == Protocol.FfiResultReplaced => Smithy.SetOutcome.Replaced,
            var kind when kind == Protocol.FfiResultNotStored => Smithy.SetOutcome.NotStored,
            _ => throw UnexpectedKind("SET", result.Kind),
        };
    }

    /// <summary>Stores opaque bytes under an application key through the protected value API.</summary>
    public ValueTask<Smithy.SetOutcome> SetRawAsync(
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        CancellationToken cancellationToken = default)
    {
        return SetRawAsync(key, value, new SetOptions(), cancellationToken);
    }

    /// <summary>
    /// Stores opaque bytes under an application key with optional expiration and condition.
    /// </summary>
    public async ValueTask<Smithy.SetOutcome> SetRawAsync(
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        SetOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        var ttlMilliseconds = options.ValidateAndGetTtlMilliseconds();
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Set,
            ValidateApplicationKey(key),
            ValidateValue(value),
            options.Condition,
            ttlMilliseconds,
            options.MutationId ?? CreateMutationId(),
            raw: false,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultCreated => Smithy.SetOutcome.Created,
            var kind when kind == Protocol.FfiResultReplaced => Smithy.SetOutcome.Replaced,
            var kind when kind == Protocol.FfiResultNotStored => Smithy.SetOutcome.NotStored,
            _ => throw UnexpectedKind("SET_RAW", result.Kind),
        };
    }

    /// <summary>Stores a canonical JSON document under an application key.</summary>
    public ValueTask<Smithy.SetOutcome> SetJsonAsync(
        ReadOnlyMemory<byte> key,
        string json,
        CancellationToken cancellationToken = default)
    {
        return SetJsonAsync(key, json, new SetOptions(), cancellationToken);
    }

    /// <summary>
    /// Stores a canonical JSON document under an application key with optional expiration and
    /// condition. The shared core validates and canonicalizes the JSON representation.
    /// </summary>
    public async ValueTask<Smithy.SetOutcome> SetJsonAsync(
        ReadOnlyMemory<byte> key,
        string json,
        SetOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(json);
        ArgumentNullException.ThrowIfNull(options);
        var ttlMilliseconds = options.ValidateAndGetTtlMilliseconds();
        var result = await RequestAsync(
            Protocol.FfiOperationSetJson,
            ValidateApplicationKey(key),
            new UTF8Encoding(false, true).GetBytes(json),
            options.Condition,
            ttlMilliseconds,
            options.MutationId ?? CreateMutationId(),
            raw: false,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultCreated => Smithy.SetOutcome.Created,
            var kind when kind == Protocol.FfiResultReplaced => Smithy.SetOutcome.Replaced,
            var kind when kind == Protocol.FfiResultNotStored => Smithy.SetOutcome.NotStored,
            _ => throw UnexpectedKind("SET_JSON", result.Kind),
        };
    }

    /// <summary>
    /// Deletes an exact binary item ID.
    /// </summary>
    /// <returns><see langword="true"/> when the item ID existed.</returns>
    public async ValueTask<bool> DeleteAsync(
        ReadOnlyMemory<byte> itemId,
        CancellationToken cancellationToken = default)
    {
        return await DeleteAsync(itemId, new SetOptions(), cancellationToken).ConfigureAwait(false);
    }

    /// <summary>Deletes an item ID while reusing an optional idempotency token.</summary>
    public async ValueTask<bool> DeleteAsync(
        ReadOnlyMemory<byte> itemId,
        SetOptions options,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(options);
        _ = options.ValidateAndGetTtlMilliseconds();
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Delete,
            ValidateItemId(itemId),
            ReadOnlyMemory<byte>.Empty,
            mutationId: options.MutationId ?? CreateMutationId(),
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultDeleted => true,
            var kind when kind == Protocol.FfiResultNotDeleted => false,
            _ => throw UnexpectedKind("DELETE", result.Kind),
        };
    }

    /// <summary>Deletes the value associated with an application key.</summary>
    public async ValueTask<bool> DeleteRawAsync(
        ReadOnlyMemory<byte> key,
        SetOptions? options = null,
        CancellationToken cancellationToken = default)
    {
        options ??= new SetOptions();
        _ = options.ValidateAndGetTtlMilliseconds();
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Delete,
            ValidateApplicationKey(key),
            ReadOnlyMemory<byte>.Empty,
            mutationId: options.MutationId ?? CreateMutationId(),
            raw: false,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultDeleted => true,
            var kind when kind == Protocol.FfiResultNotDeleted => false,
            _ => throw UnexpectedKind("DELETE_RAW", result.Kind),
        };
    }

    /// <summary>
    /// Returns the server statistics payload as UTF-8 JSON.
    /// </summary>
    public async ValueTask<string> StatsAsync(
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Stats,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("STATS", result, Protocol.FfiResultValue);
        try
        {
            return new UTF8Encoding(false, true).GetString(result.Payload);
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
        var result = await RequestAsync(
            (uint)Protocol.Opcode.Sync,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("SYNC", result, Protocol.FfiResultOk);
    }

    /// <summary>Returns a point-in-time native metrics snapshot.</summary>
    public MetricsSnapshot MetricsSnapshot()
    {
        ObjectDisposedException.ThrowIf(Volatile.Read(ref _disposed) != 0, this);
        var snapshot = _nativeClient.Metrics();
        return new MetricsSnapshot(
            snapshot.Requests,
            snapshot.Hits,
            snapshot.Misses,
            snapshot.Retries,
            snapshot.Reconnects,
            snapshot.Cancellations,
            snapshot.TransportErrors,
            snapshot.ProtocolErrors,
            snapshot.BytesSent,
            snapshot.BytesReceived,
            snapshot.ActiveLanes);
    }

    /// <summary>
    /// Invokes the generated Smithy PING operation.
    /// </summary>
    public async ValueTask<Smithy.PingOutput> PingAsync(
        Smithy.PingInput input,
        CancellationToken cancellationToken = default)
    {
        _ = input;
        await PingAsync(cancellationToken).ConfigureAwait(false);
        return new Smithy.PingOutput();
    }

    /// <summary>
    /// Invokes the generated Smithy GET operation.
    /// </summary>
    public async ValueTask<Smithy.GetOutput> GetAsync(
        Smithy.GetInput input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        return new Smithy.GetOutput
        {
            Value = await GetAsync(input.ItemId, cancellationToken).ConfigureAwait(false),
        };
    }

    /// <summary>
    /// Invokes the generated Smithy SET operation.
    /// </summary>
    public async ValueTask<Smithy.SetOutput> SetAsync(
        Smithy.SetInput input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        TimeSpan? timeToLive = input.TtlMilliseconds switch
        {
            null => null,
            <= 0 => throw new ArgumentOutOfRangeException(nameof(input.TtlMilliseconds)),
            var milliseconds => TimeSpan.FromMilliseconds(milliseconds.Value),
        };
        var outcome = await SetAsync(
            input.ItemId,
            input.Value,
            new SetOptions
            {
                Condition = input.Condition,
                TimeToLive = timeToLive,
                MutationId = input.MutationId,
            },
            cancellationToken).ConfigureAwait(false);
        return new Smithy.SetOutput
        {
            Outcome = outcome,
        };
    }

    /// <summary>
    /// Invokes the generated Smithy DELETE operation.
    /// </summary>
    public async ValueTask<Smithy.DeleteOutput> DeleteAsync(
        Smithy.DeleteInput input,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(input);
        return new Smithy.DeleteOutput
        {
            Deleted = await DeleteAsync(
                input.ItemId,
                new SetOptions { MutationId = input.MutationId },
                cancellationToken).ConfigureAwait(false),
        };
    }

    /// <summary>
    /// Invokes the generated Smithy STATS operation.
    /// </summary>
    public async ValueTask<Smithy.StatsOutput> StatsAsync(
        Smithy.StatsInput input,
        CancellationToken cancellationToken = default)
    {
        _ = input;
        return new Smithy.StatsOutput
        {
            Json = await StatsAsync(cancellationToken).ConfigureAwait(false),
        };
    }

    /// <summary>
    /// Invokes the generated Smithy SYNC operation.
    /// </summary>
    public async ValueTask<Smithy.SyncOutput> SyncAsync(
        Smithy.SyncInput input,
        CancellationToken cancellationToken = default)
    {
        _ = input;
        await SyncAsync(cancellationToken).ConfigureAwait(false);
        return new Smithy.SyncOutput();
    }

    /// <summary>
    /// Closes the shared-core worker and releases its native resources.
    /// </summary>
    public async ValueTask DisposeAsync()
    {
        if (Interlocked.Exchange(ref _disposed, 1) != 0)
        {
            return;
        }

        await _nativeClient.DisposeAsync().ConfigureAwait(false);
    }

    private async ValueTask<NativeResult> RequestAsync(
        uint opcode,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        Smithy.SetCondition? condition = null,
        ulong? ttlMilliseconds = null,
        ReadOnlyMemory<byte> mutationId = default,
        bool raw = true,
        CancellationToken cancellationToken = default)
    {
        ObjectDisposedException.ThrowIf(
            Volatile.Read(ref _disposed) != 0,
            this);
        try
        {
            return await _nativeClient.ExecuteAsync(
                (uint)opcode,
                itemId,
                value,
                NativeSetCondition(condition),
                ttlMilliseconds.HasValue,
                ttlMilliseconds.GetValueOrDefault(),
                mutationId,
                raw,
                cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw new OpenKacheException(
                "TIMEOUT",
                $"{opcode.ToString().ToUpperInvariant()} exceeded.");
        }
        catch (NativeException error)
        {
            throw MapNativeError(error, "CONNECTION_FAILED");
        }
    }

    private static ReadOnlyMemory<byte> ValidateItemId(ReadOnlyMemory<byte> itemId)
    {
        if (itemId.Length != Protocol.ItemIdBytes)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                $"item ID must contain exactly {Protocol.ItemIdBytes} bytes.");
        }

        return itemId;
    }

    private static ReadOnlyMemory<byte> ValidateApplicationKey(ReadOnlyMemory<byte> key)
    {
        if (key.IsEmpty)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "application key must not be empty.");
        }

        return key;
    }

    private static ReadOnlyMemory<byte> ValidateValue(ReadOnlyMemory<byte> value)
    {
        if (value.Length > Protocol.MaximumValueBytes)
        {
            throw new OpenKacheException(
                "VALUE_TOO_LARGE",
                $"Value size {value.Length} exceeds {Protocol.MaximumValueBytes} bytes.");
        }

        return value;
    }

    private static uint NativeSetCondition(Smithy.SetCondition? condition)
    {
        return condition switch
        {
            null => Protocol.FfiSetConditionNone,
            Smithy.SetCondition.IfAbsent => Protocol.FfiSetConditionIfAbsent,
            Smithy.SetCondition.IfPresent => Protocol.FfiSetConditionIfPresent,
            _ => throw new ArgumentOutOfRangeException(nameof(condition)),
        };
    }

    private static void ExpectKind(string operation, NativeResult result, uint expected)
    {
        if (result.Kind != expected)
        {
            throw UnexpectedKind(operation, result.Kind);
        }
    }

    private static OpenKacheException UnexpectedKind(string operation, uint kind)
    {
        return new OpenKacheException(
            "PROTOCOL_ERROR",
            $"{operation} returned unexpected native result kind {kind}.");
    }

    private static OpenKacheException MapNativeError(
        NativeException error,
        string fallbackCode)
    {
        var message = error.Message;
        var normalized = message.ToUpperInvariant();
        var code = normalized switch
        {
            _ when normalized.Contains("TIMEOUT", StringComparison.Ordinal)
                || normalized.Contains("TIMED OUT", StringComparison.Ordinal) => "TIMEOUT",
            _ when normalized.Contains("ITEM_ID", StringComparison.Ordinal)
                || normalized.Contains("PROTOCOL", StringComparison.Ordinal)
                || normalized.Contains("OPERATION DOES NOT", StringComparison.Ordinal)
                || normalized.Contains("SET TTL", StringComparison.Ordinal) => "PROTOCOL_ERROR",
            _ when normalized.Contains("EXCEEDS", StringComparison.Ordinal)
                && normalized.Contains("VALUE", StringComparison.Ordinal) => "VALUE_TOO_LARGE",
            _ when normalized.Contains("TOOLARGE", StringComparison.Ordinal) => "TOO_LARGE",
            _ when normalized.Contains("INVALIDREQUEST", StringComparison.Ordinal) => "INVALID_REQUEST",
            _ when normalized.Contains("UNSUPPORTEDOPERATION", StringComparison.Ordinal) =>
                "UNSUPPORTED_OPCODE",
            _ when normalized.Contains("FORBIDDEN", StringComparison.Ordinal) => "FORBIDDEN",
            _ when normalized.Contains("OVERLOADED", StringComparison.Ordinal) => "OVERLOADED",
            _ when normalized.Contains("INTERNAL", StringComparison.Ordinal) => "INTERNAL_ERROR",
            _ => fallbackCode,
        };
        return new OpenKacheException(code, message, error, error.Metadata);
    }

    private static byte[] CreateMutationId()
    {
        var mutationId = new byte[Protocol.MutationIdBytes];
        System.Security.Cryptography.RandomNumberGenerator.Fill(mutationId);
        return mutationId;
    }
}
