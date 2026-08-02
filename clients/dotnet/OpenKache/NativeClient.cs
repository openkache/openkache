// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;

namespace OpenKache;

internal readonly record struct NativeResult(
    uint Kind,
    byte[] Payload,
    IntPtr Client,
    ErrorMetadata? Metadata = null);

internal sealed class NativeException : Exception
{
    internal NativeException(string message, ErrorMetadata? metadata = null)
        : base(message)
    {
        Metadata = metadata;
    }

    internal ErrorMetadata? Metadata { get; }
}

internal static class NativeMethods
{
    private const string LibraryName = "openkache_client_core";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(
            typeof(NativeMethods).Assembly,
            ResolveLibrary);
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct ConnectOptions
    {
        internal IntPtr Address;
        internal nuint AddressLength;
        internal IntPtr ServerName;
        internal nuint ServerNameLength;
        internal IntPtr Certificate;
        internal nuint CertificateLength;
        internal IntPtr ClientCertificateChain;
        internal nuint ClientCertificateChainLength;
        internal IntPtr ClientPrivateKey;
        internal nuint ClientPrivateKeyLength;
        internal IntPtr DataProtectionKey;
        internal nuint DataProtectionKeyLength;
        internal IntPtr PreviousDataProtectionKeys;
        internal nuint PreviousDataProtectionKeysLength;
        internal nuint PreviousDataProtectionKeyCount;
        internal byte CompressionEnabled;
        internal int CompressionLevel;
        internal nuint MinimumInputSize;
        internal nuint MinimumSavings;
        internal uint Encryption;
        internal ulong ConnectTimeoutMilliseconds;
        internal ulong RequestTimeoutMilliseconds;
        internal nuint RetryMaxAttempts;
        internal nuint MaxInFlight;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct NativeErrorMetadata
    {
        internal uint Code;
        internal uint Operation;
        internal uint Phase;
        internal uint Backend;
        internal byte Retryable;
        internal byte Ambiguous;
        internal byte MutationIdLength;
        internal byte Reserved;
        [MarshalAs(UnmanagedType.ByValArray, SizeConst = Protocol.MutationIdBytes)]
        internal byte[]? MutationId;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct MetricsSnapshot
    {
        internal ulong Requests;
        internal ulong Hits;
        internal ulong Misses;
        internal ulong Retries;
        internal ulong Reconnects;
        internal ulong Cancellations;
        internal ulong TransportErrors;
        internal ulong ProtocolErrors;
        internal ulong BytesSent;
        internal ulong BytesReceived;
        internal ulong ActiveLanes;
    }

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.U4)]
    internal static extern uint openkache_client_abi_version();

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_connect_with_options(
        ref ConnectOptions options);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_execute_raw_with_request_id(
        IntPtr client,
        ulong requestId,
        uint operation,
        IntPtr itemId,
        nuint itemIdLength,
        IntPtr value,
        nuint valueLength,
        uint setCondition,
        byte ttlEnabled,
        ulong ttlMilliseconds);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_execute_with_request_id(
        IntPtr client,
        ulong requestId,
        uint operation,
        IntPtr applicationKey,
        nuint applicationKeyLength,
        IntPtr value,
        nuint valueLength,
        uint setCondition,
        byte ttlEnabled,
        ulong ttlMilliseconds);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_execute_with_request_id_and_mutation_id(
        IntPtr client,
        ulong requestId,
        uint operation,
        IntPtr applicationKey,
        nuint applicationKeyLength,
        IntPtr value,
        nuint valueLength,
        uint setCondition,
        byte ttlEnabled,
        ulong ttlMilliseconds,
        IntPtr mutationId,
        nuint mutationIdLength);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_execute_raw_with_request_id_and_mutation_id(
        IntPtr client,
        ulong requestId,
        uint operation,
        IntPtr itemId,
        nuint itemIdLength,
        IntPtr value,
        nuint valueLength,
        uint setCondition,
        byte ttlEnabled,
        ulong ttlMilliseconds,
        IntPtr mutationId,
        nuint mutationIdLength);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint openkache_client_connection_state(IntPtr client);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern uint openkache_client_result_kind(IntPtr result);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_result_data(IntPtr result);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern nuint openkache_client_result_data_length(IntPtr result);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern byte openkache_client_result_error_metadata(
        IntPtr result,
        out NativeErrorMetadata metadata);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern byte openkache_client_cancel(IntPtr client, ulong requestId);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    [return: MarshalAs(UnmanagedType.I1)]
    internal static extern byte openkache_client_metrics_snapshot(
        IntPtr client,
        out MetricsSnapshot snapshot);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr openkache_client_result_take_client(IntPtr result);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void openkache_client_result_free(IntPtr result);

