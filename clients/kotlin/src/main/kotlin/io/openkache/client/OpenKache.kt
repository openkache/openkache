package io.openkache.client

import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import io.openkache.client.generated_local.SmithyContract
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/**
 * Contract surface for the Rust-backed Kotlin client.
 */
public interface OpenKacheClient : SmithyEchoApi

/**
 * Experimental Kotlin client backed by the shared Rust client-core C ABI.
 *
 * The native core owns QUIC, TLS, framing, retry, and result lifetimes. This
 * adapter only marshals Kotlin strings and runs the blocking ABI call on the
 * IO dispatcher.
 */
public class EchoClient private constructor(
    private val native: NativeApi,
    private var handle: Pointer?,
) : OpenKacheClient, AutoCloseable {
    private var closed = false

    override suspend fun echo(input: EchoInput): EchoOutput = withContext(Dispatchers.IO) {
        checkOpen()
        EchoOutput(echoText(input.message))
    }

    /**
     * Sends one message and returns its echoed text.
     */
    public suspend fun echo(message: String): String = echo(EchoInput(message)).message

    @Synchronized
    override fun close() {
        if (closed) return
        closed = true
        handle?.let(native::openkache_client_free)
        handle = null
    }

    private fun checkOpen() {
        check(!closed && handle != null) { "OpenKache client is closed" }
    }

    private fun echoText(message: String): String {
        val result = invokeEcho(message.toByteArray(StandardCharsets.UTF_8))
        val decoder = StandardCharsets.UTF_8.newDecoder()
            .onMalformedInput(CodingErrorAction.REPORT)
            .onUnmappableCharacter(CodingErrorAction.REPORT)
        return try {
            decoder.decode(ByteBuffer.wrap(result)).toString()
        } catch (error: CharacterCodingException) {
            throw EchoClientException("ECHO response is not valid UTF-8", error)
        }
    }

    @Synchronized
    private fun invokeEcho(message: ByteArray): ByteArray {
        val client = handle ?: error("OpenKache client is closed")
        Buffer(ByteArray(0)).use { applicationKey ->
            Buffer(message).use { value ->
                val result = readResult(
                    native,
                    native.openkache_client_execute(
                        client,
                        SmithyContract.OPERATION_ECHO,
                        applicationKey.pointer,
                        applicationKey.length,
                        value.pointer,
                        value.length,
                        SmithyContract.SET_CONDITION_ANY,
                        0,
                        0,
                    ),
                )
                check(result.kind == SmithyContract.RESULT_VALUE) {
                    "native client returned an invalid ECHO result"
                }
                return result.payload
            }
        }
    }

    private class Buffer(value: ByteArray) : AutoCloseable {
        private val memory = if (value.isEmpty()) null else Memory(value.size.toLong())

        init {
            memory?.write(0, value, 0, value.size)
        }

        val pointer: Pointer?
            get() = memory
        val length: Long
            get() = memory?.size() ?: 0

        override fun close() {
            memory?.close()
        }
    }

    private interface NativeApi : Library {
        fun openkache_client_abi_version(): Int

        fun openkache_client_connect(
            address: Pointer?,
            addressLength: Long,
            serverName: Pointer?,
            serverNameLength: Long,
            certificate: Pointer?,
            certificateLength: Long,
            dataProtectionKey: Pointer?,
            dataProtectionKeyLength: Long,
            compressionEnabled: Byte,
            compressionLevel: Int,
            minimumInputSize: Long,
            minimumSavings: Long,
            connectTimeoutMilliseconds: Long,
            requestTimeoutMilliseconds: Long,
        ): Pointer?

        fun openkache_client_execute(
            client: Pointer,
            operation: Int,
            applicationKey: Pointer?,
            applicationKeyLength: Long,
            value: Pointer?,
            valueLength: Long,
            setCondition: Int,
            ttlEnabled: Byte,
            ttlMilliseconds: Long,
        ): Pointer?

        fun openkache_client_result_kind(result: Pointer): Int
        fun openkache_client_result_data(result: Pointer): Pointer?
        fun openkache_client_result_data_length(result: Pointer): Long
        fun openkache_client_result_take_client(result: Pointer): Pointer?
        fun openkache_client_result_free(result: Pointer)
        fun openkache_client_free(client: Pointer)
    }

    companion object {
        /**
         * Connects to an OpenKache server.
         */
        public fun connect(
            address: String,
            serverName: String,
            certificate: ByteArray = ByteArray(0),
            dataProtectionKey: ByteArray = ByteArray(32),
        ): EchoClient {
            require(dataProtectionKey.size == 32) {
                "dataProtectionKey must contain exactly 32 bytes"
            }
            val configured = System.getenv("OPENKACHE_CLIENT_NATIVE")
            val native = try {
                Native.load(
                    if (configured.isNullOrBlank()) "openkache_client_core" else configured,
                    NativeApi::class.java,
                )
            } catch (error: UnsatisfiedLinkError) {
                throw EchoClientException("failed to load OpenKache native client", error)
            }
            check(native.openkache_client_abi_version() == SmithyContract.ABI_VERSION) {
                "unsupported OpenKache native ABI version"
            }
            Buffer(address.toByteArray(StandardCharsets.UTF_8)).use { addressBuffer ->
                Buffer(serverName.toByteArray(StandardCharsets.UTF_8)).use { serverBuffer ->
                    Buffer(certificate).use { certificateBuffer ->
                        Buffer(dataProtectionKey).use { keyBuffer ->
                            val result = readResult(
                                native,
                                native.openkache_client_connect(
                                    addressBuffer.pointer,
                                    addressBuffer.length,
                                    serverBuffer.pointer,
                                    serverBuffer.length,
                                    certificateBuffer.pointer,
                                    certificateBuffer.length,
                                    keyBuffer.pointer,
                                    keyBuffer.length,
                                    0,
                                    SmithyContract.DEFAULT_ZSTANDARD_LEVEL,
                                    SmithyContract.DEFAULT_ZSTANDARD_MINIMUM_INPUT_BYTES,
                                    SmithyContract.DEFAULT_ZSTANDARD_MINIMUM_SAVINGS_BYTES,
                                    SmithyContract.DEFAULT_CONNECT_TIMEOUT_MILLISECONDS,
                                    SmithyContract.DEFAULT_REQUEST_TIMEOUT_MILLISECONDS,
                                ),
                                takeClient = true,
                            )
                            check(result.kind == SmithyContract.RESULT_CONNECTED) {
                                "native client did not return a connected handle"
                            }
                            val handle = result.payloadPointer
                                ?: error("native client returned no client handle")
                            return EchoClient(native, handle)
                        }
                    }
                }
            }
        }

        private fun readResult(
            native: NativeApi,
            result: Pointer?,
            takeClient: Boolean = false,
        ): NativeResultWithHandle {
            requireNotNull(result) { "native client returned a null result" }
            try {
                val kind = native.openkache_client_result_kind(result)
                val length = native.openkache_client_result_data_length(result)
                require(length in 0..Int.MAX_VALUE) { "native client returned an oversized payload" }
                val data = native.openkache_client_result_data(result)
                val payload = if (length == 0L) {
                    ByteArray(0)
                } else {
                    requireNotNull(data) { "native client returned a null payload" }
                        .getByteArray(0, length.toInt())
                }
                val client = if (takeClient) {
                    native.openkache_client_result_take_client(result)
                } else {
                    null
                }
                if (kind == SmithyContract.RESULT_ERROR) {
                    throw EchoClientException(
                        String(payload, StandardCharsets.UTF_8)
                            .ifEmpty { "native client operation failed" },
                    )
                }
                return NativeResultWithHandle(kind, payload, client)
            } finally {
                native.openkache_client_result_free(result)
            }
        }
    }

    private data class NativeResultWithHandle(
        val kind: Int,
        val payload: ByteArray,
        val payloadPointer: Pointer?,
    )

    private class EchoClientException(message: String, cause: Throwable? = null) :
        RuntimeException(message, cause)
}
