package io.openkache.client;

import com.sun.jna.Library;
import com.sun.jna.Memory;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import com.sun.jna.Structure;
import io.openkache.client.generated_local.SmithyContract;

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
public final class EchoClient implements OpenKacheClient, AutoCloseable {
    private interface NativeApi extends Library {
        int openkache_client_abi_version();

        Pointer openkache_client_connect(
            Pointer address,
            long addressLength,
            Pointer serverName,
            long serverNameLength,
            Pointer certificate,
            long certificateLength,
            Pointer dataProtectionKey,
            long dataProtectionKeyLength,
            byte compressionEnabled,
            int compressionLevel,
            long minimumInputSize,
            long minimumSavings,
            long connectTimeoutMilliseconds,
            long requestTimeoutMilliseconds);

        Pointer openkache_client_execute(
            Pointer client,
            int operation,
            Pointer applicationKey,
            long applicationKeyLength,
            Pointer value,
            long valueLength,
            int setCondition,
            byte ttlEnabled,
            long ttlMilliseconds);

        Pointer openkache_client_execute_scoped(
            Pointer client,
            int operation,
            long namespaceId,
            Pointer itemId,
            long itemIdLength,
            Pointer value,
            long valueLength,
            byte setFlags,
            long ttlMilliseconds);

        Pointer openkache_client_namespace_open(
            Pointer client,
            Pointer name,
            long nameLength,
            byte createIfMissing,
            byte policyFlags,
            long ttlMilliseconds);

        Pointer openkache_client_namespace_update_policy(
            Pointer client,
            long namespaceId,
            long expectedRevision,
            byte policyFlags,
            long ttlMilliseconds);

        Pointer openkache_client_namespace_delete(
            Pointer client,
            long namespaceId,
            long expectedRevision);

        int openkache_client_namespace_descriptor_decode(
            Pointer payload,
            long payloadLength,
            NativeDescriptor output);

        int openkache_client_result_kind(Pointer result);

        Pointer openkache_client_result_data(Pointer result);

        long openkache_client_result_data_length(Pointer result);

        Pointer openkache_client_result_take_client(Pointer result);

        void openkache_client_result_free(Pointer result);

        void openkache_client_free(Pointer client);
    }

    @Structure.FieldOrder({
        "namespaceId",
        "revision",
        "defaultTtlMs",
        "defaultExpiration",
        "expirationOverride",
        "defaultEviction",
        "evictionOverride"
    })
    public static final class NativeDescriptor extends Structure {
        public long namespaceId;
        public long revision;
        public long defaultTtlMs;
        public int defaultExpiration;
        public int expirationOverride;
        public int defaultEviction;
        public int evictionOverride;
    }

    private record NativeResult(int kind, byte[] payload, Pointer client) {}

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

    private final NativeApi nativeApi;
    private final ExecutorService executor;
    private final Object lifecycle = new Object();
    private Pointer handle;
    private boolean closed;

    private EchoClient(NativeApi nativeApi, Pointer handle) {
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
    public static EchoClient connect(
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

        NativeApi nativeApi = loadNativeApi();
        if (nativeApi.openkache_client_abi_version() != SmithyContract.ABI_VERSION) {
            throw new EchoClientException("unsupported OpenKache native ABI version");
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
                throw new EchoClientException("native client did not return a connected handle");
            }
            return new EchoClient(nativeApi, result.client());
        }
    }

    private static NativeApi loadNativeApi() {
        String configured = System.getenv("OPENKACHE_CLIENT_NATIVE");
        try {
            return Native.load(
                configured == null || configured.isBlank()
                    ? "openkache_client_core"
                    : configured,
                NativeApi.class);
        } catch (UnsatisfiedLinkError error) {
            throw new EchoClientException("failed to load OpenKache native client", error);
        }
    }