    [DllImport(LibraryName, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void openkache_client_free(IntPtr client);

    internal static NativeResult ReadResult(IntPtr result, bool takeClient = false)
    {
        if (result == IntPtr.Zero)
        {
            throw new NativeException("native client returned a null result");
        }

        try
        {
            var kind = openkache_client_result_kind(result);
            var length = openkache_client_result_data_length(result);
            if (length > int.MaxValue)
            {
                throw new NativeException("native client returned an oversized payload");
            }

            var data = openkache_client_result_data(result);
            if (length != 0 && data == IntPtr.Zero)
            {
                throw new NativeException("native client returned a null payload");
            }

            var payload = new byte[(int)length];
            if (payload.Length != 0)
            {
                Marshal.Copy(data, payload, 0, payload.Length);
            }

            if (kind == Protocol.FfiResultError)
            {
                OpenKache.ErrorMetadata? metadata = null;
                if (openkache_client_result_error_metadata(result, out var nativeMetadata) != 0)
                {
                    metadata = new OpenKache.ErrorMetadata(
                        nativeMetadata.Code,
                        nativeMetadata.Operation,
                        nativeMetadata.Phase,
                        nativeMetadata.Backend,
                        nativeMetadata.Retryable != 0,
                        nativeMetadata.Ambiguous != 0,
                        nativeMetadata.MutationId is null || nativeMetadata.MutationIdLength == 0
                            ? null
                            : nativeMetadata.MutationId[..Math.Min(
                                nativeMetadata.MutationIdLength,
                                nativeMetadata.MutationId.Length)]);
                }
                throw new NativeException(
                    payload.Length == 0
                        ? "native client operation failed"
                        : Encoding.UTF8.GetString(payload),
                    metadata);
            }

            var client = IntPtr.Zero;
            if (takeClient)
            {
                client = openkache_client_result_take_client(result);
                if (client == IntPtr.Zero)
                {
                    throw new NativeException("native client returned no client handle");
                }
            }

            return new NativeResult(kind, payload, client);
        }
        finally
        {
            openkache_client_result_free(result);
        }
    }

    private static IntPtr ResolveLibrary(
        string libraryName,
        Assembly assembly,
        DllImportSearchPath? searchPath)
    {
        if (!string.Equals(libraryName, LibraryName, StringComparison.Ordinal))
        {
            return IntPtr.Zero;
        }

        var configured = Environment.GetEnvironmentVariable("OPENKACHE_CLIENT_NATIVE");
        if (!string.IsNullOrWhiteSpace(configured)
            && NativeLibrary.TryLoad(configured, out var configuredHandle))
        {
            return configuredHandle;
        }

        return IntPtr.Zero;
    }
}

internal sealed class NativeBuffer : IDisposable
{
    private GCHandle _handle;
    private byte[]? _copy;

    internal NativeBuffer(ReadOnlyMemory<byte> bytes)
    {
        if (bytes.IsEmpty)
        {
            return;
        }

        var copy = bytes.ToArray();
        _copy = copy;
        _handle = GCHandle.Alloc(copy, GCHandleType.Pinned);
        Pointer = _handle.AddrOfPinnedObject();
        Length = (nuint)copy.Length;
    }

    internal IntPtr Pointer { get; }

    internal nuint Length { get; }

    public void Dispose()
    {
        if (_handle.IsAllocated)
        {
            if (_copy is { } copy)
            {
                Array.Clear(copy, 0, copy.Length);
            }
            _copy = null;
            _handle.Free();
        }
    }
}

internal sealed class NativeClient : IAsyncDisposable
{
    private readonly object _gate = new();
    private IntPtr _handle;
    private int _activeCalls;
    private bool _closed;
    private long _nextRequestId = 1;

    private NativeClient(IntPtr handle)
    {
        _handle = handle;
    }

    internal static async ValueTask<NativeClient> ConnectAsync(
        string address,
        string serverName,
        ReadOnlyMemory<byte> certificate,
        ReadOnlyMemory<byte> dataProtectionKey,
        IReadOnlyList<ReadOnlyMemory<byte>> previousDataProtectionKeys,
        TimeSpan connectTimeout,
        TimeSpan requestTimeout,
        int maximumStreamLanes,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => ConnectNative(
                address,
                serverName,
                certificate,
                dataProtectionKey,
                previousDataProtectionKeys,
                connectTimeout,
                requestTimeout,
                maximumStreamLanes),
            CancellationToken.None);
        try
        {
            return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch
        {
            _ = task.ContinueWith(
                static completed =>
                {
                    if (completed.Status == TaskStatus.RanToCompletion)
                    {
                        _ = completed.Result.DisposeAsync();
                    }
                },
                CancellationToken.None,
                TaskContinuationOptions.ExecuteSynchronously,
                TaskScheduler.Default);
            throw;
        }
    }

    internal async ValueTask<NativeResult> ExecuteAsync(
        uint operation,
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        uint setCondition,
        bool ttlEnabled,
        ulong ttlMilliseconds,
        ReadOnlyMemory<byte> mutationId,
        bool raw,
        CancellationToken cancellationToken)
    {
        var requestId = AllocateRequestId();
        var task = Task.Run(
            () => ExecuteNative(
                requestId,
                operation,
                key,
                value,
                setCondition,
                ttlEnabled,
                ttlMilliseconds,
                mutationId,
                raw),
            CancellationToken.None);
        try
        {
            return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            TryCancel(requestId);
            throw;
        }
    }

    private ulong AllocateRequestId()
    {
        while (true)
        {
            var current = Volatile.Read(ref _nextRequestId);
            var next = current <= 0 || current == long.MaxValue ? 1 : current + 1;
            if (Interlocked.CompareExchange(ref _nextRequestId, next, current) == current)
            {
                return (ulong)(current > 0 ? current : 1);
            }
        }
    }

    public async ValueTask DisposeAsync()
    {
        IntPtr handle;
        lock (_gate)
        {
            if (_closed)
            {
                return;
            }

            _closed = true;
            while (_activeCalls != 0)
            {
                Monitor.Wait(_gate);
            }

            handle = _handle;
            _handle = IntPtr.Zero;
        }

        if (handle != IntPtr.Zero)
        {
            await Task.Run(
                () => NativeMethods.openkache_client_free(handle)).ConfigureAwait(false);
        }
    }

    internal uint ConnectionState()
    {
        var handle = AcquireHandle();
        try
        {
            return NativeMethods.openkache_client_connection_state(handle);
        }
        finally
        {
            ReleaseCall();
        }
    }

    internal NativeMethods.MetricsSnapshot Metrics()
    {
        var handle = AcquireHandle();
        try
        {
            if (NativeMethods.openkache_client_metrics_snapshot(handle, out var snapshot) == 0)
            {
                throw new NativeException("native client did not return metrics");
            }
            return snapshot;
        }
        finally
        {
            ReleaseCall();
        }
    }

    private static NativeClient ConnectNative(
        string address,
        string serverName,
        ReadOnlyMemory<byte> certificate,
        ReadOnlyMemory<byte> dataProtectionKey,
        IReadOnlyList<ReadOnlyMemory<byte>> previousDataProtectionKeys,
        TimeSpan connectTimeout,
        TimeSpan requestTimeout,
        int maximumStreamLanes)
    {
        if (NativeMethods.openkache_client_abi_version() != Protocol.FfiAbiVersion)
        {
            throw new NativeException("unsupported shared OpenKache client ABI version");
        }

        using var addressBuffer = new NativeBuffer(Encoding.UTF8.GetBytes(address));
        using var serverNameBuffer = new NativeBuffer(Encoding.UTF8.GetBytes(serverName));
        using var certificateBuffer = new NativeBuffer(certificate);
        using var dataProtectionKeyBuffer = new NativeBuffer(dataProtectionKey);
        var previousBytes = previousDataProtectionKeys
            .SelectMany(static key => key.ToArray())
            .ToArray();
        using var previousDataProtectionKeysBuffer = new NativeBuffer(previousBytes);
        Array.Clear(previousBytes, 0, previousBytes.Length);
        var options = new NativeMethods.ConnectOptions
        {
            Address = addressBuffer.Pointer,
            AddressLength = addressBuffer.Length,
            ServerName = serverNameBuffer.Pointer,
            ServerNameLength = serverNameBuffer.Length,
            Certificate = certificateBuffer.Pointer,
            CertificateLength = certificateBuffer.Length,
            DataProtectionKey = dataProtectionKeyBuffer.Pointer,
            DataProtectionKeyLength = dataProtectionKeyBuffer.Length,
            PreviousDataProtectionKeys = previousDataProtectionKeysBuffer.Pointer,
            PreviousDataProtectionKeysLength = previousDataProtectionKeysBuffer.Length,
            PreviousDataProtectionKeyCount = (nuint)previousDataProtectionKeys.Count,
            CompressionEnabled = 0,
            CompressionLevel = Protocol.DefaultZstandardLevel,
            MinimumInputSize = (nuint)Protocol.DefaultZstandardMinimumInputBytes,
            MinimumSavings = (nuint)Protocol.DefaultZstandardMinimumSavingsBytes,
            Encryption = Protocol.ValueFormatEncryptionRobust,
            ConnectTimeoutMilliseconds = ToMilliseconds(connectTimeout),
            RequestTimeoutMilliseconds = ToMilliseconds(requestTimeout),
            RetryMaxAttempts = (nuint)Protocol.DefaultRetryMaxAttempts,
            MaxInFlight = (nuint)maximumStreamLanes,
        };

        var result = NativeMethods.openkache_client_connect_with_options(ref options);
        var nativeResult = NativeMethods.ReadResult(result, takeClient: true);
        if (nativeResult.Kind != Protocol.FfiResultConnected)
        {
            throw new NativeException("native client did not return a connected handle");
        }
        return new NativeClient(nativeResult.Client);
    }

    private NativeResult ExecuteNative(
        ulong requestId,
        uint operation,
        ReadOnlyMemory<byte> key,
        ReadOnlyMemory<byte> value,
        uint setCondition,
        bool ttlEnabled,
        ulong ttlMilliseconds,
        ReadOnlyMemory<byte> mutationId,
        bool raw)
    {
        var handle = AcquireHandle();
        try
        {
            using var keyBuffer = new NativeBuffer(key);
            using var valueBuffer = new NativeBuffer(value);
            using var mutationBuffer = new NativeBuffer(mutationId);
            var result = mutationId.IsEmpty
                ? (raw
                    ? NativeMethods.openkache_client_execute_raw_with_request_id(
                        handle,
                        requestId,
                        operation,
                        keyBuffer.Pointer,
                        keyBuffer.Length,
                        valueBuffer.Pointer,
                        valueBuffer.Length,
                        setCondition,
                        ttlEnabled ? (byte)1 : (byte)0,
                        ttlMilliseconds)
                    : NativeMethods.openkache_client_execute_with_request_id(
                        handle,
                        requestId,
                        operation,
                        keyBuffer.Pointer,
                        keyBuffer.Length,
                        valueBuffer.Pointer,
                        valueBuffer.Length,
                        setCondition,
                        ttlEnabled ? (byte)1 : (byte)0,
                        ttlMilliseconds))
                : (raw
                    ? NativeMethods.openkache_client_execute_raw_with_request_id_and_mutation_id(
                        handle,
                        requestId,
                        operation,
                        keyBuffer.Pointer,
                        keyBuffer.Length,
                        valueBuffer.Pointer,
                        valueBuffer.Length,
                        setCondition,
                        ttlEnabled ? (byte)1 : (byte)0,
                        ttlMilliseconds,
                        mutationBuffer.Pointer,
                        mutationBuffer.Length)
                    : NativeMethods.openkache_client_execute_with_request_id_and_mutation_id(
                        handle,
                        requestId,
                        operation,
                        keyBuffer.Pointer,
                        keyBuffer.Length,
                        valueBuffer.Pointer,
                        valueBuffer.Length,
                        setCondition,
                        ttlEnabled ? (byte)1 : (byte)0,
                        ttlMilliseconds,
                        mutationBuffer.Pointer,
                        mutationBuffer.Length));
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private IntPtr AcquireHandle()
    {
        lock (_gate)
        {
            if (_closed || _handle == IntPtr.Zero)
            {
                throw new ObjectDisposedException(nameof(NativeClient));
            }

            _activeCalls += 1;
            return _handle;
        }
    }

    private bool TryCancel(ulong requestId)
    {
        IntPtr handle;
        lock (_gate)
        {
            if (_closed || _handle == IntPtr.Zero)
            {
                return false;
            }

            _activeCalls += 1;
            handle = _handle;
        }

        try
        {
            return NativeMethods.openkache_client_cancel(handle, requestId) != 0;
        }
        finally
        {
            ReleaseCall();
        }
    }

    private void ReleaseCall()
    {
        lock (_gate)
        {
            _activeCalls -= 1;
            if (_activeCalls == 0)
            {
                Monitor.PulseAll(_gate);
            }
        }
    }

    private static ulong ToMilliseconds(TimeSpan timeout)
    {
        return checked((ulong)Math.Max(1, timeout.TotalMilliseconds));
    }
}
