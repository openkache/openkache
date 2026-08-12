/** Protocol-v1 compatibility compact optional-value size planning. */

/** Width of each big-endian optional-value length/sentinel prefix. */
export const OPTIONAL_VALUE_LENGTH_BYTES = 4

/** Sentinel representing a missing value in the historical response table. */
export const OPTIONAL_VALUE_MISSING = 0xffff_ffff

function bounded_add(value: number, increment: number): number {
  if (
    !Number.isSafeInteger(value) ||
    value < 0 ||
    !Number.isSafeInteger(increment) ||
    increment < 0
  ) {
    throw new Error("optional-value size arithmetic requires safe non-negative integers")
  }
  if (value > Number.MAX_SAFE_INTEGER - increment) {
    throw new Error("optional-value payload size exceeds the safe integer range")
  }
  return value + increment
}

/** Computes a protocol-v1 optional-value payload size without allocating. */
export function optional_values_encoded_len_from_lengths(
  lengths: readonly (number | undefined)[],
): number {
  let encoded_length = 0
  for (const length of lengths) {
    if (
      length !== undefined &&
      (!Number.isSafeInteger(length) || length < 0 ||
        length >= OPTIONAL_VALUE_MISSING)
    ) {
      throw new Error(`optional value length is outside the u32 range: ${length}`)
    }
    encoded_length = bounded_add(
      encoded_length,
      OPTIONAL_VALUE_LENGTH_BYTES + (length ?? 0),
    )
  }
  return encoded_length
}
