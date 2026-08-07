package io.openkache.client

import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import io.openkache.client.generated_local.SmithyContract
import io.openkache.client.generated_local.SmithyNativeApi
import io.openkache.client.generated_local.SmithyNativeDescriptor
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Public Kotlin adapter surface for the complete generated Smithy API. */
public interface OpenKacheClient : SmithyGeneratedOperations

/**
 * Rust-backed Kotlin client implementing every generated Smithy operation.
 *
 * The native core owns QUIC, TLS, framing, retries, and result ownership.
 * This adapter only marshals Kotlin DTOs through the stable C ABI.
 */
public class Client private constructor(
    private val native: SmithyNativeApi,
    private var handle: Pointer?,
) : OpenKacheClient, AutoCloseable {
    private var closed = false

    @Synchronized
    override fun close() {
        if (closed) return
        closed = true
        handle?.let(native::openkache_client_free)
        handle = null
    }

    override fun smithyInvoke(
        operation: Int,
        applicationKey: ByteArray,
        value: ByteArray,
        setCondition: Int,
        ttlMilliseconds: Long,
    ): NativeResult {
        synchronized(this) {
            val client = requireHandle()
            Buffer(applicationKey).use { key ->
                Buffer(value).use { payload ->
                    return readResult(
                        native,
                        native.openkache_client_execute_typed(
                            client,
                            operation,
                            SmithyContract.KEY_SPEC_BYTES,
                            key.pointer,
                            key.length,
                            payload.pointer,
                            payload.length,
                            setCondition,
                            if (ttlMilliseconds == 0L) 0 else 1,
                            ttlMilliseconds,
                        ),
                    )
                }
            }
        }
    }

    override fun smithyInvokeScoped(
        operation: Int,
        namespaceId: Long,
        itemId: ByteArray,
        value: ByteArray,
        flags: Int,
        ttlMilliseconds: Long,
    ): NativeResult {
        val requiredItemIdBytes = SmithyContract.operationItemIdBytes(operation)
        if (requiredItemIdBytes != 0 && itemId.size != requiredItemIdBytes) {
            throw IllegalArgumentException("itemId must contain exactly ${requiredItemIdBytes} bytes")
        }
        if (itemId.isNotEmpty() && !SmithyContract.operationSupportsScoped(operation)) {
            throw IllegalArgumentException("operation does not accept an itemId")
        }
        synchronized(this) {
            val client = requireHandle()
            Buffer(itemId).use { item ->
                Buffer(value).use { payload ->
                    return readResult(
                        native,
                        native.openkache_client_execute_scoped(
                            client,
                            operation,
                            namespaceId,
                            item.pointer,
                            item.length,
                            payload.pointer,
                            payload.length,
                            flags.toByte(),
                            ttlMilliseconds,
                        ),
                    )
                }
            }
        }
    }

    override fun smithyNamespaceOpen(
        name: ByteArray,
        createIfMissing: Boolean,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult {
        synchronized(this) {
            val client = requireHandle()
            Buffer(name).use { nameBuffer ->
                return readResult(
                    native,
                    native.openkache_client_namespace_open(
                        client,
                        nameBuffer.pointer,
                        nameBuffer.length,
                        if (createIfMissing) 1 else 0,
                        policyFlags.toByte(),
                        ttlMilliseconds,
                    ),
                )
            }
        }
    }

    override fun smithyNamespaceUpdatePolicy(
        namespaceId: Long,
        expectedRevision: Long,
        policyFlags: Int,
        ttlMilliseconds: Long,
    ): NativeResult {
        synchronized(this) {
            return readResult(
                native,
                native.openkache_client_namespace_update_policy(
                    requireHandle(),
                    namespaceId,
                    expectedRevision,
                    policyFlags.toByte(),
                    ttlMilliseconds,
                ),
            )
        }
    }

    override fun smithyNamespaceDelete(
        namespaceId: Long,
        expectedRevision: Long,
    ): NativeResult {
        synchronized(this) {
            return readResult(
                native,
                native.openkache_client_namespace_delete(
                    requireHandle(),
                    namespaceId,
                    expectedRevision,
                ),
            )
        }
    }

    override fun smithyDecodeDescriptor(payload: ByteArray): NamespaceDescriptor {
        Buffer(payload).use { buffer ->
            val descriptor = SmithyNativeDescriptor()
            val status = native.openkache_client_namespace_descriptor_decode(
                buffer.pointer,
                buffer.length,
                descriptor,
            )
            require(status == SmithyContract.DESCRIPTOR_DECODE_OK) {
                "native client returned an invalid namespace descriptor"
            }
            descriptor.read()
            val expiration = if (descriptor.defaultExpiration == SmithyContract.DEFAULT_EXPIRATION_FIXED_TTL) {
                ExpirationDefault.FixedTtl
            } else {
                ExpirationDefault.NoExpiry
            }
            val eviction = if (descriptor.defaultEviction == SmithyContract.DEFAULT_EVICTION_PROTECTED) {
                EvictionDefault.EvictionProtected
            } else {
                EvictionDefault.Evictable
            }
            return NamespaceDescriptor(
                descriptor.namespaceId,
                descriptor.revision,
                NamespacePolicy(
                    expiration,
                    if (expiration == ExpirationDefault.FixedTtl) descriptor.defaultTtlMs else null,
                    if (descriptor.expirationOverride == SmithyContract.OVERRIDE_ALLOWED) {
                        OverridePolicy.Allowed
                    } else {
                        OverridePolicy.Disallowed
                    },
                    eviction,
                    if (descriptor.evictionOverride == SmithyContract.OVERRIDE_ALLOWED) {
                        OverridePolicy.Allowed
                    } else {
                        OverridePolicy.Disallowed
                    },
                ),
            )
        }
    }

    override fun smithyDecodeUtf8(payload: ByteArray, operation: String): String {
        return try {
            StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(payload))
                .toString()
        } catch (error: CharacterCodingException) {
            throw OpenKacheClientException("$operation response is not valid UTF-8", error)
        }
    }

    private fun requireHandle(): Pointer {
        check(!closed && handle != null) { "OpenKache client is closed" }
        return handle!!
    }

    private class Buffer(value: ByteArray) : AutoCloseable {
        private val memory = if (value.isEmpty()) null else Memory(value.size.toLong()).also {
            it.write(0, value, 0, value.size)
        }

        val pointer: Pointer?
            get() = memory
        val length: Long
            get() = memory?.size() ?: 0

        override fun close() {
            memory?.close()
        }
    }

    companion object {
        /**
         * Connects to an OpenKache server through the shared native client core.
         */
        public fun connect(
            address: String,
            serverName: String,
            certificate: ByteArray = ByteArray(0),
            dataProtectionKey: ByteArray = ByteArray(32),
        ): Client {
            require(dataProtectionKey.size == 32) {
                "dataProtectionKey must contain exactly 32 bytes"
            }
            val configured = System.getenv("OPENKACHE_CLIENT_NATIVE")
            val native = try {
                Native.load(
                    if (configured.isNullOrBlank()) "openkache_client_core" else configured,
                    SmithyNativeApi::class.java,
                )
            } catch (error: UnsatisfiedLinkError) {
                throw OpenKacheClientException("failed to load OpenKache native client", error)
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
                            return Client(
                                native,
                                result.client ?: error("native client returned no client handle"),
                            )
                        }
                    }
                }
            }
        }

        private fun readResult(
            native: SmithyNativeApi,
            result: Pointer?,
            takeClient: Boolean = false,
        ): NativeResult {
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
                val client = if (takeClient) native.openkache_client_result_take_client(result) else null
                if (kind == SmithyContract.RESULT_ERROR) {
                    throw OpenKacheClientException(
                        String(payload, StandardCharsets.UTF_8)
                            .ifEmpty { "native client operation failed" },
                    )
                }
                return NativeResult(kind, payload, client)
            } finally {
                native.openkache_client_result_free(result)
            }
        }

        private fun requireKind(result: NativeResult, expected: Int, operation: String) {
            if (result.kind != expected) throw unexpectedKind(operation, result.kind)
        }

        private fun unexpectedKind(operation: String, kind: Int) =
            OpenKacheClientException("$operation returned unexpected native result $kind")
    }
}

public data class NativeResult(
    public val kind: Int,
    public val payload: ByteArray,
    public val client: Pointer? = null,
)

public class OpenKacheClientException(message: String, cause: Throwable? = null) :
    RuntimeException(message, cause)
