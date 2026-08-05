package io.openkache.client;

import com.sun.jna.Memory;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import io.openkache.client.generated_local.SmithyContract;
import io.openkache.client.generated_local.SmithyNativeApi;
import io.openkache.client.generated_local.SmithyNativeDescriptor;

import java.nio.ByteBuffer;
import java.nio.CharBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * Rust-backed Java client implementing the complete generated Smithy surface.
 *
 * <p>The adapter owns only Java/JNA marshaling and DTO conversion. QUIC, TLS,
 * retry, namespace framing, and response validation remain in the shared Rust
 * client core.</p>
 */
public final class Client implements OpenKacheClient, AutoCloseable {
    private static final class NativeBuffer implements AutoCloseable {
        private final Memory memory;

        private NativeBuffer(byte[] value) {
            memory = value.length == 0 ? null : new Memory(value.length);
            if (memory != null) {
                memory.write(0, value, 0, value.length);
            }
        }

        private Pointer pointer() {
            return memory;
        }

        private long length() {
            return memory == null ? 0 : memory.size();
        }

        @Override
        public void close() {
            if (memory != null) {
                memory.close();
            }
        }
    }

    private final SmithyNativeApi nativeApi;
    private final ExecutorService executor;
    private final Object lifecycle = new Object();
    private Pointer handle;
    private boolean closed;

    private Client(SmithyNativeApi nativeApi, Pointer handle) {
        this.nativeApi = nativeApi;
        this.handle = handle;
        executor = Executors.newVirtualThreadPerTaskExecutor();
    }

    /**
     * Connects to an OpenKache server using the shared Rust client core.
     *
     * @param address host and port, such as {@code 127.0.0.1:4433}
     * @param serverName TLS server name
     * @param certificate DER certificate, PEM chain, or an empty array
     * @param dataProtectionKey exactly 32 bytes shared with the server client
     * @return connected Java client
     */
    public static Client connect(
        String address,
        String serverName,
        byte[] certificate,
        byte[] dataProtectionKey) {
        Objects.requireNonNull(address, "address");
        Objects.requireNonNull(serverName, "serverName");
        Objects.requireNonNull(certificate, "certificate");
        Objects.requireNonNull(dataProtectionKey, "dataProtectionKey");
        if (dataProtectionKey.length != 32) {
            throw new IllegalArgumentException("dataProtectionKey must contain exactly 32 bytes");
        }

        SmithyNativeApi nativeApi = loadNativeApi();
        if (nativeApi.openkache_client_abi_version() != SmithyContract.ABI_VERSION) {
            throw new OpenKacheClientException("unsupported OpenKache native ABI version");
        }
        try (
            NativeBuffer addressBuffer = new NativeBuffer(address.getBytes(StandardCharsets.UTF_8));
            NativeBuffer serverNameBuffer = new NativeBuffer(serverName.getBytes(StandardCharsets.UTF_8));
            NativeBuffer certificateBuffer = new NativeBuffer(certificate);
            NativeBuffer keyBuffer = new NativeBuffer(dataProtectionKey)
        ) {
            NativeResult result = readResult(
                nativeApi,
                nativeApi.openkache_client_connect(
                    addressBuffer.pointer(),
                    addressBuffer.length(),
                    serverNameBuffer.pointer(),
                    serverNameBuffer.length(),
                    certificateBuffer.pointer(),
                    certificateBuffer.length(),
                    keyBuffer.pointer(),
                    keyBuffer.length(),
                    (byte) 0,
                    SmithyContract.DEFAULT_ZSTANDARD_LEVEL,
                    SmithyContract.DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
                    SmithyContract.DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
                    SmithyContract.DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
                    SmithyContract.DEFAULT_REQUEST_TIMEOUT_MILLISECONDS),
                true);
            if (result.kind() != SmithyContract.RESULT_CONNECTED || result.client() == null) {
                throw new OpenKacheClientException("native client did not return a connected handle");
            }
            return new Client(nativeApi, result.client());
        }
    }

