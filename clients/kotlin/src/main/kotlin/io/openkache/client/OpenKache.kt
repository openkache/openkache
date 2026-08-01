package io.openkache.client

import io.openkache.client.nativebridge.NativeOpenKacheClient
import java.util.concurrent.CompletableFuture

/**
 * Kotlin async facade over the Java Panama binding.
 *
 * The returned futures are cancellation-aware at the language boundary; the
 * shared native worker remains responsible for request and lane cleanup.
 */
class OpenKacheClient private constructor(
    private val delegate: NativeOpenKacheClient,
) : AutoCloseable {
    companion object {
        @JvmStatic
        fun connectAsync(options: Options): CompletableFuture<OpenKacheClient> =
            NativeOpenKacheClient.connectAsync(options.toJava()).thenApply(::OpenKacheClient)
    }

    fun ping(): CompletableFuture<Void> = delegate.ping()

    fun get(key: ByteArray): CompletableFuture<ByteArray?> = delegate.get(key)

    /** Retrieves a canonical JSON document, or null when absent. */
    fun getJson(key: ByteArray): CompletableFuture<String?> = delegate.getJson(key)

    /** Retrieves exact bytes for a protocol item ID. */
    fun getRaw(itemId: ByteArray): CompletableFuture<ByteArray?> = delegate.getRaw(itemId)

    fun set(
        key: ByteArray,
        value: ByteArray,
        options: SetOptions = SetOptions(),
    ): CompletableFuture<NativeOpenKacheClient.SetOutcome> =
        delegate.set(key, value, options.toJava())

    /** Stores one canonical JSON document. */
    fun setJson(
        key: ByteArray,
        json: String,
        options: SetOptions = SetOptions(),
    ): CompletableFuture<NativeOpenKacheClient.SetOutcome> =
        delegate.setJson(key, json, options.toJava())

    /** Stores exact bytes for a protocol item ID. */
    fun setRaw(
        itemId: ByteArray,
        value: ByteArray,
        options: SetOptions = SetOptions(),
    ): CompletableFuture<NativeOpenKacheClient.SetOutcome> =
        delegate.setRaw(itemId, value, options.toJava())

    fun delete(
        key: ByteArray,
        options: SetOptions = SetOptions(),
    ): CompletableFuture<Boolean> = delegate.delete(key, options.toJava())

    fun deleteRaw(
        itemId: ByteArray,
        options: SetOptions = SetOptions(),
    ): CompletableFuture<Boolean> = delegate.deleteRaw(itemId, options.toJava())

    fun stats(): CompletableFuture<String> = delegate.stats()

    fun sync(): CompletableFuture<Void> = delegate.sync()

    fun reconnect(): CompletableFuture<Void> = delegate.reconnect()

    fun cancel(requestId: Long): Boolean = delegate.cancel(requestId)

    fun connectionState(): Int = delegate.connectionState()

    fun metricsSnapshot(): MetricsSnapshot =
        delegate.metricsSnapshot().let {
            MetricsSnapshot(
                it.requests(),
                it.hits(),
                it.misses(),
                it.retries(),
                it.reconnects(),
                it.cancellations(),
                it.transportErrors(),
                it.protocolErrors(),
                it.bytesSent(),
                it.bytesReceived(),
                it.activeLanes(),
            )
        }

    override fun close() {
        delegate.close()
    }
}

/** Immutable Kotlin connection settings. */
data class Options(
    val address: String,
    val serverName: String = "localhost",
    val certificate: ByteArray = byteArrayOf(),
    val dataProtectionKey: ByteArray = byteArrayOf(),
    val clientCertificateChain: ByteArray = byteArrayOf(),
    val clientPrivateKey: ByteArray = byteArrayOf(),
    val compressionEnabled: Boolean = false,
    val compressionLevel: Int = 1,
    val minimumInputBytes: Int = 1024,
    val minimumSavingsBytes: Int = 64,
    val encryption: Int = 2,
    val retryMaxAttempts: Int = 2,
    val maxInFlight: Int = 256,
    val connectTimeoutMillis: Long = 5000,
    val requestTimeoutMillis: Long = 2000,
    val previousDataProtectionKeys: Array<ByteArray> = emptyArray(),
    val keyRing: DataProtectionKeyRing? = null,
) {
    internal fun toJava(): NativeOpenKacheClient.Options {
        require(keyRing != null || dataProtectionKey.isNotEmpty()) {
            "provide either dataProtectionKey or keyRing"
        }
        require(keyRing == null || dataProtectionKey.isEmpty()) {
            "dataProtectionKey cannot be combined with keyRing"
        }
        return NativeOpenKacheClient.Options(
            address,
            serverName,
            certificate,
            keyRing?.active ?: dataProtectionKey,
        clientCertificateChain,
        clientPrivateKey,
        compressionEnabled,
        compressionLevel,
        minimumInputBytes,
        minimumSavings,
        encryption,
        retryMaxAttempts,
        maxInFlight,
        connectTimeoutMillis,
        requestTimeoutMillis,
        keyRing?.previous ?: previousDataProtectionKeys,
        )
    }
}

/** Active data-protection key and up to eight retired keys. */
data class DataProtectionKeyRing(
    val active: ByteArray,
    val previous: Array<ByteArray> = emptyArray(),
)

/** Kotlin mutation options matching the shared Smithy condition and token contract. */
data class SetOptions(
    val condition: Int = 0,
    val ttlMillis: Long = 0,
    val mutationId: ByteArray = byteArrayOf(),
) {
    internal fun toJava(): NativeOpenKacheClient.SetOptions =
        NativeOpenKacheClient.SetOptions(condition, ttlMillis, mutationId)
}

/** Point-in-time native request and transport counters. */
data class MetricsSnapshot(
    val requests: Long,
    val hits: Long,
    val misses: Long,
    val retries: Long,
    val reconnects: Long,
    val cancellations: Long,
    val transportErrors: Long,
    val protocolErrors: Long,
    val bytesSent: Long,
    val bytesReceived: Long,
    val activeLanes: Long,
)

/** Structured metadata attached to a native operation failure. */
typealias ErrorMetadata = NativeOpenKacheClient.ErrorMetadata

/** Native operation failure with retry and ambiguity metadata. */
typealias OpenKacheException = NativeOpenKacheClient.OpenKacheException
