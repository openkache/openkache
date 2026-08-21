// SPDX-FileCopyrightText: 2026 OpenStd Inc.
// SPDX-License-Identifier: Apache-2.0

using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;

namespace OpenKache;

internal readonly record struct NativeResult(uint Kind, byte[] Payload, IntPtr Client);

internal sealed class NativeException : Exception
{
    internal NativeException(string message)
        : base(message) {}
}

internal static partial class NativeMethods
{
    private const string LibraryName = "openkache_client_core";

    static NativeMethods()
    {
        NativeLibrary.SetDllImportResolver(
            typeof(NativeMethods).Assembly,
            ResolveLibrary);
        ValidateNamespaceDescriptorLayout();
    }

    private static void ValidateNamespaceDescriptorLayout()
    {
        static int Offset(string fieldName) =>
            checked((int)Marshal.OffsetOf<Protocol.FfiNamespaceDescriptor>(fieldName));

        if (Marshal.SizeOf<Protocol.FfiNamespaceDescriptor>()
                != Protocol.FfiNamespaceDescriptorSizeBytes
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.NamespaceId))
                != Protocol.FfiNamespaceDescriptorNamespaceIdOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.Revision))
                != Protocol.FfiNamespaceDescriptorRevisionOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.DefaultTtlMs))
                != Protocol.FfiNamespaceDescriptorDefaultTtlMsOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.DefaultExpiration))
                != Protocol.FfiNamespaceDescriptorDefaultExpirationOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.ExpirationOverride))
                != Protocol.FfiNamespaceDescriptorExpirationOverrideOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.DefaultEviction))
                != Protocol.FfiNamespaceDescriptorDefaultEvictionOffset
            || Offset(nameof(Protocol.FfiNamespaceDescriptor.EvictionOverride))
                != Protocol.FfiNamespaceDescriptorEvictionOverrideOffset)
        {
            throw new TypeInitializationException(
                typeof(NativeMethods).FullName,
                new InvalidOperationException(
                    "native namespace descriptor layout does not match the Smithy contract."));
        }
    }

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
                throw new NativeException(
                    payload.Length == 0
                        ? "native client operation failed"
                        : Encoding.UTF8.GetString(payload));
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

    internal NativeBuffer(ReadOnlyMemory<byte> bytes)
    {
        if (bytes.IsEmpty)
        {
            return;
        }

        var copy = bytes.ToArray();
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

    private NativeClient(IntPtr handle)
    {
        _handle = handle;
    }

    internal static async ValueTask<NativeClient> ConnectAsync(
        string address,
        string serverName,
        ReadOnlyMemory<byte> certificate,
        TimeSpan connectTimeout,
        TimeSpan requestTimeout,
        int maximumStreamLanes,
        Transport transport,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => ConnectNative(
                address,
                serverName,
                certificate,
                connectTimeout,
                requestTimeout,
                maximumStreamLanes,
                transport),
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
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        uint setCondition,
        bool ttlEnabled,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => ExecuteNative(
                operation,
                itemId,
                value,
                setCondition,
                ttlEnabled,
                ttlMilliseconds),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    internal async ValueTask<NativeResult> ExecuteRawWithOptionsAsync(
        uint operation,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => ExecuteRawWithOptionsNative(
                operation,
                itemId,
                value,
                setFlags,
                ttlMilliseconds),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    internal async ValueTask<NativeResult> ExecuteScopedAsync(
        uint operation,
        ulong namespaceId,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => ExecuteScopedNative(
                operation,
                namespaceId,
                itemId,
                value,
                setFlags,
                ttlMilliseconds),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    internal async ValueTask<NativeResult> NamespaceOpenAsync(
        ReadOnlyMemory<byte> name,
        bool createIfMissing,
        byte policyFlags,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => NamespaceOpenNative(name, createIfMissing, policyFlags, ttlMilliseconds),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    internal async ValueTask<NativeResult> NamespaceUpdatePolicyAsync(
        ulong namespaceId,
        ulong expectedRevision,
        byte policyFlags,
        ulong ttlMilliseconds,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => NamespaceUpdatePolicyNative(
                namespaceId,
                expectedRevision,
                policyFlags,
                ttlMilliseconds),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
    }

    internal async ValueTask<NativeResult> NamespaceDeleteAsync(
        ulong namespaceId,
        ulong expectedRevision,
        CancellationToken cancellationToken)
    {
        var task = Task.Run(
            () => NamespaceDeleteNative(namespaceId, expectedRevision),
            CancellationToken.None);
        return await task.WaitAsync(cancellationToken).ConfigureAwait(false);
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

    private static NativeClient ConnectNative(
        string address,
        string serverName,
        ReadOnlyMemory<byte> certificate,
        TimeSpan connectTimeout,
        TimeSpan requestTimeout,
        int maximumStreamLanes,
        Transport transport)
    {
        if (NativeMethods.openkache_client_abi_version() != Protocol.FfiAbiVersion)
        {
            throw new NativeException("unsupported shared OpenKache client ABI version");
        }

        using var addressBuffer = new NativeBuffer(Encoding.UTF8.GetBytes(address));
        using var serverNameBuffer = new NativeBuffer(Encoding.UTF8.GetBytes(serverName));
        using var certificateBuffer = new NativeBuffer(certificate);
        using var dataProtectionKey = new NativeBuffer(Array.Empty<byte>());
        var options = new NativeMethods.ConnectOptions
        {
            Address = addressBuffer.Pointer,
            AddressLength = addressBuffer.Length,
            ServerName = serverNameBuffer.Pointer,
            ServerNameLength = serverNameBuffer.Length,
            Certificate = certificateBuffer.Pointer,
            CertificateLength = certificateBuffer.Length,
            DataProtectionKey = dataProtectionKey.Pointer,
            DataProtectionKeyLength = dataProtectionKey.Length,
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

        IntPtr result;
        try
        {
            result = transport == Transport.Quic
                ? NativeMethods.openkache_client_connect_with_options(ref options)
                : NativeMethods.openkache_client_connect_transport(
                    ref options, (uint)transport);
        }
        catch (EntryPointNotFoundException)
        {
            throw new NativeException(
                "native OpenKache client does not export the optional transport selector");
        }
        var nativeResult = NativeMethods.ReadResult(result, takeClient: true);
        if (nativeResult.Kind != Protocol.FfiResultConnected)
        {
            throw new NativeException("native client did not return a connected handle");
        }
        return new NativeClient(nativeResult.Client);
    }

    private NativeResult ExecuteNative(
        uint operation,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        uint setCondition,
        bool ttlEnabled,
        ulong ttlMilliseconds)
    {
        var handle = AcquireHandle();
        try
        {
            using var itemIdBuffer = new NativeBuffer(itemId);
            using var valueBuffer = new NativeBuffer(value);
            var result = NativeMethods.openkache_client_execute_raw(
                handle,
                operation,
                itemIdBuffer.Pointer,
                itemIdBuffer.Length,
                valueBuffer.Pointer,
                valueBuffer.Length,
                setCondition,
                ttlEnabled ? (byte)1 : (byte)0,
                ttlMilliseconds);
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private NativeResult ExecuteRawWithOptionsNative(
        uint operation,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags,
        ulong ttlMilliseconds)
    {
        var handle = AcquireHandle();
        try
        {
            using var itemIdBuffer = new NativeBuffer(itemId);
            using var valueBuffer = new NativeBuffer(value);
            var result = NativeMethods.openkache_client_execute_raw_with_options(
                handle,
                operation,
                itemIdBuffer.Pointer,
                itemIdBuffer.Length,
                valueBuffer.Pointer,
                valueBuffer.Length,
                setFlags,
                ttlMilliseconds);
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private NativeResult ExecuteScopedNative(
        uint operation,
        ulong namespaceId,
        ReadOnlyMemory<byte> itemId,
        ReadOnlyMemory<byte> value,
        byte setFlags,
        ulong ttlMilliseconds)
    {
        var handle = AcquireHandle();
        try
        {
            using var itemIdBuffer = new NativeBuffer(itemId);
            using var valueBuffer = new NativeBuffer(value);
            var result = NativeMethods.openkache_client_execute_scoped(
                handle,
                operation,
                namespaceId,
                itemIdBuffer.Pointer,
                itemIdBuffer.Length,
                valueBuffer.Pointer,
                valueBuffer.Length,
                setFlags,
                ttlMilliseconds);
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private NativeResult NamespaceOpenNative(
        ReadOnlyMemory<byte> name,
        bool createIfMissing,
        byte policyFlags,
        ulong ttlMilliseconds)
    {
        var handle = AcquireHandle();
        try
        {
            using var nameBuffer = new NativeBuffer(name);
            var result = NativeMethods.openkache_client_namespace_open(
                handle,
                nameBuffer.Pointer,
                nameBuffer.Length,
                createIfMissing ? (byte)1 : (byte)0,
                policyFlags,
                ttlMilliseconds);
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private NativeResult NamespaceUpdatePolicyNative(
        ulong namespaceId,
        ulong expectedRevision,
        byte policyFlags,
        ulong ttlMilliseconds)
    {
        var handle = AcquireHandle();
        try
        {
            var result = NativeMethods.openkache_client_namespace_update_policy(
                handle,
                namespaceId,
                expectedRevision,
                policyFlags,
                ttlMilliseconds);
            return NativeMethods.ReadResult(result);
        }
        finally
        {
            ReleaseCall();
        }
    }

    private NativeResult NamespaceDeleteNative(ulong namespaceId, ulong expectedRevision)
    {
        var handle = AcquireHandle();
        try
        {
            var result = NativeMethods.openkache_client_namespace_delete(
                handle,
                namespaceId,
                expectedRevision);
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
