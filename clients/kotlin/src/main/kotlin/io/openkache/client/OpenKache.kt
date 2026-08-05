package io.openkache.client

import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import io.openkache.client.generated_local.SmithyContract
import io.openkache.client.generated_local.SmithyNativeApi
import io.openkache.client.generated_local.SmithyNativeDescriptor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

/** Public Kotlin adapter surface for the complete generated Smithy API. */
public interface OpenKacheClient : SmithyOpenKacheApi

/**
 * Rust-backed Kotlin client implementing every generated Smithy operation.
 *
 * The native core owns QUIC, TLS, framing, retries, and result ownership.
 * This adapter only marshals Kotlin DTOs through the stable C ABI.
 */
public class EchoClient private constructor(
    private val native: SmithyNativeApi,
    private var handle: Pointer?,
) : OpenKacheClient, AutoCloseable {
    private var closed = false

    override suspend fun ping(input: PingInput): PingOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        requireKind(invoke(SmithyContract.OPERATION_PING, byteArrayOf(), byteArrayOf()), SmithyContract.RESULT_OK, "PING")
        PingOutput()
    }

    override suspend fun echo(input: EchoInput): EchoOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        EchoOutput(
            decodeUtf8(
                invoke(
                    SmithyContract.OPERATION_ECHO,
                    byteArrayOf(),
                    input.message.toByteArray(),
                ).payload,
            ),
        )
    }

    /** Sends one message and returns its echoed text. */
    public suspend fun echo(message: String): String = echo(EchoInput(message)).message

    override suspend fun get(input: GetInput): GetOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val result = invokeScoped(SmithyContract.OPERATION_GET, input.namespaceId, input.itemId, byteArrayOf())
        when (result.kind) {
            SmithyContract.RESULT_VALUE -> GetOutput(result.payload)
            SmithyContract.RESULT_NOT_FOUND -> GetOutput(null)
            else -> throw unexpectedKind("GET", result.kind)
        }
    }

    override suspend fun set(input: SetInput): SetOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val flags = setFlags(input)
        val result = invokeScoped(
            SmithyContract.OPERATION_SET,
            input.namespaceId,
            input.itemId,
            input.value,
            flags.flags,
            flags.ttlMilliseconds,
        )
        val outcome = when (result.kind) {
            SmithyContract.RESULT_CREATED -> SetOutcome.Created
            SmithyContract.RESULT_REPLACED -> SetOutcome.Replaced
            SmithyContract.RESULT_NOT_STORED -> SetOutcome.NotStored
            else -> throw unexpectedKind("SET", result.kind)
        }
        SetOutput(outcome)
    }

    override suspend fun delete(input: DeleteInput): DeleteOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val result = invokeScoped(SmithyContract.OPERATION_DELETE, input.namespaceId, input.itemId, byteArrayOf())
        when (result.kind) {
            SmithyContract.RESULT_DELETED -> DeleteOutput(true)
            SmithyContract.RESULT_NOT_DELETED -> DeleteOutput(false)
            else -> throw unexpectedKind("DELETE", result.kind)
        }
    }

    override suspend fun stats(input: StatsInput): StatsOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val result = invokeScoped(SmithyContract.OPERATION_STATS, input.namespaceId, byteArrayOf(), byteArrayOf())
        requireKind(result, SmithyContract.RESULT_VALUE, "STATS")
        StatsOutput(decodeUtf8(result.payload))
    }

    override suspend fun sync(input: SyncInput): SyncOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val result = invokeScoped(SmithyContract.OPERATION_SYNC, input.namespaceId, byteArrayOf(), byteArrayOf())
        requireKind(result, SmithyContract.RESULT_OK, "SYNC")
        SyncOutput()
    }

    override suspend fun namespaceOpen(input: NamespaceOpenInput): NamespaceOpenOutput =
        withContext(Dispatchers.IO) {
            requireNotNull(input)
            val name = input.name.toByteArray(StandardCharsets.UTF_8)
            require(name.size <= SmithyContract.NAMESPACE_NAME_MAX_BYTES) {
                "namespace name exceeds protocol limit"
            }
            val policy = policyFlags(input.policy, input.createIfMissing)
            Buffer(name).use { nameBuffer ->
                val result = synchronized(this@EchoClient) {
                    readResult(
                        native,
                        native.openkache_client_namespace_open(
                            requireHandle(),
                            nameBuffer.pointer,
                            nameBuffer.length,
                            if (input.createIfMissing) 1 else 0,
                            policy.flags.toByte(),
                            policy.ttlMilliseconds,
                        ),
                    )
                }
                val created = result.kind == SmithyContract.RESULT_CREATED
                if (!created && result.kind != SmithyContract.RESULT_OK) {
                    throw unexpectedKind("NAMESPACE_OPEN", result.kind)
                }
                NamespaceOpenOutput(decodeDescriptor(result.payload), created)
            }
        }

    override suspend fun namespaceUpdatePolicy(
        input: NamespaceUpdatePolicyInput,
    ): NamespaceUpdatePolicyOutput = withContext(Dispatchers.IO) {
        requireNotNull(input)
        val policy = policyFlags(input.policy, true)
        val result = synchronized(this@EchoClient) {
            readResult(
                native,
                native.openkache_client_namespace_update_policy(
                    requireHandle(),
                    input.namespaceId,
                    input.expectedRevision,
                    policy.flags.toByte(),
                    policy.ttlMilliseconds,
                ),
            )
        }
        requireKind(result, SmithyContract.RESULT_VALUE, "NAMESPACE_UPDATE_POLICY")
        NamespaceUpdatePolicyOutput(decodeDescriptor(result.payload))
    }

    override suspend fun namespaceDelete(input: NamespaceDeleteInput): NamespaceDeleteOutput =
        withContext(Dispatchers.IO) {
            requireNotNull(input)
            val result = synchronized(this@EchoClient) {
                readResult(
                    native,
                    native.openkache_client_namespace_delete(
                        requireHandle(),
                        input.namespaceId,
                        input.expectedRevision,
                    ),
                )
            }
            requireKind(result, SmithyContract.RESULT_OK, "NAMESPACE_DELETE")
            NamespaceDeleteOutput()
        }

    @Synchronized
    override fun close() {
        if (closed) return
        closed = true
        handle?.let(native::openkache_client_free)
        handle = null
    }

    private fun invoke(
        operation: Int,
        applicationKey: ByteArray,
        value: ByteArray,
        setCondition: Int = SmithyContract.SET_CONDITION_ANY,
        ttlMilliseconds: Long = 0,
    ): NativeResult {
        synchronized(this) {
            val client = requireHandle()
            Buffer(applicationKey).use { key ->
                Buffer(value).use { payload ->
                    return readResult(
                        native,
                        native.openkache_client_execute(
                            client,
                            operation,
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

    private fun invokeScoped(
        operation: Int,
        namespaceId: Long,
        itemId: ByteArray,
        value: ByteArray,
        flags: Int = 0,
        ttlMilliseconds: Long = 0,
    ): NativeResult {
        if ((operation == SmithyContract.OPERATION_GET
                || operation == SmithyContract.OPERATION_SET
                || operation == SmithyContract.OPERATION_DELETE)
            && itemId.size != SmithyContract.ITEM_ID_BYTES
        ) {
            throw IllegalArgumentException("itemId must contain exactly ${SmithyContract.ITEM_ID_BYTES} bytes")
        }
        if (itemId.isNotEmpty()
            && operation != SmithyContract.OPERATION_GET
            && operation != SmithyContract.OPERATION_SET
            && operation != SmithyContract.OPERATION_DELETE
        ) {
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

    private fun setFlags(input: SetInput): SetFlags {
        var flags = when (input.condition ?: SetCondition.Any) {
            SetCondition.Any -> SmithyContract.SET_CONDITION_ANY
            SetCondition.IfAbsent -> SmithyContract.SET_CONDITION_IF_ABSENT
            SetCondition.IfPresent -> SmithyContract.SET_CONDITION_IF_PRESENT
        }
        val expiration = input.expirationMode
            ?: if (input.ttlMilliseconds == null) ExpirationMode.Inherit else ExpirationMode.ExplicitTtl
        when (expiration) {
            ExpirationMode.Inherit -> {
                require(input.ttlMilliseconds == null) { "INHERIT cannot carry a TTL" }
                flags = flags or SmithyContract.SET_INHERIT_EXPIRATION_BITS
            }
            ExpirationMode.NoExpiry -> {
                require(input.ttlMilliseconds == null) { "NO_EXPIRY cannot carry a TTL" }
                flags = flags or SmithyContract.SET_NO_EXPIRY_BITS
            }
            ExpirationMode.ExplicitTtl -> {
                require(input.ttlMilliseconds != null && input.ttlMilliseconds > 0) {
                    "EXPLICIT_TTL requires a positive TTL"
                }
                flags = flags or SmithyContract.SET_EXPLICIT_TTL_BITS
            }
        }
        flags = flags or when (input.evictionMode ?: EvictionMode.Inherit) {
            EvictionMode.Inherit -> SmithyContract.SET_INHERIT_EVICTION_BITS
            EvictionMode.Evictable -> SmithyContract.SET_EVICTABLE_BITS
            EvictionMode.EvictionProtected -> SmithyContract.SET_EVICTION_PROTECTED_BITS
        }
        require(input.value.size <= SmithyContract.MAX_VALUE_BYTES) { "value exceeds protocol limit" }
        return SetFlags(flags, input.ttlMilliseconds ?: 0)
    }

    private fun policyFlags(policy: NamespacePolicy?, required: Boolean): PolicyFlags {
        if (required) requireNotNull(policy) { "namespace policy is required" }
        if (!required) require(policy == null) { "namespace policy requires createIfMissing" }
        if (policy == null) return PolicyFlags(0, 0)
        var flags = when (policy.defaultExpiration) {
            ExpirationDefault.NoExpiry -> SmithyContract.POLICY_NO_EXPIRY_BITS
            ExpirationDefault.FixedTtl -> SmithyContract.POLICY_FIXED_TTL_BITS
        }
        val ttl = policy.defaultTtlMilliseconds ?: 0
        if (policy.defaultExpiration == ExpirationDefault.FixedTtl) {
            require(ttl > 0) { "FIXED_TTL requires a positive TTL" }
        } else {
            require(ttl == 0L) { "NO_EXPIRY cannot carry a TTL" }
        }
        if (policy.expirationOverride == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EXPIRATION_OVERRIDE_FLAG
        }
        if (policy.defaultEviction == EvictionDefault.EvictionProtected) {
            flags = flags or SmithyContract.POLICY_EVICTION_PROTECTED_FLAG
        }
        if (policy.evictionOverride == OverridePolicy.Allowed) {
            flags = flags or SmithyContract.POLICY_EVICTION_OVERRIDE_FLAG
        }
        return PolicyFlags(flags, ttl)
    }

    private fun decodeDescriptor(payload: ByteArray): NamespaceDescriptor {
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

    private fun decodeUtf8(payload: ByteArray): String {
        return try {
            StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(payload))
                .toString()
        } catch (error: CharacterCodingException) {
            throw EchoClientException("native response is not valid UTF-8", error)
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

    private data class NativeResult(val kind: Int, val payload: ByteArray, val client: Pointer? = null)
    private data class SetFlags(val flags: Int, val ttlMilliseconds: Long)
    private data class PolicyFlags(val flags: Int, val ttlMilliseconds: Long)

    companion object {
        /**
         * Connects to an OpenKache server through the shared native client core.
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
                    SmithyNativeApi::class.java,
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
                            return EchoClient(
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
                    throw EchoClientException(
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
            EchoClientException("$operation returned unexpected native result $kind")
    }
}

private class EchoClientException(message: String, cause: Throwable? = null) :
    RuntimeException(message, cause)
