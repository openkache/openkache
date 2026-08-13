/** swift field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_swift_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `private func smithyEncodeFieldVarUInt(_ value: UInt64) -> Data {
  if value < 0x80 { return Data([UInt8(value)]) }
  if value < 0x4000 {
    return Data([UInt8(0x80 | (value & 0x3f)), UInt8(value >> 6)])
  }
  if value < 0x200000 {
    return Data([
      UInt8(0xc0 | (value & 0x1f)), UInt8(value >> 5), UInt8(value >> 13)
    ])
  }
  if value < 0x10000000 {
    return Data([
      UInt8(0xe0 | (value & 0x0f)), UInt8(value >> 4),
      UInt8(value >> 12), UInt8(value >> 20)
    ])
  }
  var width = 8
  while width > 1 && (value >> UInt64((width - 1) * 8)) == 0 { width -= 1 }
  var bytes = [UInt8](repeating: 0, count: width + 1)
  bytes[0] = UInt8(0xf0 | (width - 1))
  for index in 0..<width {
    bytes[index + 1] = UInt8(value >> UInt64(index * 8))
  }
  return Data(bytes)
}

private func smithyDecodeFieldVarUInt(
  _ payload: Data,
  _ offset: inout Int,
  operation: String
) throws -> UInt64 {
  let bytes = [UInt8](payload)
  guard offset < bytes.count else {
    throw OpenKacheError("\(operation) field length is truncated")
  }
  let first = bytes[offset]
  let width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3 :
    first < 0xf0 ? 4 : Int(first & 0x0f) + 2
  guard width <= 9, offset + width <= bytes.count else {
    throw OpenKacheError("\(operation) field length is truncated")
  }
  var value: UInt64 = 0
  if width == 1 {
    value = UInt64(first)
  } else if width == 2 {
    value = UInt64(first & 0x3f) | UInt64(bytes[offset + 1]) << 6
  } else if width == 3 {
    value = UInt64(first & 0x1f) |
      UInt64(bytes[offset + 1]) << 5 |
      UInt64(bytes[offset + 2]) << 13
  } else if width == 4 {
    value = UInt64(first & 0x0f) |
      UInt64(bytes[offset + 1]) << 4 |
      UInt64(bytes[offset + 2]) << 12 |
      UInt64(bytes[offset + 3]) << 20
  } else {
    for index in 1..<width {
      value |= UInt64(bytes[offset + index]) << UInt64((index - 1) * 8)
    }
  }
  guard smithyEncodeFieldVarUInt(value).count == width else {
    throw OpenKacheError("\(operation) field length is non-canonical")
  }
  offset += width
  return value
}

private func smithyEncodeFieldSequence(_ values: [Data?]) throws -> Data {
  let maskBytes = (values.count + 7) / 8
  let lastPresent = values.indices.reversed().first { values[$0] != nil } ?? -1
  var payload = Data(repeating: 0, count: maskBytes)
  for (index, value) in values.enumerated() {
    if let value {
      guard value.count <= ${framing.max_value_bytes} else {
        throw OpenKacheError("field-sequence entry exceeds the maximum value size")
      }
      let encodedLength = index == lastPresent
        ? Data()
        : smithyEncodeFieldVarUInt(UInt64(value.count))
      guard payload.count + encodedLength.count + value.count <= ${framing.max_value_bytes} else {
        throw OpenKacheError("field-sequence payload exceeds the maximum value size")
      }
      payload[index / 8] |= UInt8(1 << (index % 8))
      payload.append(encodedLength)
      payload.append(value)
    }
  }
  return payload
}

private func smithyDecodeFieldSequence(
  _ payload: Data,
  fieldCount: Int,
  operation: String
) throws -> [Data?] {
  let bytes = [UInt8](payload)
  var values = [Data?](repeating: nil, count: fieldCount)
  let maskBytes = (fieldCount + 7) / 8
  guard bytes.count >= maskBytes else {
    throw OpenKacheError("\(operation) field sequence is missing its presence mask")
  }
  if maskBytes > 0 && fieldCount % 8 != 0 {
    let unused = bytes[maskBytes - 1] & ~UInt8((1 << (fieldCount % 8)) - 1)
    guard unused == 0 else {
      throw OpenKacheError("\(operation) field sequence presence mask has unused bits set")
    }
  }
  let lastPresent = (0..<fieldCount).reversed().first { index in
    bytes[index / 8] & UInt8(1 << (index % 8)) != 0
  } ?? -1
  var offset = maskBytes
  for index in 0..<fieldCount {
    if bytes[index / 8] & UInt8(1 << (index % 8)) == 0 { continue }
    let length: UInt64
    if index == lastPresent {
      length = UInt64(bytes.count - offset)
    } else {
      length = try smithyDecodeFieldVarUInt(payload, &offset, operation: operation)
    }
    guard length <= ${framing.max_value_bytes}, length <= UInt64(Int.max) else {
      throw OpenKacheError("\(operation) field sequence entry exceeds the maximum value size")
    }
    let end = offset + Int(length)
    guard end <= bytes.count else {
      throw OpenKacheError("\(operation) field sequence entry is truncated")
    }
    values[index] = Data(bytes[offset..<end])
    offset = end
  }
  guard offset == bytes.count else {
    throw OpenKacheError("\(operation) field sequence contains trailing bytes")
  }
  return values
}

private func smithyEncodeDenseFields(_ values: [Data]) throws -> Data {
  let total = values.reduce(0) { $0 + $1.count }
  guard total <= ${framing.max_value_bytes} else {
    throw OpenKacheError("dense field payload exceeds the maximum value size")
  }
  var payload = Data()
  payload.reserveCapacity(total)
  values.forEach { payload.append($0) }
  return payload
}

private func smithyDecodeDenseFields(
  _ payload: Data,
  widths: [Int],
  operation: String
) throws -> [Data?] {
  var values = [Data?](repeating: nil, count: widths.count)
  var offset = 0
  for (index, width) in widths.enumerated() {
    guard width >= 0, width <= payload.count - offset else {
      throw OpenKacheError("\(operation) dense field payload is truncated")
    }
    values[index] = Data(payload[offset..<(offset + width)])
    offset += width
  }
  guard offset == payload.count else {
    throw OpenKacheError("\(operation) dense field payload has trailing bytes")
  }
  return values
}

private func smithyEncodeU64(_ value: UInt64) -> Data {
  var encoded = value.bigEndian
  return Data(bytes: &encoded, count: MemoryLayout<UInt64>.size)
}

private func smithyDecodeU64(_ payload: Data, operation: String) throws -> UInt64 {
  guard payload.count == MemoryLayout<UInt64>.size else {
    throw OpenKacheError("\(operation) response has an invalid u64 field")
  }
  return payload.withUnsafeBytes { pointer in
    UInt64(bigEndian: pointer.load(as: UInt64.self))
  }
}

private func smithyEncodeBool(_ value: Bool) -> Data {
  Data([value ? 1 : 0])
}

private func smithyDecodeBool(_ payload: Data, operation: String) throws -> Bool {
  guard payload.count == 1, payload[0] == 0 || payload[0] == 1 else {
    throw OpenKacheError("\(operation) response has an invalid boolean field")
  }
  return payload[0] == 1
}

private func smithyEncodeF64(_ value: Double) throws -> Data {
  guard value.isFinite else {
    throw OpenKacheError("binary64 field must be finite")
  }
  var encoded = value.bitPattern.bigEndian
  return Data(bytes: &encoded, count: MemoryLayout<UInt64>.size)
}

private func smithyDecodeF64(_ payload: Data, operation: String) throws -> Double {
  guard payload.count == MemoryLayout<UInt64>.size else {
    throw OpenKacheError("\(operation) response has an invalid f64 field")
  }
  let bits = payload.withUnsafeBytes { pointer in
    UInt64(bigEndian: pointer.load(as: UInt64.self))
  }
  let value = Double(bitPattern: bits)
  guard value.isFinite else {
    throw OpenKacheError("\(operation) response contains a non-finite f64 field")
  }
  return value
}

private func smithyEncodeI32(_ value: Int32) -> Data {
  var encoded = value.bigEndian
  return Data(bytes: &encoded, count: MemoryLayout<Int32>.size)
}

private func smithyDecodeI32(_ payload: Data, operation: String) throws -> Int32 {
  guard payload.count == MemoryLayout<Int32>.size else {
    throw OpenKacheError("\(operation) response has an invalid i32 field")
  }
  return payload.withUnsafeBytes { pointer in
    Int32(bigEndian: pointer.load(as: Int32.self))
  }
}

private extension UInt32 {
  var bigEndianBytes: [UInt8] {
    withUnsafeBytes(of: self.bigEndian) { Array($0) }
  }
}
`
}

export function render_swift_container_helpers(max_value_bytes: number): string {
  return `private func smithyEncodeVarUInt(_ value: UInt64) -> Data {
  if value < 0x80 { return Data([UInt8(value)]) }
  if value < 0x4000 { return Data([UInt8(0x80 | (value & 0x3f)), UInt8(value >> 6)]) }
  if value < 0x200000 {
    return Data([UInt8(0xc0 | (value & 0x1f)), UInt8(value >> 5), UInt8(value >> 13)])
  }
  if value < 0x10000000 {
    return Data([UInt8(0xe0 | (value & 0x0f)), UInt8(value >> 4), UInt8(value >> 12), UInt8(value >> 20)])
  }
  var width = 8
  while width > 1 && (value >> UInt64((width - 1) * 8)) == 0 { width -= 1 }
  var bytes = [UInt8](repeating: 0, count: width + 1)
  bytes[0] = UInt8(0xf0 | (width - 1))
  for index in 0..<width { bytes[index + 1] = UInt8(value >> UInt64(index * 8)) }
  return Data(bytes)
}

private func smithyDecodeVarUInt(_ payload: Data, _ offset: inout Int, operation: String) throws -> Int {
  let bytes = [UInt8](payload)
  guard offset < bytes.count else { throw OpenKacheError("\(operation) container count is truncated") }
  let first = bytes[offset]
  let width = first < 0x80 ? 1 : first < 0xc0 ? 2 : first < 0xe0 ? 3 : first < 0xf0 ? 4 : Int(first & 0x0f) + 2
  guard width <= 9, offset + width <= bytes.count else {
    throw OpenKacheError("\(operation) container count is truncated")
  }
  var value: UInt64 = 0
  if width == 1 { value = UInt64(first) }
  else if width == 2 { value = UInt64(first & 0x3f) | UInt64(bytes[offset + 1]) << 6 }
  else if width == 3 { value = UInt64(first & 0x1f) | UInt64(bytes[offset + 1]) << 5 | UInt64(bytes[offset + 2]) << 13 }
  else if width == 4 { value = UInt64(first & 0x0f) | UInt64(bytes[offset + 1]) << 4 | UInt64(bytes[offset + 2]) << 12 | UInt64(bytes[offset + 3]) << 20 }
  else {
    for index in 1..<width { value |= UInt64(bytes[offset + index]) << UInt64((index - 1) * 8) }
  }
  guard smithyEncodeVarUInt(value).count == width, value <= UInt64(Int.max) else {
    throw OpenKacheError("\(operation) container count is non-canonical")
  }
  offset += width
  return Int(value)
}

private func smithyEncodeLengthDelimited(_ value: Data) throws -> Data {
  guard value.count <= ${max_value_bytes} else {
    throw OpenKacheError("container entry exceeds the maximum value size")
  }
  var output = smithyEncodeVarUInt(UInt64(value.count))
  output.append(value)
  return output
}

private func smithyReadLengthDelimited(_ payload: Data, _ offset: inout Int, operation: String) throws -> Data {
  let bytes = [UInt8](payload)
  let length = try smithyDecodeVarUInt(payload, &offset, operation: operation)
  guard length <= ${max_value_bytes}, length <= bytes.count - offset else {
    throw OpenKacheError("\(operation) container entry is malformed")
  }
  let start = offset
  offset = start + length
  return Data(bytes[start..<offset])
}

private func smithyEncodeList(_ values: [Data]) throws -> Data {
  var output = smithyEncodeVarUInt(UInt64(values.count))
  for value in values { output.append(try smithyEncodeLengthDelimited(value)) }
  return output
}

private func smithyDecodeList(_ payload: Data, operation: String) throws -> [Data] {
  var offset = 0
  let count = try smithyDecodeVarUInt(payload, &offset, operation: operation)
  var values: [Data] = []
  values.reserveCapacity(count)
  for _ in 0..<count { values.append(try smithyReadLengthDelimited(payload, &offset, operation: operation)) }
  guard offset == payload.count else { throw OpenKacheError("\(operation) list has trailing bytes") }
  return values
}

private func smithyEncodeMap(_ values: [(Data, Data)]) throws -> Data {
  var output = smithyEncodeVarUInt(UInt64(values.count))
  for (key, value) in values {
    output.append(try smithyEncodeLengthDelimited(key))
    output.append(try smithyEncodeLengthDelimited(value))
  }
  return output
}

private func smithyDecodeMap(_ payload: Data, operation: String) throws -> [(Data, Data)] {
  var offset = 0
  let count = try smithyDecodeVarUInt(payload, &offset, operation: operation)
  var values: [(Data, Data)] = []
  values.reserveCapacity(count)
  for _ in 0..<count {
    values.append((
      try smithyReadLengthDelimited(payload, &offset, operation: operation),
      try smithyReadLengthDelimited(payload, &offset, operation: operation)
    ))
  }
  guard offset == payload.count else { throw OpenKacheError("\(operation) map has trailing bytes") }
  return values
}

private func smithyEncodeUnion(_ payload: Data, operation: String) throws -> Data {
  try smithyDecodeUnion(payload, operation: operation)
}

private func smithyDecodeUnion(_ payload: Data, operation: String) throws -> Data {
  guard payload.count >= 2 else { throw OpenKacheError("\(operation) union payload is truncated") }
  var offset = 1
  _ = try smithyReadLengthDelimited(payload, &offset, operation: operation)
  guard offset == payload.count else { throw OpenKacheError("\(operation) union payload has trailing bytes") }
  return payload
}

private func smithyDecodeEnum<T: RawRepresentable>(_ payload: Data, _ type: T.Type, _ operation: String) throws -> T
where T.RawValue == String {
  guard let value = String(data: payload, encoding: .utf8), let result = T(rawValue: value) else {
    throw OpenKacheError("\(operation) response contains an unknown enum value")
  }
  return result
}
`
}

/** Shared .NET helpers for ordered field-sequence framing and scalar codecs. */