    private static SmithyNativeApi loadNativeApi() {
        String configured = System.getenv("OPENKACHE_CLIENT_NATIVE");
        try {
            return Native.load(
                configured == null || configured.isBlank()
                    ? "openkache_client_core"
                    : configured,
                SmithyNativeApi.class);
        } catch (UnsatisfiedLinkError error) {
            throw new OpenKacheClientException("failed to load OpenKache native client", error);
        }
    }

    @Override
    public <T> CompletionStage<T> smithySubmit(
        java.util.function.Supplier<T> operation) {
        return CompletableFuture.supplyAsync(operation, executor);
    }

    @Override
    public NativeResult smithyExecute(
        int operation,
        byte[] applicationKey,
        byte[] value,
        int setCondition,
        long ttlMilliseconds) {
        synchronized (lifecycle) {
            ensureOpen();
            try (
                NativeBuffer key = new NativeBuffer(applicationKey);
                NativeBuffer payload = new NativeBuffer(value)
            ) {
                return readResult(
                    nativeApi,
                    nativeApi.openkache_client_execute(
                        handle,
                        operation,
                        key.pointer(),
                        key.length(),
                        payload.pointer(),
                        payload.length(),
                        setCondition,
                        (byte) (ttlMilliseconds == 0 ? 0 : 1),
                        ttlMilliseconds),
                    false);
            }
        }
    }

    @Override
    public NativeResult smithyExecuteScoped(
        int operation,
        long namespaceId,
        byte[] itemId,
        byte[] value,
        int setFlags,
        long ttlMilliseconds) {
        validateItemId(itemId, operation);
        synchronized (lifecycle) {
            ensureOpen();
            try (
                NativeBuffer item = new NativeBuffer(itemId);
                NativeBuffer payload = new NativeBuffer(value)
            ) {
                return readResult(
                    nativeApi,
                    nativeApi.openkache_client_execute_scoped(
                        handle,
                        operation,
                        namespaceId,
                        item.pointer(),
                        item.length(),
                        payload.pointer(),
                        payload.length(),
                        (byte) setFlags,
                        ttlMilliseconds),
                    false);
            }
        }
    }

    @Override
    public NativeResult smithyNamespaceOpen(
        byte[] name,
        boolean createIfMissing,
        int policyFlags,
        long ttlMilliseconds) {
        try (NativeBuffer nameBuffer = new NativeBuffer(name)) {
            synchronized (lifecycle) {
                ensureOpen();
                return readResult(
                    nativeApi,
                    nativeApi.openkache_client_namespace_open(
                        handle,
                        nameBuffer.pointer(),
                        nameBuffer.length(),
                        (byte) (createIfMissing ? 1 : 0),
                        (byte) policyFlags,
                        ttlMilliseconds),
                    false);
            }
        }
    }

    @Override
    public NativeResult smithyNamespaceUpdatePolicy(
        long namespaceId,
        long expectedRevision,
        int policyFlags,
        long ttlMilliseconds) {
        synchronized (lifecycle) {
            ensureOpen();
            return readResult(
                nativeApi,
                nativeApi.openkache_client_namespace_update_policy(
                    handle,
                    namespaceId,
                    expectedRevision,
                    (byte) policyFlags,
                    ttlMilliseconds),
                false);
        }
    }

    @Override
    public NativeResult smithyNamespaceDelete(long namespaceId, long expectedRevision) {
        synchronized (lifecycle) {
            ensureOpen();
            return readResult(
                nativeApi,
                nativeApi.openkache_client_namespace_delete(
                    handle,
                    namespaceId,
                    expectedRevision),
                false);
        }
    }

    private void validateItemId(byte[] itemId, int operation) {
        if (SmithyContract.operationRequiresItemId(operation)
            && itemId.length != SmithyContract.ITEM_ID_BYTES) {
            throw new IllegalArgumentException("itemId must contain exactly "
                + SmithyContract.ITEM_ID_BYTES + " bytes");
        }
        if (itemId.length != 0 && !SmithyContract.operationSupportsScoped(operation)) {
            throw new IllegalArgumentException("operation does not accept an itemId");
        }
    }