    @Override
    public CompletionStage<PingOutput> ping(PingInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result = execute(
                SmithyContract.OPERATION_PING,
                new byte[0],
                new byte[0],
                0,
                0);
            requireKind(result, SmithyContract.RESULT_OK, "PING");
            return new PingOutput();
        });
    }

    @Override
    public CompletionStage<EchoOutput> echo(EchoInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> new EchoOutput(echoText(input.message())));
    }

    /** Sends one message and decodes the strict UTF-8 response. */
    public CompletionStage<String> echo(String message) {
        return echo(new EchoInput(Objects.requireNonNull(message, "message")))
            .thenApply(EchoOutput::message);
    }

    @Override
    public CompletionStage<GetOutput> get(GetInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result = executeScoped(
                SmithyContract.OPERATION_GET,
                input.namespaceId(),
                input.itemId(),
                new byte[0],
                0,
                0);
            if (result.kind() == SmithyContract.RESULT_NOT_FOUND) {
                return new GetOutput(null);
            }
            requireKind(result, SmithyContract.RESULT_VALUE, "GET");
            return new GetOutput(result.payload());
        });
    }

    @Override
    public CompletionStage<SetOutput> set(SetInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            SetFlags flags = setFlags(input);
            NativeResult result = executeScoped(
                SmithyContract.OPERATION_SET,
                input.namespaceId(),
                input.itemId(),
                input.value(),
                flags.flags(),
                flags.ttlMilliseconds());
            SetOutcome outcome;
            if (result.kind() == SmithyContract.RESULT_CREATED) {
                outcome = SetOutcome.CREATED;
            } else if (result.kind() == SmithyContract.RESULT_REPLACED) {
                outcome = SetOutcome.REPLACED;
            } else if (result.kind() == SmithyContract.RESULT_NOT_STORED) {
                outcome = SetOutcome.NOT_STORED;
            } else {
                throw unexpectedKind("SET", result.kind());
            }
            return new SetOutput(outcome);
        });
    }

    @Override
    public CompletionStage<DeleteOutput> delete(DeleteInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result = executeScoped(
                SmithyContract.OPERATION_DELETE,
                input.namespaceId(),
                input.itemId(),
                new byte[0],
                0,
                0);
            if (result.kind() == SmithyContract.RESULT_DELETED) {
                return new DeleteOutput(true);
            }
            if (result.kind() == SmithyContract.RESULT_NOT_DELETED) {
                return new DeleteOutput(false);
            }
            throw unexpectedKind("DELETE", result.kind());
        });
    }

    @Override
    public CompletionStage<StatsOutput> stats(StatsInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result = executeScoped(
                SmithyContract.OPERATION_STATS,
                input.namespaceId(),
                new byte[0],
                new byte[0],
                0,
                0);
            requireKind(result, SmithyContract.RESULT_VALUE, "STATS");
            return new StatsOutput(decodeUtf8(result.payload(), "STATS"));
        });
    }

    @Override
    public CompletionStage<SyncOutput> sync(SyncInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result = executeScoped(
                SmithyContract.OPERATION_SYNC,
                input.namespaceId(),
                new byte[0],
                new byte[0],
                0,
                0);
            requireKind(result, SmithyContract.RESULT_OK, "SYNC");
            return new SyncOutput();
        });
    }

    @Override
    public CompletionStage<NamespaceOpenOutput> namespaceOpen(NamespaceOpenInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            byte[] name = input.name().getBytes(StandardCharsets.UTF_8);
            if (name.length > SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                throw new EchoClientException("namespace name exceeds protocol limit");
            }
            PolicyFlags policy = policyFlags(input.policy(), input.createIfMissing());
            try (NativeBuffer nameBuffer = new NativeBuffer(name)) {
                NativeResult result;
                synchronized (lifecycle) {
                    ensureOpen();
                    result = readResult(
                        nativeApi,
                        nativeApi.openkache_client_namespace_open(
                            handle,
                            nameBuffer.pointer(),
                            nameBuffer.length(),
                            (byte) (input.createIfMissing() ? 1 : 0),
                            (byte) policy.flags(),
                            policy.ttlMilliseconds()),
                        false);
                }
                boolean created = result.kind() == SmithyContract.RESULT_CREATED;
                if (!created && result.kind() != SmithyContract.RESULT_OK) {
                    throw unexpectedKind("NAMESPACE_OPEN", result.kind());
                }
                return new NamespaceOpenOutput(
                    decodeDescriptor(result.payload()),
                    created);
            }
        });
    }

    @Override
    public CompletionStage<NamespaceUpdatePolicyOutput> namespaceUpdatePolicy(
        NamespaceUpdatePolicyInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            PolicyFlags policy = policyFlags(input.policy(), true);
            NativeResult result;
            synchronized (lifecycle) {
                ensureOpen();
                result = readResult(
                    nativeApi,
                    nativeApi.openkache_client_namespace_update_policy(
                        handle,
                        input.namespaceId(),
                        input.expectedRevision(),
                        (byte) policy.flags(),
                        policy.ttlMilliseconds()),
                    false);
            }
            requireKind(result, SmithyContract.RESULT_VALUE, "NAMESPACE_UPDATE_POLICY");
            return new NamespaceUpdatePolicyOutput(decodeDescriptor(result.payload()));
        });
    }

    @Override
    public CompletionStage<NamespaceDeleteOutput> namespaceDelete(NamespaceDeleteInput input) {
        Objects.requireNonNull(input, "input");
        return submit(() -> {
            NativeResult result;
            synchronized (lifecycle) {
                ensureOpen();
                result = readResult(
                    nativeApi,
                    nativeApi.openkache_client_namespace_delete(
                        handle,
                        input.namespaceId(),
                        input.expectedRevision()),
                    false);
            }
            requireKind(result, SmithyContract.RESULT_OK, "NAMESPACE_DELETE");
            return new NamespaceDeleteOutput();
        });
    }

    private CompletionStage<Void> submit(Runnable operation) {
        return CompletableFuture.runAsync(operation, executor);
    }

    private <T> CompletionStage<T> submit(java.util.function.Supplier<T> operation) {
        return CompletableFuture.supplyAsync(operation, executor);
    }

    private String echoText(String message) {
        byte[] bytes = execute(
            SmithyContract.OPERATION_ECHO,
            new byte[0],
            message.getBytes(StandardCharsets.UTF_8),
            0,
            0).payload();
        return decodeUtf8(bytes, "ECHO");
    }

    private NativeResult execute(
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

    private NativeResult executeScoped(
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

    private Pointer requireHandle() {
        synchronized (lifecycle) {
            ensureOpen();
            return handle;
        }
    }

    private void validateItemId(byte[] itemId, int operation) {
        if ((operation == SmithyContract.OPERATION_GET
                || operation == SmithyContract.OPERATION_SET
                || operation == SmithyContract.OPERATION_DELETE)
            && itemId.length != SmithyContract.ITEM_ID_BYTES) {
            throw new IllegalArgumentException("itemId must contain exactly "
                + SmithyContract.ITEM_ID_BYTES + " bytes");
        }
        if (itemId.length != 0 && operation != SmithyContract.OPERATION_GET
            && operation != SmithyContract.OPERATION_SET
            && operation != SmithyContract.OPERATION_DELETE) {
            throw new IllegalArgumentException("operation does not accept an itemId");
        }
    }

    private SetFlags setFlags(SetInput input) {
        int flags = switch (input.condition() == null ? SetCondition.ANY : input.condition()) {
            case ANY -> SmithyContract.SET_CONDITION_ANY;
            case IF_ABSENT -> SmithyContract.SET_CONDITION_IF_ABSENT;
            case IF_PRESENT -> SmithyContract.SET_CONDITION_IF_PRESENT;
        };
        ExpirationMode expiration = input.expirationMode() == null
            ? (input.ttlMilliseconds() == null
                ? ExpirationMode.INHERIT
                : ExpirationMode.EXPLICIT_TTL)
            : input.expirationMode();
        switch (expiration) {
            case INHERIT -> {
                if (input.ttlMilliseconds() != null) {
                    throw new IllegalArgumentException("INHERIT cannot carry a TTL");
                }
                flags |= SmithyContract.SET_INHERIT_EXPIRATION_BITS;
            }
            case NO_EXPIRY -> {
                if (input.ttlMilliseconds() != null) {
                    throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
                }
                flags |= SmithyContract.SET_NO_EXPIRY_BITS;
            }
            case EXPLICIT_TTL -> {
                if (input.ttlMilliseconds() == null || input.ttlMilliseconds() <= 0) {
                    throw new IllegalArgumentException("EXPLICIT_TTL requires a positive TTL");
                }
                flags |= SmithyContract.SET_EXPLICIT_TTL_BITS;
            }
        }
        EvictionMode eviction = input.evictionMode() == null
            ? EvictionMode.INHERIT
            : input.evictionMode();
        flags |= switch (eviction) {
            case INHERIT -> SmithyContract.SET_INHERIT_EVICTION_BITS;
            case EVICTABLE -> SmithyContract.SET_EVICTABLE_BITS;
            case EVICTION_PROTECTED -> SmithyContract.SET_EVICTION_PROTECTED_BITS;
        };
        if (input.value().length > SmithyContract.MAX_VALUE_BYTES) {
            throw new IllegalArgumentException("value exceeds protocol limit");
        }
        return new SetFlags(flags, input.ttlMilliseconds() == null ? 0 : input.ttlMilliseconds());
    }

    private PolicyFlags policyFlags(NamespacePolicy policy, boolean required) {
        if (required && policy == null) {
            throw new IllegalArgumentException("namespace policy is required");
        }
        if (!required && policy != null) {
            throw new IllegalArgumentException("namespace policy requires createIfMissing");
        }
        if (policy == null) {
            return new PolicyFlags(0, 0);
        }
        int flags = switch (policy.defaultExpiration()) {
            case NO_EXPIRY -> SmithyContract.POLICY_NO_EXPIRY_BITS;
            case FIXED_TTL -> SmithyContract.POLICY_FIXED_TTL_BITS;
        };
        long ttl = policy.defaultTtlMilliseconds() == null ? 0 : policy.defaultTtlMilliseconds();
        if (policy.defaultExpiration() == ExpirationDefault.FIXED_TTL && ttl <= 0) {
            throw new IllegalArgumentException("FIXED_TTL requires a positive TTL");
        }
        if (policy.defaultExpiration() == ExpirationDefault.NO_EXPIRY && ttl != 0) {
            throw new IllegalArgumentException("NO_EXPIRY cannot carry a TTL");
        }
        if (policy.expirationOverride() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG;
        }
        if (policy.defaultEviction() == EvictionDefault.EVICTION_PROTECTED) {
            flags |= SmithyContract.POLICY_EVICTION_PROTECTED_FLAG;
        }
        if (policy.evictionOverride() == OverridePolicy.ALLOWED) {
            flags |= SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG;
        }
        return new PolicyFlags(flags, ttl);
    }

    private NamespaceDescriptor decodeDescriptor(byte[] payload) {
        try (NativeBuffer buffer = new NativeBuffer(payload)) {
            NativeDescriptor nativeDescriptor = new NativeDescriptor();
            int status = nativeApi.openkache_client_namespace_descriptor_decode(
                buffer.pointer(),
                buffer.length(),
                nativeDescriptor);
            if (status != SmithyContract.DESCRIPTOR_DECODE_OK) {
                throw new EchoClientException("native client returned an invalid namespace descriptor");
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

    private static String decodeUtf8(byte[] payload, String operation) {
        try {
            CharBuffer decoded = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(payload));
            return decoded.toString();
        } catch (CharacterCodingException error) {
            throw new EchoClientException(operation + " response is not valid UTF-8", error);
        }
    }

    private static void requireKind(NativeResult result, int expected, String operation) {
        if (result.kind() != expected) {
            throw unexpectedKind(operation, result.kind());
        }
    }

    private static EchoClientException unexpectedKind(String operation, int kind) {
        return new EchoClientException(operation + " returned unexpected native result " + kind);
    }

    private void ensureOpen() {
        if (closed || handle == null) {
            throw new EchoClientException("OpenKache client is closed");
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
        NativeApi nativeApi,
        Pointer result,
        boolean takeClient) {
        if (result == null) {
            throw new EchoClientException("native client returned a null result");
        }
        try {
            int kind = nativeApi.openkache_client_result_kind(result);
            long length = nativeApi.openkache_client_result_data_length(result);
            if (length < 0 || length > Integer.MAX_VALUE) {
                throw new EchoClientException("native client returned an oversized payload");
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
                throw new EchoClientException(
                    new String(payload, StandardCharsets.UTF_8).isEmpty()
                        ? "native client operation failed"
                        : new String(payload, StandardCharsets.UTF_8));
            }
            return new NativeResult(kind, payload, client);
        } finally {
            nativeApi.openkache_client_result_free(result);
        }
    }

    private record SetFlags(int flags, long ttlMilliseconds) {}

    private record PolicyFlags(int flags, long ttlMilliseconds) {}
}
