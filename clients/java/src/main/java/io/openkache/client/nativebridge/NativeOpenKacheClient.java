package io.openkache.client.nativebridge;

import io.openkache.client.generated.SmithyApi;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.Arrays;
import java.util.Objects;
import java.util.concurrent.CancellationException;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.Executor;
import java.util.concurrent.ForkJoinPool;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Function;

/**
 * CompletableFuture-based Java binding over the versioned OpenKache C ABI.
 *
 * <p>The native core owns QUIC, TLS, retries, value protection, and request
 * cancellation. Java marshals byte arrays, assigns request IDs, and keeps the
 * opaque client handle alive.</p>
 */
public final class NativeOpenKacheClient implements AutoCloseable {
    private static final int ITEM_ID_BYTES =
            io.openkache.client.generated.SmithyContract.ITEM_ID_BYTES;
    private static final int MUTATION_ID_BYTES =
            io.openkache.client.generated.SmithyContract.MUTATION_ID_BYTES;

    private static final int RESULT_ERROR = io.openkache.client.generated.SmithyContract.FFI_RESULT_Error;
    private static final int RESULT_OK = io.openkache.client.generated.SmithyContract.FFI_RESULT_Ok;
    private static final int RESULT_VALUE = io.openkache.client.generated.SmithyContract.FFI_RESULT_Value;
    private static final int RESULT_NOT_FOUND = io.openkache.client.generated.SmithyContract.FFI_RESULT_NotFound;
    private static final int RESULT_CREATED = io.openkache.client.generated.SmithyContract.FFI_RESULT_Created;
    private static final int RESULT_REPLACED = io.openkache.client.generated.SmithyContract.FFI_RESULT_Replaced;
    private static final int RESULT_DELETED = io.openkache.client.generated.SmithyContract.FFI_RESULT_Deleted;
    private static final int RESULT_NOT_DELETED = io.openkache.client.generated.SmithyContract.FFI_RESULT_NotDeleted;
    private static final int RESULT_CONNECTED = io.openkache.client.generated.SmithyContract.FFI_RESULT_Connected;
    private static final int RESULT_NOT_STORED = io.openkache.client.generated.SmithyContract.FFI_RESULT_NotStored;

    private static final int OP_PING = io.openkache.client.generated.SmithyContract.OPCODE_Ping;
    private static final int OP_GET = io.openkache.client.generated.SmithyContract.OPCODE_Get;
    private static final int OP_SET = io.openkache.client.generated.SmithyContract.OPCODE_Set;
    private static final int OP_DELETE = io.openkache.client.generated.SmithyContract.OPCODE_Delete;
    private static final int OP_STATS = io.openkache.client.generated.SmithyContract.OPCODE_Stats;
    private static final int OP_SYNC = io.openkache.client.generated.SmithyContract.OPCODE_Sync;
    private static final int FFI_OPERATION_GET_JSON =
            io.openkache.client.generated.SmithyContract.FFI_OPERATION_GetJson;
    private static final int FFI_OPERATION_SET_JSON =
            io.openkache.client.generated.SmithyContract.FFI_OPERATION_SetJson;
    private static final int FFI_OPERATION_RECONNECT =
            io.openkache.client.generated.SmithyContract.FFI_OPERATION_Reconnect;

    private static final int SET_CONDITION_NONE =
            io.openkache.client.generated.SmithyContract.FFI_SET_CONDITION_None;
    private static final int SET_CONDITION_IF_ABSENT =
            io.openkache.client.generated.SmithyContract.FFI_SET_CONDITION_IfAbsent;
    private static final int SET_CONDITION_IF_PRESENT =
            io.openkache.client.generated.SmithyContract.FFI_SET_CONDITION_IfPresent;

    private static final int FFI_ERROR_CANCELLED =
            io.openkache.client.generated.SmithyContract.FFI_ERROR_Cancelled;
    private static final int FFI_ABI_VERSION =
            io.openkache.client.generated.SmithyContract.FFI_ABI_VERSION;
    private static final int ERROR_METADATA_BYTES =
            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_BYTES;
    private static final int METRICS_SNAPSHOT_BYTES =
            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_BYTES;
    private static final SecureRandom RANDOM = new SecureRandom();

    /*
     * FfiConnectOptions is a C repr(C) structure. OpenKache's supported
     * native artifacts use a 64-bit size_t, so the Java FFM layout uses
     * JAVA_LONG for size_t fields. The generated Smithy layout keeps these
     * offsets in the versioned contract, independent of Java record ordering.
     */
    private static final long CONNECT_ADDRESS =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_ADDRESS_OFFSET;
    private static final long CONNECT_ADDRESS_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_ADDRESS_LENGTH_OFFSET;
    private static final long CONNECT_SERVER_NAME =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_SERVER_NAME_OFFSET;
    private static final long CONNECT_SERVER_NAME_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_SERVER_NAME_LENGTH_OFFSET;
    private static final long CONNECT_CERTIFICATE =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CERTIFICATE_OFFSET;
    private static final long CONNECT_CERTIFICATE_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CERTIFICATE_LENGTH_OFFSET;
    private static final long CONNECT_CLIENT_CERTIFICATE =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_OFFSET;
    private static final long CONNECT_CLIENT_CERTIFICATE_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CLIENT_CERTIFICATE_CHAIN_LENGTH_OFFSET;
    private static final long CONNECT_CLIENT_PRIVATE_KEY =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CLIENT_PRIVATE_KEY_OFFSET;
    private static final long CONNECT_CLIENT_PRIVATE_KEY_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_CLIENT_PRIVATE_KEY_LENGTH_OFFSET;
    private static final long CONNECT_DATA_PROTECTION_KEY =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_DATA_PROTECTION_KEY_OFFSET;
    private static final long CONNECT_DATA_PROTECTION_KEY_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_DATA_PROTECTION_KEY_LENGTH_OFFSET;
    private static final long CONNECT_PREVIOUS_KEYS =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_OFFSET;
    private static final long CONNECT_PREVIOUS_KEYS_LENGTH =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEYS_LENGTH_OFFSET;
    private static final long CONNECT_PREVIOUS_KEY_COUNT =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_PREVIOUS_DATA_PROTECTION_KEY_COUNT_OFFSET;
    private static final long CONNECT_COMPRESSION_ENABLED =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_COMPRESSION_ENABLED_OFFSET;
    private static final long CONNECT_COMPRESSION_LEVEL =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_COMPRESSION_LEVEL_OFFSET;
    private static final long CONNECT_MINIMUM_INPUT_SIZE =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_MINIMUM_INPUT_SIZE_OFFSET;
    private static final long CONNECT_MINIMUM_SAVINGS =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_MINIMUM_SAVINGS_OFFSET;
    private static final long CONNECT_ENCRYPTION =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_ENCRYPTION_OFFSET;
    private static final long CONNECT_TIMEOUT =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_TIMEOUT_OFFSET;
    private static final long CONNECT_REQUEST_TIMEOUT =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_REQUEST_TIMEOUT_OFFSET;
    private static final long CONNECT_RETRY_MAX_ATTEMPTS =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_RETRY_MAX_ATTEMPTS_OFFSET;
    private static final long CONNECT_MAX_IN_FLIGHT =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_MAX_IN_FLIGHT_OFFSET;
    private static final long CONNECT_OPTIONS_BYTES =
            io.openkache.client.generated.SmithyContract.FFI_CONNECT_OPTIONS_BYTES;