    @Override
    public NamespaceDescriptor smithyDecodeDescriptor(byte[] payload) {
        try (NativeBuffer buffer = new NativeBuffer(payload)) {
            SmithyNativeDescriptor nativeDescriptor = new SmithyNativeDescriptor();
            int status = nativeApi.openkache_client_namespace_descriptor_decode(
                buffer.pointer(),
                buffer.length(),
                nativeDescriptor);
            if (status != SmithyContract.DESCRIPTOR_DECODE_OK) {
                throw new OpenKacheClientException("native client returned an invalid namespace descriptor");
            }
            nativeDescriptor.read();
            ExpirationDefault expiration = nativeDescriptor.defaultExpiration
                == SmithyContract.DEFAULT_EXPIRATION_FIXED_TTL
                ? ExpirationDefault.FIXED_TTL
                : ExpirationDefault.NO_EXPIRY;
            EvictionDefault eviction = nativeDescriptor.defaultEviction
                == SmithyContract.DEFAULT_EVICTION_PROTECTED
                ? EvictionDefault.EVICTION_PROTECTED
                : EvictionDefault.EVICTABLE;
            NamespacePolicy policy = new NamespacePolicy(
                expiration,
                expiration == ExpirationDefault.FIXED_TTL
                    ? nativeDescriptor.defaultTtlMs
                    : null,
                nativeDescriptor.expirationOverride == SmithyContract.OVERRIDE_ALLOWED
                    ? OverridePolicy.ALLOWED
                    : OverridePolicy.DISALLOWED,
                eviction,
                nativeDescriptor.evictionOverride == SmithyContract.OVERRIDE_ALLOWED
                    ? OverridePolicy.ALLOWED
                    : OverridePolicy.DISALLOWED);
            return new NamespaceDescriptor(
                nativeDescriptor.namespaceId,
                nativeDescriptor.revision,
                policy);
        }
    }

    @Override
    public String smithyDecodeUtf8(byte[] payload, String operation) {
        try {
            CharBuffer decoded = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(payload));
            return decoded.toString();
        } catch (CharacterCodingException error) {
            throw new OpenKacheClientException(operation + " response is not valid UTF-8", error);
        }
    }

    private void ensureOpen() {
        if (closed || handle == null) {
            throw new OpenKacheClientException("OpenKache client is closed");
        }
    }

    @Override
    public void close() {
        synchronized (lifecycle) {
            if (closed) {
                return;
            }
            closed = true;
        }
        executor.shutdown();
        try {
            if (!executor.awaitTermination(30, TimeUnit.SECONDS)) {
                executor.shutdownNow();
            }
        } catch (InterruptedException error) {
            executor.shutdownNow();
            Thread.currentThread().interrupt();
        }
        synchronized (lifecycle) {
            if (handle != null) {
                nativeApi.openkache_client_free(handle);
                handle = null;
            }
        }
    }

    private static NativeResult readResult(
        SmithyNativeApi nativeApi,
        Pointer result,
        boolean takeClient) {
        if (result == null) {
            throw new OpenKacheClientException("native client returned a null result");
        }
        try {
            int kind = nativeApi.openkache_client_result_kind(result);
            long length = nativeApi.openkache_client_result_data_length(result);
            if (length < 0 || length > Integer.MAX_VALUE) {
                throw new OpenKacheClientException("native client returned an oversized payload");
            }
            Pointer data = nativeApi.openkache_client_result_data(result);
            byte[] payload = length == 0
                ? new byte[0]
                : Objects.requireNonNull(data, "native client returned a null payload")
                    .getByteArray(0, (int) length);
            Pointer client = takeClient
                ? nativeApi.openkache_client_result_take_client(result)
                : null;
            if (kind == SmithyContract.RESULT_ERROR) {
                throw new OpenKacheClientException(
                    new String(payload, StandardCharsets.UTF_8).isEmpty()
                        ? "native client operation failed"
                        : new String(payload, StandardCharsets.UTF_8));
            }
            return new NativeResult(kind, payload, client);
        } finally {
            nativeApi.openkache_client_result_free(result);
        }
    }

}
