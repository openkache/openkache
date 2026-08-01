// Generated from the OpenKache Smithy contract. Do not edit.
package io.openkache.client.generated

/** Smithy operation types for Kotlin adapters. */
object SmithyApi {
    enum class SetCondition(val value: String) {
        IfAbsent("if_absent"),
        IfPresent("if_present")
    }

    enum class SetOutcome(val value: String) {
        Created("created"),
        Replaced("replaced"),
        NotStored("not_stored")
    }

    data class DeleteInput(
        val itemId: ByteArray,
        val mutationId: ByteArray?
    )

    data class DeleteOutput(
        val deleted: Boolean
    )

    data class GetInput(
        val itemId: ByteArray
    )

    data class GetOutput(
        val value: ByteArray?
    )

    class PingInput

    class PingOutput

    data class SetInput(
        val itemId: ByteArray,
        val value: ByteArray,
        val condition: String?,
        val ttlMilliseconds: Long?,
        val mutationId: ByteArray?
    )

    data class SetOutput(
        val outcome: String
    )

    class StatsInput

    data class StatsOutput(
        val json: String
    )

    class SyncInput

    class SyncOutput

    interface OpenKacheApi {
        suspend fun ping(input: PingInput): PingOutput
        suspend fun get(input: GetInput): GetOutput
        suspend fun set(input: SetInput): SetOutput
        suspend fun delete(input: DeleteInput): DeleteOutput
        suspend fun stats(input: StatsInput): StatsOutput
        suspend fun sync(input: SyncInput): SyncOutput
    }
}