    private final Arena arena;
    private final MemorySegment client;
    private final MethodHandle execute;
    private final MethodHandle executeMutation;
    private final MethodHandle executeRaw;
    private final MethodHandle executeRawMutation;
    private final MethodHandle cancel;
    private final MethodHandle metricsSnapshot;
    private final MethodHandle resultKind;
    private final MethodHandle resultData;
    private final MethodHandle resultDataLength;
    private final MethodHandle resultErrorMetadata;
    private final MethodHandle resultFree;
    private final MethodHandle clientFree;
    private final Executor executor;
    private final AtomicLong nextRequestId = new AtomicLong(1);
    private final Object lifecycle = new Object();
    private int activeCalls;
    private boolean closed;

    private NativeOpenKacheClient(
            Arena arena,
            MemorySegment client,
            MethodHandle execute,
            MethodHandle executeMutation,
            MethodHandle executeRaw,
            MethodHandle executeRawMutation,
            MethodHandle cancel,
            MethodHandle metricsSnapshot,
            MethodHandle resultKind,
            MethodHandle resultData,
            MethodHandle resultDataLength,
            MethodHandle resultErrorMetadata,
            MethodHandle resultFree,
            MethodHandle clientFree,
            Executor executor) {
        this.arena = arena;
        this.client = client;
        this.execute = execute;
        this.executeMutation = executeMutation;
        this.executeRaw = executeRaw;
        this.executeRawMutation = executeRawMutation;
        this.cancel = cancel;
        this.metricsSnapshot = metricsSnapshot;
        this.resultKind = resultKind;
        this.resultData = resultData;
        this.resultDataLength = resultDataLength;
        this.resultErrorMetadata = resultErrorMetadata;
        this.resultFree = resultFree;
        this.clientFree = clientFree;
        this.executor = executor;
    }

    /** Connects asynchronously using the shared native core. */
    public static CompletableFuture<NativeOpenKacheClient> connectAsync(Options options) {
        return connectAsync(options, ForkJoinPool.commonPool());
    }

    /** Connects asynchronously on a caller-selected executor. */
    public static CompletableFuture<NativeOpenKacheClient> connectAsync(
            Options options,
            Executor executor) {
        Objects.requireNonNull(options, "options");
        Objects.requireNonNull(executor, "executor");
        return CompletableFuture.supplyAsync(() -> connect(options, executor), executor);
    }

