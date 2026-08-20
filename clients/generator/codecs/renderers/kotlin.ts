/** kotlin field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_kotlin_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `    private fun smithyEncodeFieldVarUInt(value: Long): ByteArray {
        require(value >= 0) { "field length is negative" }
        if (value < 0x80L) return byteArrayOf(value.toByte())
        if (value < 0x4000L) return byteArrayOf(
            (0x80L or (value and 0x3fL)).toByte(), (value shr 6).toByte())
        if (value < 0x200000L) return byteArrayOf(
            (0xc0L or (value and 0x1fL)).toByte(), (value shr 5).toByte(),
            (value shr 13).toByte())
        if (value < 0x10000000L) return byteArrayOf(
            (0xe0L or (value and 0x0fL)).toByte(), (value shr 4).toByte(),
            (value shr 12).toByte(), (value shr 20).toByte())
        var width = 8
        while (width > 1 && (value ushr ((width - 1) * 8)) == 0L) width--
        return ByteArray(width + 1).also { output ->
            output[0] = (0xf0 or (width - 1)).toByte()
            repeat(width) { index -> output[index + 1] = (value ushr (index * 8)).toByte() }
        }
    }

    private fun smithyDecodeFieldVarUInt(payload: ByteArray, cursor: IntArray, operation: String): Long {
        val start = cursor[0]
        require(start < payload.size) { "\$operation field length is truncated" }
        val first = payload[start].toInt() and 0xff
        val width = when {
            first < 0x80 -> 1
            first < 0xc0 -> 2
            first < 0xe0 -> 3
            first < 0xf0 -> 4
            else -> (first and 0x0f) + 2
        }
        require(width <= 9 && start + width <= payload.size) {
            "\$operation field length is truncated"
        }
        val value = when (width) {
            1 -> first.toLong()
            2 -> (first and 0x3f).toLong() or ((payload[start + 1].toLong() and 0xff) shl 6)
            3 -> (first and 0x1f).toLong() or
                ((payload[start + 1].toLong() and 0xff) shl 5) or
                ((payload[start + 2].toLong() and 0xff) shl 13)
            4 -> (first and 0x0f).toLong() or
                ((payload[start + 1].toLong() and 0xff) shl 4) or
                ((payload[start + 2].toLong() and 0xff) shl 12) or
                ((payload[start + 3].toLong() and 0xff) shl 20)
            else -> (1 until width).fold(0L) { result, index ->
                result or ((payload[start + index].toLong() and 0xff) shl ((index - 1) * 8))
            }
        }
        require(value >= 0 && smithyEncodeFieldVarUInt(value).size == width) {
            "\$operation field length is non-canonical"
        }
        cursor[0] = start + width
        return value
    }

    private fun smithyEncodeFieldSequence(values: List<ByteArray?>): ByteArray {
        val maskBytes = (values.size + 7) / 8
        val lastPresent = values.indexOfLast { it != null }
        var total = maskBytes
        values.forEachIndexed { index, value ->
            if (value != null) {
                require(value.size <= ${framing.max_value_bytes}) {
                    "field-sequence entry exceeds the maximum value size"
                }
                if (index != lastPresent) {
                    total = Math.addExact(total, smithyEncodeFieldVarUInt(value.size.toLong()).size)
                }
                total = Math.addExact(total, value.size)
            }
        }
        require(total <= ${framing.max_value_bytes}) {
            "field-sequence payload exceeds the maximum value size"
        }
        val buffer = ByteBuffer.allocate(total)
        var offset = maskBytes
        values.forEachIndexed { index, value ->
            if (value != null) {
                buffer.put(index / 8, (buffer.get(index / 8).toInt() or (1 shl (index % 8))).toByte())
                if (index != lastPresent) {
                    val encodedLength = smithyEncodeFieldVarUInt(value.size.toLong())
                    buffer.position(offset)
                    buffer.put(encodedLength)
                    offset += encodedLength.size
                }
                buffer.position(offset)
                buffer.put(value)
                offset += value.size
            }
        }
        return buffer.array()
    }

    private fun smithyDecodeFieldSequence(payload: ByteArray, fieldCount: Int, operation: String): Array<ByteArray?> {
        val values = arrayOfNulls<ByteArray>(fieldCount)
        val maskBytes = (fieldCount + 7) / 8
        require(payload.size >= maskBytes) {
            "\$operation field sequence is missing its presence mask"
        }
        if (maskBytes > 0 && fieldCount % 8 != 0) {
            require(
                (payload[maskBytes - 1].toInt() and 0xff and
                    ((1 shl (fieldCount % 8)) - 1).inv()) == 0
            ) {
                "\$operation field sequence presence mask has unused bits set"
            }
        }
        val lastPresent = (fieldCount - 1 downTo 0).firstOrNull { index ->
            (payload[index / 8].toInt() and (1 shl (index % 8))) != 0
        } ?: -1
        val cursor = intArrayOf(maskBytes)
        for (index in 0 until fieldCount) {
            if ((payload[index / 8].toInt() and (1 shl (index % 8))) == 0) continue
            val length = if (index == lastPresent) {
                (payload.size - cursor[0]).toLong()
            } else {
                smithyDecodeFieldVarUInt(payload, cursor, operation)
            }
            require(length <= ${framing.max_value_bytes}L && length <= Int.MAX_VALUE) {
                "\$operation field sequence entry exceeds the maximum value size"
            }
            val end = Math.addExact(cursor[0], length.toInt())
            require(end <= payload.size) { "\$operation field sequence entry is truncated" }
            values[index] = payload.copyOfRange(cursor[0], end)
            cursor[0] = end
        }
        require(cursor[0] == payload.size) { "\$operation field sequence contains trailing bytes" }
        return values
    }

    private fun smithyEncodeDenseFields(values: List<ByteArray>): ByteArray {
        val total = values.sumOf { it.size }
        require(total <= ${framing.max_value_bytes}) {
            "dense field payload exceeds the maximum value size"
        }
        val output = ByteArray(total)
        var offset = 0
        values.forEach { value ->
            value.copyInto(output, offset)
            offset += value.size
        }
        return output
    }

    private fun smithyDecodeDenseFields(
        payload: ByteArray,
        widths: IntArray,
        operation: String,
    ): Array<ByteArray?> {
        val values = arrayOfNulls<ByteArray>(widths.size)
        var offset = 0
        widths.forEachIndexed { index, width ->
            require(width >= 0 && width <= payload.size - offset) {
                "\$operation dense field payload is truncated"
            }
            values[index] = payload.copyOfRange(offset, offset + width)
            offset += width
        }
        require(offset == payload.size) { "\$operation dense field payload has trailing bytes" }
        return values
    }

    private fun smithyEncodeU64(value: Long): ByteArray =
        ByteBuffer.allocate(java.lang.Long.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putLong(value).array()

    private fun smithyDecodeU64(payload: ByteArray, operation: String): Long {
        require(payload.size == java.lang.Long.BYTES) {
            "\$operation response has an invalid u64 field"
        }
        return ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).long
    }

    private fun smithyEncodeBool(value: Boolean): ByteArray =
        byteArrayOf(if (value) 1 else 0)

    private fun smithyDecodeBool(payload: ByteArray, operation: String): Boolean {
        require(payload.size == 1 && (payload[0].toInt() == 0 || payload[0].toInt() == 1)) {
            "\$operation response has an invalid boolean field"
        }
        return payload[0].toInt() == 1
    }

    private fun smithyEncodeF64(value: Double): ByteArray {
        require(value.isFinite()) { "binary64 field must be finite" }
        return ByteBuffer.allocate(java.lang.Double.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putDouble(value).array()
    }

    private fun smithyDecodeF64(payload: ByteArray, operation: String): Double {
        require(payload.size == java.lang.Double.BYTES) {
            "\$operation response has an invalid f64 field"
        }
        return ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).double.also { value ->
            require(value.isFinite()) {
                "\$operation response contains a non-finite f64 field"
            }
        }
    }

    private fun smithyEncodeI32(value: Int): ByteArray =
        ByteBuffer.allocate(java.lang.Integer.BYTES).order(ByteOrder.BIG_ENDIAN)
            .putInt(value).array()

    private fun smithyDecodeI32(payload: ByteArray, operation: String): Int {
        require(payload.size == java.lang.Integer.BYTES) {
            "\$operation response has an invalid i32 field"
        }
        return ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN).int
    }
`
}

export function render_kotlin_container_helpers(max_value_bytes: number): string {
  return `    private fun smithyEncodeVarUInt(value: Long): ByteArray {
        require(value >= 0) { "container count is negative" }
        if (value < 0x80) return byteArrayOf(value.toByte())
        if (value < 0x4000) return byteArrayOf(
            (0x80 or (value and 0x3f).toByte().toInt()).toByte(),
            (value ushr 6).toByte(),
        )
        if (value < 0x200000) return byteArrayOf(
            (0xc0 or (value and 0x1f).toByte().toInt()).toByte(),
            (value ushr 5).toByte(),
            (value ushr 13).toByte(),
        )
        if (value < 0x10000000) return byteArrayOf(
            (0xe0 or (value and 0x0f).toByte().toInt()).toByte(),
            (value ushr 4).toByte(),
            (value ushr 12).toByte(),
            (value ushr 20).toByte(),
        )
        var width = 8
        while (width > 1 && (value ushr ((width - 1) * 8)) == 0L) width--
        return ByteArray(width + 1).also { result ->
            result[0] = (0xf0 or (width - 1)).toByte()
            repeat(width) { index -> result[index + 1] = (value ushr (index * 8)).toByte() }
        }
    }

    private fun smithyDecodeVarUInt(payload: ByteArray, cursor: IntArray, operation: String): Long {
        val start = cursor[0]
        require(start < payload.size) { "\$operation container count is truncated" }
        val first = payload[start].toInt() and 0xff
        val width = when {
            first < 0x80 -> 1
            first < 0xc0 -> 2
            first < 0xe0 -> 3
            first < 0xf0 -> 4
            else -> (first and 0x0f) + 2
        }
        require(width <= 9 && start + width <= payload.size) {
            "\$operation container count is truncated"
        }
        val value = when (width) {
            1 -> first.toLong()
            2 -> (first and 0x3f).toLong() or ((payload[start + 1].toInt() and 0xff).toLong() shl 6)
            3 -> (first and 0x1f).toLong() or
                ((payload[start + 1].toInt() and 0xff).toLong() shl 5) or
                ((payload[start + 2].toInt() and 0xff).toLong() shl 13)
            4 -> (first and 0x0f).toLong() or
                ((payload[start + 1].toInt() and 0xff).toLong() shl 4) or
                ((payload[start + 2].toInt() and 0xff).toLong() shl 12) or
                ((payload[start + 3].toInt() and 0xff).toLong() shl 20)
            else -> (1 until width).fold(0L) { result, index ->
                result or ((payload[start + index].toLong() and 0xffL) shl ((index - 1) * 8))
            }
        }
        require(smithyEncodeVarUInt(value).size == width) {
            "\$operation container count is non-canonical"
        }
        cursor[0] = start + width
        return value
    }

    private fun smithyEncodeLengthDelimited(value: ByteArray): ByteArray {
        require(value.size <= ${max_value_bytes}) { "container entry exceeds the maximum value size" }
        return ByteBuffer.allocate(4 + value.size).order(ByteOrder.BIG_ENDIAN)
            .putInt(value.size).put(value).array()
    }

    private fun smithyReadLengthDelimited(payload: ByteArray, cursor: IntArray, operation: String): ByteArray {
        val start = cursor[0]
        require(start <= payload.size - 4) { "\$operation container entry length is truncated" }
        val length = ByteBuffer.wrap(payload, start, 4).order(ByteOrder.BIG_ENDIAN).int
        require(length >= 0 && length <= ${max_value_bytes} && length <= payload.size - start - 4) {
            "\$operation container entry is malformed"
        }
        cursor[0] = start + 4 + length
        return payload.copyOfRange(start + 4, cursor[0])
    }

    private fun smithyJoinContainer(chunks: List<ByteArray>): ByteArray {
        val result = ByteArray(chunks.sumOf { it.size })
        var offset = 0
        chunks.forEach { chunk ->
            chunk.copyInto(result, offset)
            offset += chunk.size
        }
        return result
    }

    private fun smithyEncodeList(values: List<ByteArray>): ByteArray =
        smithyJoinContainer(listOf(smithyEncodeVarUInt(values.size.toLong())) +
            values.map(::smithyEncodeLengthDelimited))

    private fun smithyDecodeList(payload: ByteArray, operation: String): List<ByteArray> {
        val cursor = intArrayOf(0)
        val count = smithyDecodeVarUInt(payload, cursor, operation)
        require(count <= Int.MAX_VALUE) { "\$operation list is too large" }
        val values = (0 until count.toInt()).map {
            smithyReadLengthDelimited(payload, cursor, operation)
        }
        require(cursor[0] == payload.size) { "\$operation list has trailing bytes" }
        return values
    }

    private fun smithyEncodeMap(values: List<List<ByteArray>>): ByteArray =
        smithyJoinContainer(
            listOf(smithyEncodeVarUInt(values.size.toLong())) +
                values.flatMap { entry ->
                    require(entry.size == 2) { "map entry must contain key and value" }
                    listOf(smithyEncodeLengthDelimited(entry[0]), smithyEncodeLengthDelimited(entry[1]))
                },
        )

    private fun smithyDecodeMap(payload: ByteArray, operation: String): List<List<ByteArray>> {
        val cursor = intArrayOf(0)
        val count = smithyDecodeVarUInt(payload, cursor, operation)
        require(count <= Int.MAX_VALUE) { "\$operation map is too large" }
        val values = (0 until count.toInt()).map {
            listOf(
                smithyReadLengthDelimited(payload, cursor, operation),
                smithyReadLengthDelimited(payload, cursor, operation),
            )
        }
        require(cursor[0] == payload.size) { "\$operation map has trailing bytes" }
        return values
    }

    private fun smithyEncodeUnion(payload: ByteArray, operation: String): ByteArray =
        smithyDecodeUnion(payload, operation)

    private fun smithyDecodeUnion(payload: ByteArray, operation: String): ByteArray {
        require(payload.size >= 5) { "\$operation union payload is truncated" }
        val cursor = intArrayOf(1)
        smithyReadLengthDelimited(payload, cursor, operation)
        require(cursor[0] == payload.size) { "\$operation union payload has trailing bytes" }
        return payload
    }
`
}

/** Shared Dart helpers for ordered field-sequence framing and scalar codecs. */
