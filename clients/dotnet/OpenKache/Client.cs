// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Text;

namespace OpenKache;

/// <summary>
/// An asynchronous, thread-safe client for the OpenKache protocol.
/// </summary>
public sealed partial class Client : IAsyncDisposable, Smithy.IOpenKacheApi
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
            Protocol.Opcode.Ping,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("PING", result, Protocol.FfiResultOk);
    }

    /// <summary>
    /// Sends an experimental UTF-8 message and returns the echoed message.
    /// </summary>
    public async ValueTask<string> EchoAsync(
        string message,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(message);
        var result = await RequestAsync(
            Protocol.Opcode.Echo,
            ReadOnlyMemory<byte>.Empty,
            ValidateValue(Encoding.UTF8.GetBytes(message)),
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("ECHO", result, Protocol.FfiResultValue);
        try
        {
            return new UTF8Encoding(false, true).GetString(result.Payload);
        }
        catch (DecoderFallbackException error)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "ECHO returned invalid UTF-8.",
                error);
        }
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
            Protocol.Opcode.Get,
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
        var (setFlags, ttlMilliseconds) = options.ValidateAndGetWireOptions();
        var result = await RequestRawWithOptionsAsync(
            Protocol.Opcode.Set,
            ValidateItemId(itemId),
            ValidateValue(value),
            setFlags,
            ttlMilliseconds,
            cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultCreated => Smithy.SetOutcome.Created,
            var kind when kind == Protocol.FfiResultReplaced => Smithy.SetOutcome.Replaced,
            var kind when kind == Protocol.FfiResultNotStored => Smithy.SetOutcome.NotStored,
            _ => throw UnexpectedKind("SET", result.Kind),
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
        var result = await RequestAsync(
            Protocol.Opcode.Delete,
            ValidateItemId(itemId),
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        return result.Kind switch
        {
            var kind when kind == Protocol.FfiResultDeleted => true,
            var kind when kind == Protocol.FfiResultNotDeleted => false,
            _ => throw UnexpectedKind("DELETE", result.Kind),
        };
    }

    /// <summary>
    /// Returns the server statistics payload as UTF-8 JSON.
    /// </summary>
    public async ValueTask<string> StatsAsync(
        CancellationToken cancellationToken = default)
    {
        var result = await RequestAsync(
            Protocol.Opcode.Stats,
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
            Protocol.Opcode.Sync,
            ReadOnlyMemory<byte>.Empty,
            ReadOnlyMemory<byte>.Empty,
            cancellationToken: cancellationToken).ConfigureAwait(false);
        ExpectKind("SYNC", result, Protocol.FfiResultOk);
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

    private async ValueTask<NativeResult> RequestScopedAsync(
        Protocol.Opcode opcode,
        ulong namespaceId,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags = 0,
        ulong ttlMilliseconds = 0,
        CancellationToken cancellationToken = default)
    {
        ObjectDisposedException.ThrowIf(
            Volatile.Read(ref _disposed) != 0,
            this);
        try
        {
            return await _nativeClient.ExecuteScopedAsync(
                (uint)opcode,
                namespaceId,
                itemId,
                value,
                setFlags,
                ttlMilliseconds,
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

    private async ValueTask<NativeResult> RequestRawWithOptionsAsync(
        Protocol.Opcode opcode,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        ObjectDisposedException.ThrowIf(
            Volatile.Read(ref _disposed) != 0,
            this);
        try
        {
            return await _nativeClient.ExecuteRawWithOptionsAsync(
                (uint)opcode,
                itemId,
                value,
                setFlags,
                ttlMilliseconds,
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

    private static (byte Flags, ulong TtlMilliseconds) NativeSetOptions(
        Smithy.SetInput input)
    {
        var flags = input.Condition switch
        {
            null or Smithy.SetCondition.Any => Protocol.SetConditionAnyBits,
            Smithy.SetCondition.IfAbsent => Protocol.SetIfAbsentBits,
            Smithy.SetCondition.IfPresent => Protocol.SetIfPresentBits,
            _ => throw new ArgumentOutOfRangeException(nameof(input.Condition)),
        };
        flags |= input.ExpirationMode switch
        {
            null or Smithy.ExpirationMode.Inherit when input.TtlMilliseconds is null =>
                Protocol.SetInheritExpirationBits,
            Smithy.ExpirationMode.NoExpiry when input.TtlMilliseconds is null =>
                Protocol.SetNoExpiryBits,
            Smithy.ExpirationMode.ExplicitTtl when input.TtlMilliseconds is > 0 =>
                Protocol.SetExplicitTtlBits,
            Smithy.ExpirationMode.NoExpiry or Smithy.ExpirationMode.Inherit =>
                throw new ArgumentException(
                    $"ttlMilliseconds is only valid with {Protocol.SmithyExpirationModeExplicitTtlValue}.",
                    nameof(input.TtlMilliseconds)),
            Smithy.ExpirationMode.ExplicitTtl => throw new ArgumentException(
                $"ttlMilliseconds must be positive with {Protocol.SmithyExpirationModeExplicitTtlValue}.",
                nameof(input.TtlMilliseconds)),
            _ => throw new ArgumentOutOfRangeException(nameof(input.ExpirationMode)),
        };
        flags |= input.EvictionMode switch
        {
            null or Smithy.EvictionMode.Inherit => Protocol.SetInheritEvictionBits,
            Smithy.EvictionMode.Evictable => Protocol.SetEvictableBits,
            Smithy.EvictionMode.EvictionProtected => Protocol.SetEvictionProtectedBits,
            _ => throw new ArgumentOutOfRangeException(nameof(input.EvictionMode)),
        };
        return (flags, input.TtlMilliseconds.GetValueOrDefault());
    }

    private static (byte Flags, ulong TtlMilliseconds) NativePolicy(
        Smithy.NamespacePolicy? policy)
    {
        if (policy is null)
        {
            return (0, 0);
        }
        var flags = policy.DefaultExpiration switch
        {
            Smithy.ExpirationDefault.NoExpiry when policy.DefaultTtlMilliseconds is null =>
                Protocol.PolicyNoExpiry,
            Smithy.ExpirationDefault.FixedTtl when policy.DefaultTtlMilliseconds is > 0 =>
                Protocol.PolicyFixedTtl,
            Smithy.ExpirationDefault.NoExpiry or Smithy.ExpirationDefault.FixedTtl =>
                throw new ArgumentException(
                    $"defaultTtlMilliseconds must be present only for a positive {Protocol.SmithyExpirationDefaultFixedTtlValue}.",
                    nameof(policy.DefaultTtlMilliseconds)),
            _ => throw new ArgumentOutOfRangeException(nameof(policy.DefaultExpiration)),
        };
        if (policy.ExpirationOverride == Smithy.OverridePolicy.Allowed)
        {
            flags |= Protocol.PolicyExpirationOverride;
        }
        else if (policy.ExpirationOverride != Smithy.OverridePolicy.Disallowed)
        {
            throw new ArgumentOutOfRangeException(nameof(policy.ExpirationOverride));
        }
        if (policy.DefaultEviction == Smithy.EvictionDefault.EvictionProtected)
        {
            flags |= Protocol.PolicyEvictionProtected;
        }
        else if (policy.DefaultEviction != Smithy.EvictionDefault.Evictable)
        {
            throw new ArgumentOutOfRangeException(nameof(policy.DefaultEviction));
        }
        if (policy.EvictionOverride == Smithy.OverridePolicy.Allowed)
        {
            flags |= Protocol.PolicyEvictionOverride;
        }
        else if (policy.EvictionOverride != Smithy.OverridePolicy.Disallowed)
        {
            throw new ArgumentOutOfRangeException(nameof(policy.EvictionOverride));
        }
        return (flags, policy.DefaultTtlMilliseconds.GetValueOrDefault());
    }

    private static Smithy.NamespaceDescriptor DecodeNamespaceDescriptor(byte[] payload)
    {
        using var buffer = new NativeBuffer(payload);
        var status = NativeMethods.openkache_client_namespace_descriptor_decode(
            buffer.Pointer,
            buffer.Length,
            out var decoded);
        if (status != Protocol.FfiNamespaceDescriptorDecodeOk)
        {
            throw new OpenKacheException(
                "PROTOCOL_ERROR",
                "native ABI returned an invalid namespace descriptor.");
        }
        return new Smithy.NamespaceDescriptor
        {
            NamespaceId = decoded.NamespaceId,
            Revision = decoded.Revision,
            Policy = new Smithy.NamespacePolicy
            {
                DefaultExpiration = decoded.DefaultExpiration
                    == Protocol.FfiNamespaceDefaultExpirationFixedTtl
                    ? Smithy.ExpirationDefault.FixedTtl
                    : Smithy.ExpirationDefault.NoExpiry,
                DefaultTtlMilliseconds = decoded.DefaultExpiration
                    == Protocol.FfiNamespaceDefaultExpirationFixedTtl
                    ? decoded.DefaultTtlMs
                    : null,
                ExpirationOverride = decoded.ExpirationOverride
                    == Protocol.FfiNamespaceOverrideAllowed
                    ? Smithy.OverridePolicy.Allowed
                    : Smithy.OverridePolicy.Disallowed,
                DefaultEviction = decoded.DefaultEviction
                    == Protocol.FfiNamespaceDefaultEvictionProtected
                    ? Smithy.EvictionDefault.EvictionProtected
                    : Smithy.EvictionDefault.Evictable,
                EvictionOverride = decoded.EvictionOverride
                    == Protocol.FfiNamespaceOverrideAllowed
                    ? Smithy.OverridePolicy.Allowed
                    : Smithy.OverridePolicy.Disallowed,
            },
        };
    }

    private async ValueTask<NativeResult> RequestAsync(
        Protocol.Opcode opcode,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        Smithy.SetCondition? condition = null,
        ulong? ttlMilliseconds = null,
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
            null => Protocol.FfiSetConditionAny,
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
        return new OpenKacheException(code, message, error);
    }
}