    private static NativeOpenKacheClient connect(Options options, Executor executor) {
        options.validate();
        Arena arena = Arena.ofShared();
        boolean retain_arena = false;
        byte[] certificate_bytes = new byte[0];
        byte[] identity_certificate_bytes = new byte[0];
        byte[] identity_private_key_bytes = new byte[0];
        byte[] data_protection_key_bytes = new byte[0];
        byte[] previous_keys = new byte[0];
        MemorySegment certificate = MemorySegment.NULL;
        MemorySegment identity_certificate = MemorySegment.NULL;
        MemorySegment identity_private_key = MemorySegment.NULL;
        MemorySegment data_protection_key = MemorySegment.NULL;
        MemorySegment previous_data_protection_keys = MemorySegment.NULL;
        try {
            SymbolLookup lookup = lookup(arena);
            MethodHandle abiVersion = downcall(
                    lookup,
                    "openkache_client_abi_version",
                    FunctionDescriptor.of(ValueLayout.JAVA_INT));
            int nativeAbiVersion = (int) abiVersion.invokeExact();
            if (nativeAbiVersion != FFI_ABI_VERSION) {
                throw new OpenKacheException(
                        "unsupported native client ABI version " + nativeAbiVersion);
            }
            MethodHandle connect = downcall(
                    lookup,
                    "openkache_client_connect_with_options",
                    FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS));
            MethodHandle resultKind = downcall(
                    lookup,
                    "openkache_client_result_kind",
                    FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS));
            MethodHandle resultData = downcall(
                    lookup,
                    "openkache_client_result_data",
                    FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS));
            MethodHandle resultDataLength = downcall(
                    lookup,
                    "openkache_client_result_data_length",
                    FunctionDescriptor.of(ValueLayout.JAVA_LONG, ValueLayout.ADDRESS));
            MethodHandle resultErrorMetadata = downcall(
                    lookup,
                    "openkache_client_result_error_metadata",
                    FunctionDescriptor.of(
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS));
            MethodHandle resultFree = downcall(
                    lookup,
                    "openkache_client_result_free",
                    FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));
            MethodHandle takeClient = downcall(
                    lookup,
                    "openkache_client_result_take_client",
                    FunctionDescriptor.of(ValueLayout.ADDRESS, ValueLayout.ADDRESS));
            MethodHandle clientFree = downcall(
                    lookup,
                    "openkache_client_free",
                    FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));
            MethodHandle execute = downcall(
                    lookup,
                    "openkache_client_execute_with_request_id",
                    FunctionDescriptor.of(
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.JAVA_LONG));
            MethodHandle executeMutation = downcall(
                    lookup,
                    "openkache_client_execute_with_request_id_and_mutation_id",
                    FunctionDescriptor.of(
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG));
            MethodHandle executeRaw = downcall(
                    lookup,
                    "openkache_client_execute_raw_with_request_id",
                    FunctionDescriptor.of(
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.JAVA_LONG));
            MethodHandle executeRawMutation = downcall(
                    lookup,
                    "openkache_client_execute_raw_with_request_id_and_mutation_id",
                    FunctionDescriptor.of(
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.JAVA_INT,
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.JAVA_LONG,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG));
            MethodHandle cancel = downcall(
                    lookup,
                    "openkache_client_cancel",
                    FunctionDescriptor.of(
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.ADDRESS,
                            ValueLayout.JAVA_LONG));
            MethodHandle metricsSnapshot = downcall(
                    lookup,
                    "openkache_client_metrics_snapshot",
                    FunctionDescriptor.of(
                            ValueLayout.JAVA_BYTE,
                            ValueLayout.ADDRESS,
                            ValueLayout.ADDRESS));

            MemorySegment optionsSegment = arena.allocate(CONNECT_OPTIONS_BYTES, 8);
            MemorySegment address = bytes(
                    arena,
                    options.address().getBytes(StandardCharsets.UTF_8));
            MemorySegment serverName = bytes(
                    arena,
                    options.serverName().getBytes(StandardCharsets.UTF_8));
            /*
             * FFM allocations are retained by the shared arena for the lifetime of a
             * connected client. The native connect call copies these buffers into
             * Rust-owned storage, so keep only zeroized allocations after it returns.
             * Clone the record fields first: zeroizing a temporary must not mutate an
             * Options value that a caller may reuse for another connection.
             */
            certificate_bytes = options.certificate().clone();
            identity_certificate_bytes = options.clientCertificateChain().clone();
            identity_private_key_bytes = options.clientPrivateKey().clone();
            data_protection_key_bytes = options.dataProtectionKey().clone();
            previous_keys = concatenate(options.previousDataProtectionKeys());
            certificate = bytes(arena, certificate_bytes);
            identity_certificate = bytes(arena, identity_certificate_bytes);
            identity_private_key = bytes(arena, identity_private_key_bytes);
            data_protection_key = bytes(arena, data_protection_key_bytes);
            previous_data_protection_keys = bytes(arena, previous_keys);
            writeConnectOptions(
                    optionsSegment,
                    address,
                    serverName,
                    certificate,
                    identity_certificate,
                    identity_private_key,
                    data_protection_key,
                    previous_data_protection_keys,
                    previous_keys.length,
                    options);

            MemorySegment result = (MemorySegment) connect.invokeExact(optionsSegment);
            if (result.equals(MemorySegment.NULL)) {
                throw new OpenKacheException("native connect returned a null result");
            }
            try {
                int kind = (int) resultKind.invokeExact(result);
                if (kind != RESULT_CONNECTED) {
                    throw readError(
                            result,
                            resultKind,
                            resultData,
                            resultDataLength,
                            resultErrorMetadata,
                            arena);
                }
                MemorySegment connected = (MemorySegment) takeClient.invokeExact(result);
                if (connected.equals(MemorySegment.NULL)) {
                    throw new OpenKacheException("native connect returned no client handle");
                }
                NativeOpenKacheClient client = new NativeOpenKacheClient(
                        arena,
                        connected,
                        execute,
                        executeMutation,
                        executeRaw,
                        executeRawMutation,
                        cancel,
                        metricsSnapshot,
                        resultKind,
                        resultData,
                        resultDataLength,
                        resultErrorMetadata,
                        resultFree,
                        clientFree,
                        executor);
                retain_arena = true;
                return client;
            } finally {
                resultFree.invokeExact(result);
            }
        } catch (Throwable error) {
            if (error instanceof RuntimeException runtime) {
                throw runtime;
            }
            throw new OpenKacheException("native connect failed", error);
        } finally {
            zeroize(certificate);
            zeroize(identity_certificate);
            zeroize(identity_private_key);
            zeroize(data_protection_key);
            zeroize(previous_data_protection_keys);
            zeroize(certificate_bytes);
            zeroize(identity_certificate_bytes);
            zeroize(identity_private_key_bytes);
            zeroize(data_protection_key_bytes);
            zeroize(previous_keys);
            if (!retain_arena) {
                arena.close();
            }
        }
    }

    /** Gets protected bytes, or {@code null} when absent. */
    public CompletableFuture<Void> ping() {
        return map(
                invoke(OP_PING, new byte[0], new byte[0], SetOptions.none(), false),
                result -> {
                    if (result.kind != RESULT_OK) {
                        throw unexpected("PING", result.kind);
                    }
                    return null;
                });
    }

    /** Gets protected bytes, or {@code null} when absent. */
    public CompletableFuture<byte[]> get(byte[] key) {
        return map(
                invoke(OP_GET, key, new byte[0], SetOptions.none(), false),
                result -> result.kind == RESULT_NOT_FOUND
                        ? null
                        : requireKind(result, RESULT_VALUE, "GET"));
    }

    /** Stores protected bytes with idempotent mutation options. */
    public CompletableFuture<SmithyApi.SetOutcome> set(
            byte[] key,
            byte[] value,
            SetOptions options) {
        SetOptions ownedOptions = mutationOptions(options);
        return map(
                invoke(OP_SET, key, value, ownedOptions, false),
                result -> switch (result.kind) {
                    case RESULT_CREATED -> SmithyApi.SetOutcome.Created;
                    case RESULT_REPLACED -> SmithyApi.SetOutcome.Replaced;
                    case RESULT_NOT_STORED -> SmithyApi.SetOutcome.NotStored;
                    default -> throw unexpected("SET", result.kind);
                });
    }

    /** Deletes protected bytes with an idempotent mutation token. */
    public CompletableFuture<Boolean> delete(byte[] key, SetOptions options) {
        SetOptions ownedOptions = mutationOptions(options);
        return map(
                invoke(OP_DELETE, key, new byte[0], ownedOptions, false),
                result -> switch (result.kind) {
                    case RESULT_DELETED -> true;
                    case RESULT_NOT_DELETED, RESULT_NOT_FOUND -> false;
                    default -> throw unexpected("DELETE", result.kind);
                });
    }

    /** Retrieves one canonical JSON document as UTF-8, or {@code null} when absent. */
    public CompletableFuture<String> getJson(byte[] key) {
        return map(
                invoke(FFI_OPERATION_GET_JSON, key, new byte[0], SetOptions.none(), false),
                result -> {
                    if (result.kind == RESULT_NOT_FOUND) {
                        return null;
                    }
                    return new String(requireKind(result, RESULT_VALUE, "GET_JSON"),
                            StandardCharsets.UTF_8);
                });
    }

    /** Stores one canonical JSON document supplied as UTF-8. */
    public CompletableFuture<SmithyApi.SetOutcome> setJson(
            byte[] key,
            String json,
            SetOptions options) {
        Objects.requireNonNull(json, "json");
        SetOptions ownedOptions = mutationOptions(options);
        return map(
                invoke(
                        FFI_OPERATION_SET_JSON,
                        key,
                        json.getBytes(StandardCharsets.UTF_8),
                        ownedOptions,
                        false),
                result -> switch (result.kind) {
                    case RESULT_CREATED -> SmithyApi.SetOutcome.Created;
                    case RESULT_REPLACED -> SmithyApi.SetOutcome.Replaced;
                    case RESULT_NOT_STORED -> SmithyApi.SetOutcome.NotStored;
                    default -> throw unexpected("SET_JSON", result.kind);
                });
    }

    /** Gets exact decrypted bytes for a 32-byte protocol item ID. */
    public CompletableFuture<byte[]> getRaw(byte[] itemId) {
        validateItemId(itemId);
        return map(
                invoke(OP_GET, itemId, new byte[0], SetOptions.none(), true),
                result -> result.kind == RESULT_NOT_FOUND
                        ? null
                        : requireKind(result, RESULT_VALUE, "RAW_GET"));
    }

    /** Stores exact bytes for a 32-byte protocol item ID. */
    public CompletableFuture<SmithyApi.SetOutcome> setRaw(
            byte[] itemId,
            byte[] value,
            SetOptions options) {
        validateItemId(itemId);
        SetOptions ownedOptions = mutationOptions(options);
        return map(
                invoke(OP_SET, itemId, value, ownedOptions, true),
                result -> switch (result.kind) {
                    case RESULT_CREATED -> SmithyApi.SetOutcome.Created;
                    case RESULT_REPLACED -> SmithyApi.SetOutcome.Replaced;
                    case RESULT_NOT_STORED -> SmithyApi.SetOutcome.NotStored;
                    default -> throw unexpected("RAW_SET", result.kind);
                });
    }

    /** Deletes an exact 32-byte protocol item ID. */
    public CompletableFuture<Boolean> deleteRaw(byte[] itemId, SetOptions options) {
        validateItemId(itemId);
        SetOptions ownedOptions = mutationOptions(options);
        return map(
                invoke(OP_DELETE, itemId, new byte[0], ownedOptions, true),
                result -> switch (result.kind) {
                    case RESULT_DELETED -> true;
                    case RESULT_NOT_DELETED, RESULT_NOT_FOUND -> false;
                    default -> throw unexpected("RAW_DELETE", result.kind);
                });
    }

    /** Retrieves server statistics as UTF-8 JSON. */
    public CompletableFuture<String> stats() {
        return map(
                invoke(OP_STATS, new byte[0], new byte[0], SetOptions.none(), false),
                result -> new String(requireKind(result, RESULT_VALUE, "STATS"),
                        StandardCharsets.UTF_8));
    }

    /** Waits for the server durability barrier. */
    public CompletableFuture<Void> sync() {
        return map(
                invoke(OP_SYNC, new byte[0], new byte[0], SetOptions.none(), false),
                result -> {
                    if (result.kind != RESULT_OK) {
                        throw unexpected("SYNC", result.kind);
                    }
                    return null;
                });
    }

    /** Returns a point-in-time native metrics snapshot. */
    public MetricsSnapshot metricsSnapshot() {
        MemorySegment handle = acquireClient();
        try (Arena callArena = Arena.ofConfined()) {
            MemorySegment snapshot = callArena.allocate(METRICS_SNAPSHOT_BYTES, 8);
            try {
                byte ok = (byte) metricsSnapshot.invokeExact(handle, snapshot);
                if (ok == 0) {
                    throw new OpenKacheException("native client did not return metrics");
                }
                return MetricsSnapshot.from(snapshot);
            } catch (OpenKacheException error) {
                throw error;
            } catch (Throwable error) {
                throw new OpenKacheException("native metrics snapshot failed", error);
            }
        } finally {
            releaseCall();
        }
    }

    /** Returns the native connection state discriminator. */
    public int connectionState() {
        // The ABI function is intentionally looked up lazily to keep the
        // constructor small and to permit older custom artifacts to fail
        // with a clear symbol error.
        MemorySegment handle = acquireClient();
        try {
            MethodHandle state = downcall(
                    lookup(arena),
                    "openkache_client_connection_state",
                    FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS));
            try {
                return (int) state.invokeExact(handle);
            } catch (Throwable error) {
                throw new OpenKacheException("native connection state failed", error);
            }
        } finally {
            releaseCall();
        }
    }

    /** Requests an explicit reconnect without replaying an operation. */
    public CompletableFuture<Void> reconnect() {
        return map(
                invoke(FFI_OPERATION_RECONNECT, new byte[0], new byte[0],
                        SetOptions.none(), false),
                result -> {
                    if (result.kind != RESULT_OK) {
                        throw unexpected("RECONNECT", result.kind);
                    }
                    return null;
                });
    }

    private CompletableFuture<Result> invoke(
            int operation,
            byte[] key,
            byte[] value,
            SetOptions options,
            boolean raw) {
        byte[] ownedKey = Objects.requireNonNull(key, "key").clone();
        byte[] ownedValue = Objects.requireNonNull(value, "value").clone();
        SetOptions ownedOptions = Objects.requireNonNull(options, "options");
        long requestId = nextRequestId();
        CancelableFuture<Result> future = new CancelableFuture<>(
                () -> cancel(requestId));
        try {
            executor.execute(() -> {
                if (future.isCancelled()) {
                    return;
                }
                try {
                    Result result = invokeNative(
                            requestId,
                            operation,
                            ownedKey,
                            ownedValue,
                            ownedOptions,
                            raw);
                    future.complete(result);
                } catch (Throwable error) {
                    future.completeExceptionally(error);
                }
            });
        } catch (RejectedExecutionException error) {
            future.completeExceptionally(
                    new OpenKacheException("native executor rejected operation", error));
        }
        return future;
    }

    private <T> CompletableFuture<T> map(
            CompletableFuture<Result> source,
            Function<Result, T> mapper) {
        CancelableFuture<T> target = new CancelableFuture<>(() -> source.cancel(true));
        source.whenComplete((result, error) -> {
            if (error != null) {
                if (error instanceof CancellationException
                        || error.getCause() instanceof CancellationException) {
                    target.cancel(false);
                } else {
                    target.completeExceptionally(unwrap(error));
                }
                return;
            }
            try {
                target.complete(mapper.apply(result));
            } catch (Throwable mappingError) {
                target.completeExceptionally(mappingError);
            }
        });
        return target;
    }

    private Result invokeNative(
            long requestId,
            int operation,
            byte[] key,
            byte[] value,
            SetOptions options,
            boolean raw) {
        MemorySegment handle = acquireClient();
        try (Arena callArena = Arena.ofConfined()) {
            MemorySegment keySegment = bytes(callArena, key);
            MemorySegment valueSegment = bytes(callArena, value);
            byte[] mutationId = options.mutationId();
            MemorySegment mutationSegment = bytes(callArena, mutationId);
            MemorySegment result;
            MethodHandle selectedExecute = raw ? executeRaw : execute;
            MethodHandle selectedExecuteMutation = raw
                    ? executeRawMutation
                    : executeMutation;
            if (mutationId.length == 0) {
                result = (MemorySegment) selectedExecute.invokeExact(
                        handle,
                        requestId,
                        operation,
                        keySegment,
                        (long) key.length,
                        valueSegment,
                        (long) value.length,
                        options.conditionCode(),
                        (byte) (options.ttlMillis() == 0 ? 0 : 1),
                        options.ttlMillis());
            } else {
                result = (MemorySegment) selectedExecuteMutation.invokeExact(
                        handle,
                        requestId,
                        operation,
                        keySegment,
                        (long) key.length,
                        valueSegment,
                        (long) value.length,
                        options.conditionCode(),
                        (byte) (options.ttlMillis() == 0 ? 0 : 1),
                        options.ttlMillis(),
                        mutationSegment,
                        (long) mutationId.length);
            }
            if (result.equals(MemorySegment.NULL)) {
                throw new OpenKacheException("native operation returned a null result");
            }
            try {
                int kind = (int) resultKind.invokeExact(result);
                byte[] payload = readPayload(result);
                if (kind == RESULT_ERROR) {
                    throw readError(
                            result,
                            resultKind,
                            resultData,
                            resultDataLength,
                            resultErrorMetadata,
                            callArena,
                            payload);
                }
                return new Result(kind, payload);
            } finally {
                resultFree.invokeExact(result);
            }
        } catch (OpenKacheException error) {
            throw error;
        } catch (Throwable error) {
            throw new OpenKacheException("native operation failed", error);
        } finally {
            releaseCall();
        }
    }

    private byte[] readPayload(MemorySegment result) throws Throwable {
        long length = (long) resultDataLength.invokeExact(result);
        if (length < 0 || length > Integer.MAX_VALUE) {
            throw new OpenKacheException("native payload is too large");
        }
        if (length == 0) {
            return new byte[0];
        }
        MemorySegment data = (MemorySegment) resultData.invokeExact(result);
        return data.reinterpret(length).toArray(ValueLayout.JAVA_BYTE);
    }

    private static OpenKacheException readError(
            MemorySegment result,
            MethodHandle ignoredResultKind,
            MethodHandle data,
            MethodHandle dataLength,
            MethodHandle metadataHandle,
            Arena metadataArena) throws Throwable {
        long length = (long) dataLength.invokeExact(result);
        byte[] payload = length == 0
                ? new byte[0]
                : ((MemorySegment) data.invokeExact(result))
                        .reinterpret(length)
                        .toArray(ValueLayout.JAVA_BYTE);
        return readError(
                result,
                ignoredResultKind,
                data,
                dataLength,
                metadataHandle,
                metadataArena,
                payload);
    }

    private static OpenKacheException readError(
            MemorySegment result,
            MethodHandle ignoredResultKind,
            MethodHandle ignoredData,
            MethodHandle ignoredDataLength,
            MethodHandle metadataHandle,
            Arena metadataArena,
            byte[] payload) throws Throwable {
        MemorySegment metadata = metadataArena.allocate(ERROR_METADATA_BYTES, 4);
        ErrorMetadata value = null;
        byte present = (byte) metadataHandle.invokeExact(result, metadata);
        if (present != 0) {
            value = ErrorMetadata.from(metadata);
        }
        String message = payload.length == 0
                ? "native client operation failed"
                : new String(payload, StandardCharsets.UTF_8);
        return new OpenKacheException(message, value);
    }

    private static byte[] requireKind(Result result, int expected, String operation) {
        if (result.kind != expected) {
            throw unexpected(operation, result.kind);
        }
        return result.payload;
    }

    private static OpenKacheException unexpected(String operation, int kind) {
        return new OpenKacheException(
                "unexpected native " + operation + " result " + kind);
    }

    private static SetOptions mutationOptions(SetOptions options) {
        SetOptions value = Objects.requireNonNull(options, "options");
        if (value.mutationId().length != 0) {
            return value;
        }
        byte[] mutationId = new byte[MUTATION_ID_BYTES];
        RANDOM.nextBytes(mutationId);
        return value.withMutationId(mutationId);
    }

    private long nextRequestId() {
        long value = nextRequestId.getAndUpdate(
                current -> current == Long.MAX_VALUE ? 1 : current + 1);
        return value == 0 ? nextRequestId() : value;
    }

    /**
     * Requests cancellation of a queued or active native operation.
     *
     * @param requestId caller-assigned request identifier
     * @return {@code true} when the worker found and canceled the request
     */
    public boolean cancel(long requestId) {
        MemorySegment handle;
        try {
            handle = acquireClient();
        } catch (OpenKacheException error) {
            return false;
        }
        try {
            byte value = (byte) cancel.invokeExact(handle, requestId);
            return value != 0;
        } catch (Throwable error) {
            return false;
        } finally {
            releaseCall();
        }
    }

    private MemorySegment acquireClient() {
        synchronized (lifecycle) {
            if (closed) {
                throw new OpenKacheException("client is closed");
            }
            activeCalls++;
            return client;
        }
    }

    private void releaseCall() {
        synchronized (lifecycle) {
            activeCalls--;
            if (activeCalls == 0) {
                lifecycle.notifyAll();
            }
        }
    }

    @Override
    public void close() {
        synchronized (lifecycle) {
            if (closed) {
                return;
            }
            closed = true;
            while (activeCalls != 0) {
                try {
                    lifecycle.wait();
                } catch (InterruptedException error) {
                    Thread.currentThread().interrupt();
                    throw new OpenKacheException("native close interrupted", error);
                }
            }
        }
        try {
            clientFree.invokeExact(client);
        } catch (Throwable error) {
            throw new OpenKacheException("native close failed", error);
        } finally {
            arena.close();
        }
    }

    private static SymbolLookup lookup(Arena arena) {
        String configured = System.getenv("OPENKACHE_CLIENT_NATIVE");
        if (configured != null && !configured.isBlank()) {
            return SymbolLookup.libraryLookup(configured, arena);
        }
        return SymbolLookup.loaderLookup();
    }

    private static MethodHandle downcall(
            SymbolLookup lookup,
            String name,
            FunctionDescriptor descriptor) {
        MemorySegment symbol = lookup.find(name)
                .orElseThrow(() -> new OpenKacheException("native symbol is missing: " + name));
        return Linker.nativeLinker().downcallHandle(symbol, descriptor);
    }

    private static MemorySegment bytes(Arena arena, byte[] value) {
        Objects.requireNonNull(value, "value");
        if (value.length == 0) {
            return MemorySegment.NULL;
        }
        MemorySegment segment = arena.allocate(value.length, 1);
        segment.asByteBuffer().put(value);
        return segment;
    }

    private static void zeroize(MemorySegment segment) {
        if (!segment.equals(MemorySegment.NULL) && segment.byteSize() > 0) {
            segment.fill((byte) 0);
        }
    }

    private static void zeroize(byte[] value) {
        Arrays.fill(value, (byte) 0);
    }

    private static void writeConnectOptions(
            MemorySegment target,
            MemorySegment address,
            MemorySegment serverName,
            MemorySegment certificate,
            MemorySegment clientCertificate,
            MemorySegment clientPrivateKey,
            MemorySegment dataProtectionKey,
            MemorySegment previousKeys,
            int previousKeyBytes,
            Options options) {
        target.set(ValueLayout.ADDRESS, CONNECT_ADDRESS, address);
        target.set(ValueLayout.JAVA_LONG, CONNECT_ADDRESS_LENGTH,
                (long) options.address().getBytes(StandardCharsets.UTF_8).length);
        target.set(ValueLayout.ADDRESS, CONNECT_SERVER_NAME, serverName);
        target.set(ValueLayout.JAVA_LONG, CONNECT_SERVER_NAME_LENGTH,
                (long) options.serverName().getBytes(StandardCharsets.UTF_8).length);
        target.set(ValueLayout.ADDRESS, CONNECT_CERTIFICATE, certificate);
        target.set(ValueLayout.JAVA_LONG, CONNECT_CERTIFICATE_LENGTH,
                (long) options.certificate().length);
        target.set(ValueLayout.ADDRESS, CONNECT_CLIENT_CERTIFICATE, clientCertificate);
        target.set(ValueLayout.JAVA_LONG, CONNECT_CLIENT_CERTIFICATE_LENGTH,
                (long) options.clientCertificateChain().length);
        target.set(ValueLayout.ADDRESS, CONNECT_CLIENT_PRIVATE_KEY, clientPrivateKey);
        target.set(ValueLayout.JAVA_LONG, CONNECT_CLIENT_PRIVATE_KEY_LENGTH,
                (long) options.clientPrivateKey().length);
        target.set(ValueLayout.ADDRESS, CONNECT_DATA_PROTECTION_KEY, dataProtectionKey);
        target.set(ValueLayout.JAVA_LONG, CONNECT_DATA_PROTECTION_KEY_LENGTH,
                (long) options.dataProtectionKey().length);
        target.set(ValueLayout.ADDRESS, CONNECT_PREVIOUS_KEYS, previousKeys);
        target.set(ValueLayout.JAVA_LONG, CONNECT_PREVIOUS_KEYS_LENGTH, previousKeyBytes);
        target.set(ValueLayout.JAVA_LONG, CONNECT_PREVIOUS_KEY_COUNT,
                (long) options.previousDataProtectionKeys().length);
        target.set(ValueLayout.JAVA_BYTE, CONNECT_COMPRESSION_ENABLED,
                (byte) (options.compressionEnabled() ? 1 : 0));
        target.set(ValueLayout.JAVA_INT, CONNECT_COMPRESSION_LEVEL, options.compressionLevel());
        target.set(ValueLayout.JAVA_LONG, CONNECT_MINIMUM_INPUT_SIZE,
                (long) options.minimumInputBytes());
        target.set(ValueLayout.JAVA_LONG, CONNECT_MINIMUM_SAVINGS,
                (long) options.minimumSavingsBytes());
        target.set(ValueLayout.JAVA_INT, CONNECT_ENCRYPTION, options.encryption());
        target.set(ValueLayout.JAVA_LONG, CONNECT_TIMEOUT, options.connectTimeoutMillis());
        target.set(ValueLayout.JAVA_LONG, CONNECT_REQUEST_TIMEOUT,
                options.requestTimeoutMillis());
        target.set(ValueLayout.JAVA_LONG, CONNECT_RETRY_MAX_ATTEMPTS,
                (long) options.retryMaxAttempts());
        target.set(ValueLayout.JAVA_LONG, CONNECT_MAX_IN_FLIGHT,
                (long) options.maxInFlight());
    }

    private static byte[] concatenate(byte[][] values) {
        int length = 0;
        for (byte[] value : values) {
            length = Math.addExact(length, value.length);
        }
        byte[] result = new byte[length];
        int offset = 0;
        for (byte[] value : values) {
            System.arraycopy(value, 0, result, offset, value.length);
            offset += value.length;
        }
        return result;
    }

    private static void validateItemId(byte[] itemId) {
        Objects.requireNonNull(itemId, "itemId");
        if (itemId.length != ITEM_ID_BYTES) {
            throw new IllegalArgumentException(
                    "itemId must contain exactly " + ITEM_ID_BYTES + " bytes");
        }
    }

    private static Throwable unwrap(Throwable error) {
        return error.getCause() instanceof CompletionException
                ? error.getCause().getCause()
                : error;
    }

    private record Result(int kind, byte[] payload) {}

    /** Structured metadata returned with a native error. */
    public record ErrorMetadata(
            int code,
            int operation,
            int phase,
            int backend,
            boolean retryable,
            boolean ambiguous,
            byte[] mutationId) {
        private static ErrorMetadata from(MemorySegment segment) {
            int mutationIdLength = Math.min(
                    MUTATION_ID_BYTES,
                    Byte.toUnsignedInt(segment.get(
                            ValueLayout.JAVA_BYTE,
                            io.openkache.client.generated.SmithyContract
                                    .FFI_ERROR_METADATA_MUTATION_ID_LENGTH_OFFSET)));
            byte[] mutationId = mutationIdLength == 0
                    ? null
                    : segment.asSlice(
                            io.openkache.client.generated.SmithyContract
                                    .FFI_ERROR_METADATA_MUTATION_ID_OFFSET,
                            mutationIdLength)
                            .toArray(ValueLayout.JAVA_BYTE);
            return new ErrorMetadata(
                    segment.get(ValueLayout.JAVA_INT,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_CODE_OFFSET),
                    segment.get(ValueLayout.JAVA_INT,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_OPERATION_OFFSET),
                    segment.get(ValueLayout.JAVA_INT,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_PHASE_OFFSET),
                    segment.get(ValueLayout.JAVA_INT,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_BACKEND_OFFSET),
                    segment.get(ValueLayout.JAVA_BYTE,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_RETRYABLE_OFFSET) != 0,
                    segment.get(ValueLayout.JAVA_BYTE,
                            io.openkache.client.generated.SmithyContract.FFI_ERROR_METADATA_AMBIGUOUS_OFFSET) != 0,
                    mutationId);
        }
    }

    /** Point-in-time native request, retry, transport, and lane counters. */
    public record MetricsSnapshot(
            long requests,
            long hits,
            long misses,
            long retries,
            long reconnects,
            long cancellations,
            long transportErrors,
            long protocolErrors,
            long bytesSent,
            long bytesReceived,
            long activeLanes) {
        private static MetricsSnapshot from(MemorySegment segment) {
            return new MetricsSnapshot(
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_REQUESTS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_HITS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_MISSES_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_RETRIES_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_RECONNECTS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_CANCELLATIONS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_TRANSPORT_ERRORS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_PROTOCOL_ERRORS_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_BYTES_SENT_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_BYTES_RECEIVED_OFFSET),
                    segment.get(ValueLayout.JAVA_LONG,
                            io.openkache.client.generated.SmithyContract.FFI_METRICS_SNAPSHOT_ACTIVE_LANES_OFFSET));
        }
    }

    /** Immutable mutation and TTL options. */
    public record SetOptions(
            SmithyApi.SetCondition condition,
            long ttlMillis,
            byte[] mutationId) {
        public SetOptions {
            mutationId = mutationId == null ? new byte[0] : mutationId.clone();
            if (mutationId.length != 0 && mutationId.length != MUTATION_ID_BYTES) {
                throw new IllegalArgumentException(
                        "mutationId must contain exactly " + MUTATION_ID_BYTES + " bytes");
            }
            if (ttlMillis < 0) {
                throw new IllegalArgumentException("ttlMillis must not be negative");
            }
        }

        public int conditionCode() {
            if (condition == null) {
                return SET_CONDITION_NONE;
            }
            return switch (condition) {
                case IfAbsent -> SET_CONDITION_IF_ABSENT;
                case IfPresent -> SET_CONDITION_IF_PRESENT;
            };
        }

        @Override
        public byte[] mutationId() {
            return mutationId.clone();
        }

        public SetOptions withMutationId(byte[] value) {
            return new SetOptions(condition, ttlMillis, value);
        }

        public static SetOptions none() {
            return new SetOptions(null, 0, new byte[0]);
        }
    }

    /** Connection settings for one native client. */
    public record DataProtectionKeyRing(byte[] active, byte[][] previous) {
        public DataProtectionKeyRing {
            active = active == null ? new byte[0] : active.clone();
            previous = Options.deepCopy(previous == null ? new byte[0][] : previous);
            if (active.length
                    != io.openkache.client.generated.SmithyContract
                            .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES) {
                throw new IllegalArgumentException(
                        "active key must contain "
                                + io.openkache.client.generated.SmithyContract
                                        .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES
                                + " bytes");
            }
            if (previous.length
                    > io.openkache.client.generated.SmithyContract
                            .MAX_PREVIOUS_DATA_PROTECTION_KEYS) {
                throw new IllegalArgumentException(
                        "previous keys may contain at most "
                                + io.openkache.client.generated.SmithyContract
                                        .MAX_PREVIOUS_DATA_PROTECTION_KEYS
                                + " entries");
            }
            for (byte[] key : previous) {
                if (key.length
                        != io.openkache.client.generated.SmithyContract
                                .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES) {
                    throw new IllegalArgumentException(
                            "each previous key must contain "
                                    + io.openkache.client.generated.SmithyContract
                                            .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES
                                    + " bytes");
                }
            }
        }

        @Override
        public byte[] active() {
            return active.clone();
        }

        @Override
        public byte[][] previous() {
            return Options.deepCopy(previous);
        }
    }

    /** Connection settings for one native client. */
    public record Options(
            String address,
            String serverName,
            byte[] certificate,
            byte[] dataProtectionKey,
            byte[] clientCertificateChain,
            byte[] clientPrivateKey,
            boolean compressionEnabled,
            int compressionLevel,
            int minimumInputBytes,
            int minimumSavingsBytes,
            int encryption,
            int retryMaxAttempts,
            int maxInFlight,
            long connectTimeoutMillis,
            long requestTimeoutMillis,
            byte[][] previousDataProtectionKeys) {
        public Options {
            certificate = certificate == null ? new byte[0] : certificate.clone();
            dataProtectionKey = dataProtectionKey == null
                    ? new byte[0]
                    : dataProtectionKey.clone();
            clientCertificateChain = clientCertificateChain == null
                    ? new byte[0]
                    : clientCertificateChain.clone();
            clientPrivateKey = clientPrivateKey == null
                    ? new byte[0]
                    : clientPrivateKey.clone();
            previousDataProtectionKeys = deepCopy(
                    previousDataProtectionKeys == null
                            ? new byte[0][]
                            : previousDataProtectionKeys);
        }

        /** Compatibility constructor without retired keys. */
        public Options(
                String address,
                String serverName,
                byte[] certificate,
                byte[] dataProtectionKey,
                byte[] clientCertificateChain,
                byte[] clientPrivateKey,
                boolean compressionEnabled,
                int compressionLevel,
                int minimumInputBytes,
                int minimumSavingsBytes,
                int encryption,
                int retryMaxAttempts,
                int maxInFlight,
                long connectTimeoutMillis,
                long requestTimeoutMillis) {
            this(
                    address,
                    serverName,
                    certificate,
                    dataProtectionKey,
                    clientCertificateChain,
                    clientPrivateKey,
                    compressionEnabled,
                    compressionLevel,
                    minimumInputBytes,
                    minimumSavingsBytes,
                    encryption,
                    retryMaxAttempts,
                    maxInFlight,
                    connectTimeoutMillis,
                    requestTimeoutMillis,
                    new byte[0][]);
        }

        /** Convenience constructor accepting an active and retired key ring. */
        public Options(
                String address,
                String serverName,
                byte[] certificate,
                DataProtectionKeyRing keyRing,
                byte[] clientCertificateChain,
                byte[] clientPrivateKey,
                boolean compressionEnabled,
                int compressionLevel,
                int minimumInputBytes,
                int minimumSavingsBytes,
                int encryption,
                int retryMaxAttempts,
                int maxInFlight,
                long connectTimeoutMillis,
                long requestTimeoutMillis) {
            this(
                    address,
                    serverName,
                    certificate,
                    keyRing.active(),
                    clientCertificateChain,
                    clientPrivateKey,
                    compressionEnabled,
                    compressionLevel,
                    minimumInputBytes,
                    minimumSavingsBytes,
                    encryption,
                    retryMaxAttempts,
                    maxInFlight,
                    connectTimeoutMillis,
                    requestTimeoutMillis,
                    keyRing.previous());
        }

        @Override
        public byte[][] previousDataProtectionKeys() {
            return deepCopy(previousDataProtectionKeys);
        }

        public void validate() {
            if (address == null || address.isBlank()) {
                throw new IllegalArgumentException("address");
            }
            if (serverName == null) {
                throw new IllegalArgumentException("serverName");
            }
            if (dataProtectionKey.length
                    != io.openkache.client.generated.SmithyContract
                            .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES) {
                throw new IllegalArgumentException(
                        "dataProtectionKey must contain "
                                + io.openkache.client.generated.SmithyContract
                                        .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES
                                + " bytes");
            }
            if (previousDataProtectionKeys.length
                    > io.openkache.client.generated.SmithyContract
                            .MAX_PREVIOUS_DATA_PROTECTION_KEYS) {
                throw new IllegalArgumentException(
                        "previousDataProtectionKeys may contain at most "
                                + io.openkache.client.generated.SmithyContract
                                        .MAX_PREVIOUS_DATA_PROTECTION_KEYS
                                + " keys");
            }
            for (byte[] key : previousDataProtectionKeys) {
                if (key.length
                        != io.openkache.client.generated.SmithyContract
                                .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES) {
                    throw new IllegalArgumentException(
                            "each previous data-protection key must contain "
                                    + io.openkache.client.generated.SmithyContract
                                            .VALUE_FORMAT_DATA_PROTECTION_KEY_BYTES
                                    + " bytes");
                }
            }
            if (connectTimeoutMillis
                            < io.openkache.client.generated.SmithyContract
                                    .CLIENT_MINIMUM_POSITIVE_VALUE
                    || requestTimeoutMillis
                            < io.openkache.client.generated.SmithyContract
                                    .CLIENT_MINIMUM_POSITIVE_VALUE) {
                throw new IllegalArgumentException("timeouts must be positive");
            }
            if (retryMaxAttempts
                            < io.openkache.client.generated.SmithyContract
                                    .CLIENT_MINIMUM_POSITIVE_VALUE
                    || maxInFlight
                            < io.openkache.client.generated.SmithyContract
                                    .CLIENT_MINIMUM_POSITIVE_VALUE) {
                throw new IllegalArgumentException("retry and lane limits must be positive");
            }
            if (compressionLevel
                            < io.openkache.client.generated.SmithyContract
                                    .DEFAULT_ZSTANDARD_LEVEL_MIN
                    || compressionLevel
                            > io.openkache.client.generated.SmithyContract
                                    .DEFAULT_ZSTANDARD_LEVEL_MAX) {
                throw new IllegalArgumentException("compressionLevel is outside Zstandard limits");
            }
            if (minimumInputBytes < 0 || minimumSavingsBytes < 0) {
                throw new IllegalArgumentException("compression thresholds must not be negative");
            }
            if (encryption
                            < io.openkache.client.generated.SmithyContract
                                    .VALUE_FORMAT_ENCRYPTION_NONE
                    || encryption
                            > io.openkache.client.generated.SmithyContract
                                    .VALUE_FORMAT_ENCRYPTION_ROBUST) {
                throw new IllegalArgumentException("unsupported encryption profile " + encryption);
            }
        }

        private static byte[][] deepCopy(byte[][] values) {
            byte[][] copy = new byte[values.length][];
            for (int index = 0; index < values.length; index++) {
                copy[index] = values[index] == null ? new byte[0] : values[index].clone();
            }
            return copy;
        }
    }

    /** Structured native failure. */
    public static final class OpenKacheException extends RuntimeException {
        private final ErrorMetadata metadata;

        public OpenKacheException(String message) {
            this(message, null, null);
        }

        public OpenKacheException(String message, Throwable cause) {
            this(message, cause, null);
        }

        public OpenKacheException(String message, ErrorMetadata metadata) {
            this(message, null, metadata);
        }

        private OpenKacheException(
                String message,
                Throwable cause,
                ErrorMetadata metadata) {
            super(message, cause);
            this.metadata = metadata;
        }

        public ErrorMetadata metadata() {
            return metadata;
        }

        public boolean retryable() {
            return metadata != null && metadata.retryable();
        }

        public boolean ambiguous() {
            return metadata != null && metadata.ambiguous();
        }

        public boolean cancelled() {
            return metadata != null && metadata.code() == FFI_ERROR_CANCELLED;
        }
    }

    private static final class CancelableFuture<T> extends CompletableFuture<T> {
        private final Runnable cancellation;

        private CancelableFuture(Runnable cancellation) {
            this.cancellation = cancellation;
        }

        @Override
        public boolean cancel(boolean mayInterruptIfRunning) {
            boolean canceled = super.cancel(mayInterruptIfRunning);
            if (canceled) {
                try {
                    cancellation.run();
                } catch (RuntimeException ignored) {
                    // The Java future is already canceled; the native worker
                    // will also clean up requests during client shutdown.
                }
            }
            return canceled;
        }
    }
}
