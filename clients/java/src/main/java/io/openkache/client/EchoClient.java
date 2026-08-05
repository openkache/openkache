package io.openkache.client;

import com.sun.jna.Library;
import com.sun.jna.Memory;
import com.sun.jna.Native;
import com.sun.jna.Pointer;
import io.openkache.client.generated_local.SmithyContract;

import java.nio.ByteBuffer;
import java.nio.CharBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Objects;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * Experimental Java client for the Smithy {@code Echo} operation.
 *
 * <p>All QUIC, TLS, retry, framing, and response ownership behavior remains
 * in the shared Rust client core. This adapter only marshals Java strings to
 * the stable C ABI and owns the native client handle.</p>
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

        int openkache_client_result_kind(Pointer result);

        Pointer openkache_client_result_data(Pointer result);

        long openkache_client_result_data_length(Pointer result);

        Pointer openkache_client_result_take_client(Pointer result);

        void openkache_client_result_free(Pointer result);

        void openkache_client_free(Pointer client);
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
     * @throws EchoClientException when loading the ABI or connecting fails
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
            Pointer result = nativeApi.openkache_client_connect(
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
                SmithyContract.DEFAULT_REQUEST_TIMEOUT_MILLISECONDS);
            NativeResult nativeResult = readResult(nativeApi, result, true);
            if (nativeResult.kind() != SmithyContract.RESULT_CONNECTED
                || nativeResult.client() == null) {
                throw new EchoClientException("native client did not return a connected handle");
            }
            return new EchoClient(nativeApi, nativeResult.client());
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
    public CompletionStage<EchoOutput> echo(EchoInput input) {
        Objects.requireNonNull(input, "input");
        synchronized (lifecycle) {
            ensureOpen();
        }
        return CompletableFuture.supplyAsync(
            () -> new EchoOutput(echoText(input.message())),
            executor);
    }

    /**
     * Sends one message and decodes the strict UTF-8 response.
     *
     * @param message message to echo
     * @return asynchronous echoed message
     */
    public CompletionStage<String> echo(String message) {
        return echo(new EchoInput(message)).thenApply(EchoOutput::message);
    }

    private String echoText(String message) {
        byte[] bytes = echoBytes(message.getBytes(StandardCharsets.UTF_8));
        try {
            CharBuffer decoded = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(bytes));
            return decoded.toString();
        } catch (CharacterCodingException error) {
            throw new EchoClientException("ECHO response is not valid UTF-8", error);
        }
    }

    private byte[] echoBytes(byte[] message) {
        synchronized (lifecycle) {
            ensureOpen();
            try (
                NativeBuffer applicationKey = new NativeBuffer(new byte[0]);
                NativeBuffer value = new NativeBuffer(message)
            ) {
                NativeResult result = readResult(
                    nativeApi,
                    nativeApi.openkache_client_execute(
                        handle,
                        SmithyContract.OPERATION_ECHO,
                        applicationKey.pointer(),
                        applicationKey.length(),
                        value.pointer(),
                        value.length(),
                        SmithyContract.SET_CONDITION_ANY,
                        (byte) 0,
                        0),
                    false);
                if (result.kind() != SmithyContract.RESULT_VALUE) {
                    throw new EchoClientException("native client returned an invalid ECHO result");
                }
                return result.payload();
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

    private void ensureOpen() {
        if (closed || handle == null) {
            throw new EchoClientException("OpenKache client is closed");
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
                : data.getByteArray(0, (int) length);
            Pointer client = takeClient
                ? nativeApi.openkache_client_result_take_client(result)
                : null;
            if (kind == SmithyContract.RESULT_ERROR) {
                throw new EchoClientException(
                    payload.length == 0
                        ? "native client operation failed"
                        : new String(payload, StandardCharsets.UTF_8));
            }
            return new NativeResult(kind, Arrays.copyOf(payload, payload.length), client);
        } finally {
            nativeApi.openkache_client_result_free(result);
        }
    }
}
