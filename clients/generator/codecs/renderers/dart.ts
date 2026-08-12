/** dart field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_dart_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `List<int> _smithyEncodeFieldVarUInt(int value) {
  if (value < 0) throw const OpenKacheClientException('field length is negative');
  if (value < 0x80) return <int>[value];
  if (value < 0x4000) return <int>[0x80 | (value & 0x3f), value >> 6];
  if (value < 0x200000) {
    return <int>[0xc0 | (value & 0x1f), value >> 5, value >> 13];
  }
  if (value < 0x10000000) {
    return <int>[0xe0 | (value & 0x0f), value >> 4, value >> 12, value >> 20];
  }
  var width = 8;
  while (width > 1 && (value >> ((width - 1) * 8)) == 0) width--;
  return <int>[
    0xf0 | (width - 1),
    ...List<int>.generate(width, (index) => value >> (index * 8)),
  ];
}

(int, int) _smithyDecodeFieldVarUInt(
  List<int> payload,
  int offset,
  String operation,
) {
  if (offset >= payload.length) {
    throw OpenKacheClientException('\$operation field length is truncated');
  }
  final first = payload[offset];
  final width = first < 0x80
      ? 1
      : first < 0xc0
      ? 2
      : first < 0xe0
      ? 3
      : first < 0xf0
      ? 4
      : (first & 0x0f) + 2;
  if (width > 9 || offset + width > payload.length) {
    throw OpenKacheClientException('\$operation field length is truncated');
  }
  var value = 0;
  if (width == 1) {
    value = first;
  } else if (width == 2) {
    value = (first & 0x3f) | (payload[offset + 1] << 6);
  } else if (width == 3) {
    value = (first & 0x1f) |
        (payload[offset + 1] << 5) |
        (payload[offset + 2] << 13);
  } else if (width == 4) {
    value = (first & 0x0f) |
        (payload[offset + 1] << 4) |
        (payload[offset + 2] << 12) |
        (payload[offset + 3] << 20);
  } else {
    for (var index = 1; index < width; index++) {
      value |= payload[offset + index] << ((index - 1) * 8);
    }
  }
  if (_smithyEncodeFieldVarUInt(value).length != width) {
    throw OpenKacheClientException('\$operation field length is non-canonical');
  }
  return (value, offset + width);
}

List<int> _smithyEncodeFieldSequence(List<List<int>?> values) {
  final maskBytes = (values.length + 7) ~/ 8;
  final lastPresent = values.lastIndexWhere((value) => value != null);
  var total = maskBytes;
  for (var index = 0; index < values.length; index++) {
    final value = values[index];
    if (value != null && value.length > ${framing.max_value_bytes}) {
      throw const OpenKacheClientException(
        'field-sequence entry exceeds the maximum value size',
      );
    }
    if (value != null) {
      if (index != lastPresent) {
        total += _smithyEncodeFieldVarUInt(value.length).length;
      }
      total += value.length;
    }
  }
  if (total > ${framing.max_value_bytes}) {
    throw const OpenKacheClientException(
      'field-sequence payload exceeds the maximum value size',
    );
  }
  final output = Uint8List(total);
  var offset = maskBytes;
  for (var index = 0; index < values.length; index++) {
    final value = values[index];
    if (value != null) {
      output[index ~/ 8] |= 1 << (index % 8);
      if (index != lastPresent) {
        final encodedLength = _smithyEncodeFieldVarUInt(value.length);
        output.setRange(offset, offset + encodedLength.length, encodedLength);
        offset += encodedLength.length;
      }
      output.setRange(offset, offset + value.length, value);
      offset += value.length;
    }
  }
  return output;
}

List<List<int>?> _smithyDecodeFieldSequence(
  List<int> payload,
  int fieldCount,
  String operation,
) {
  final values = List<List<int>?>.filled(fieldCount, null);
  final maskBytes = (fieldCount + 7) ~/ 8;
  if (payload.length < maskBytes) {
    throw OpenKacheClientException('\$operation field sequence is missing its presence mask');
  }
  if (maskBytes > 0 && fieldCount % 8 != 0 &&
      (payload[maskBytes - 1] & ~((1 << (fieldCount % 8)) - 1)) != 0) {
    throw OpenKacheClientException('\$operation field sequence presence mask has unused bits set');
  }
  var lastPresent = -1;
  for (var index = fieldCount - 1; index >= 0; index--) {
    if ((payload[index ~/ 8] & (1 << (index % 8))) != 0) {
      lastPresent = index;
      break;
    }
  }
  var offset = maskBytes;
  for (var index = 0; index < fieldCount; index++) {
    if ((payload[index ~/ 8] & (1 << (index % 8))) == 0) continue;
    final (length, next) = index == lastPresent
        ? (payload.length - offset, offset)
        : _smithyDecodeFieldVarUInt(payload, offset, operation);
    if (length > ${framing.max_value_bytes} || next + length > payload.length) {
      throw OpenKacheClientException('\$operation field sequence entry is truncated');
    }
    values[index] = payload.sublist(next, next + length);
    offset = next + length;
  }
  if (offset != payload.length) {
    throw OpenKacheClientException('\$operation field sequence contains trailing bytes');
  }
  return values;
}

List<int> _smithyEncodeDenseFields(List<List<int>> values) {
  final total = values.fold<int>(0, (sum, value) => sum + value.length);
  if (total > ${framing.max_value_bytes}) {
    throw const OpenKacheClientException(
      'dense field payload exceeds the maximum value size',
    );
  }
  final output = Uint8List(total);
  var offset = 0;
  for (final value in values) {
    output.setRange(offset, offset + value.length, value);
    offset += value.length;
  }
  return output;
}

List<List<int>?> _smithyDecodeDenseFields(
  List<int> payload,
  List<int> widths,
  String operation,
) {
  final values = List<List<int>?>.filled(widths.length, null);
  var offset = 0;
  for (var index = 0; index < widths.length; index++) {
    final width = widths[index];
    if (width < 0 || width > payload.length - offset) {
      throw OpenKacheClientException(
        '\$operation dense field payload is truncated',
      );
    }
    values[index] = payload.sublist(offset, offset + width);
    offset += width;
  }
  if (offset != payload.length) {
    throw OpenKacheClientException(
      '\$operation dense field payload has trailing bytes',
    );
  }
  return values;
}

List<int> _smithyEncodeU64(int value) {
  final bytes = ByteData(8)..setUint64(0, value, Endian.big);
  return bytes.buffer.asUint8List();
}

int _smithyDecodeU64(List<int> payload, String operation) {
  if (payload.length != 8) {
    throw OpenKacheClientException(
      '\$operation response has an invalid u64 field',
    );
  }
  return ByteData.sublistView(Uint8List.fromList(payload)).getUint64(0, Endian.big);
}

List<int> _smithyEncodeBool(bool value) => <int>[value ? 1 : 0];

bool _smithyDecodeBool(List<int> payload, String operation) {
  if (payload.length != 1 || (payload[0] != 0 && payload[0] != 1)) {
    throw OpenKacheClientException(
      '\$operation response has an invalid boolean field',
    );
  }
  return payload[0] == 1;
}

List<int> _smithyEncodeF64(double value) {
  if (!value.isFinite) {
    throw const OpenKacheClientException('binary64 field must be finite');
  }
  final bytes = ByteData(8)..setFloat64(0, value, Endian.big);
  return bytes.buffer.asUint8List();
}

double _smithyDecodeF64(List<int> payload, String operation) {
  if (payload.length != 8) {
    throw OpenKacheClientException(
      '\$operation response has an invalid f64 field',
    );
  }
  final value = ByteData.sublistView(Uint8List.fromList(payload))
      .getFloat64(0, Endian.big);
  if (!value.isFinite) {
    throw OpenKacheClientException(
      '\$operation response contains a non-finite f64 field',
    );
  }
  return value;
}

List<int> _smithyEncodeI32(int value) {
  final bytes = ByteData(4)..setInt32(0, value, Endian.big);
  return bytes.buffer.asUint8List();
}

int _smithyDecodeI32(List<int> payload, String operation) {
  if (payload.length != 4) {
    throw OpenKacheClientException(
      '\$operation response has an invalid i32 field',
    );
  }
  return ByteData.sublistView(Uint8List.fromList(payload)).getInt32(0, Endian.big);
}
`
}

export function render_dart_container_helpers(max_value_bytes: number): string {
  return `List<int> _smithyEncodeVarUInt(int value) {
  if (value < 0) throw ArgumentError('container count is negative');
  if (value < 0x80) return <int>[value];
  if (value < 0x4000) return <int>[0x80 | (value & 0x3f), value >> 6];
  if (value < 0x200000) return <int>[0xc0 | (value & 0x1f), value >> 5, value >> 13];
  if (value < 0x10000000) {
    return <int>[0xe0 | (value & 0x0f), value >> 4, value >> 12, value >> 20];
  }
  var width = 8;
  while (width > 1 && (value >> ((width - 1) * 8)) == 0) {
    width--;
  }
  final result = List<int>.filled(width + 1, 0);
  result[0] = 0xf0 | (width - 1);
  for (var index = 0; index < width; index++) {
    result[index + 1] = (value >> (index * 8)) & 0xff;
  }
  return result;
}

(int, int) _smithyDecodeVarUInt(List<int> payload, int offset, String operation) {
  if (offset >= payload.length) throw OpenKacheClientException('\$operation container count is truncated');
  final first = payload[offset];
  final width = first < 0x80
      ? 1
      : first < 0xc0
      ? 2
      : first < 0xe0
      ? 3
      : first < 0xf0
      ? 4
      : (first & 0x0f) + 2;
  if (width > 9 || offset + width > payload.length) {
    throw OpenKacheClientException('\$operation container count is truncated');
  }
  var value = 0;
  if (width == 1) {
    value = first;
  } else if (width == 2) {
    value = (first & 0x3f) | (payload[offset + 1] << 6);
  } else if (width == 3) {
    value = (first & 0x1f) | (payload[offset + 1] << 5) | (payload[offset + 2] << 13);
  } else if (width == 4) {
    value = (first & 0x0f) | (payload[offset + 1] << 4) |
        (payload[offset + 2] << 12) | (payload[offset + 3] << 20);
  } else {
    for (var index = 1; index < width; index++) {
      value |= payload[offset + index] << ((index - 1) * 8);
    }
  }
  if (_smithyEncodeVarUInt(value).length != width) {
    throw OpenKacheClientException('\$operation container count is non-canonical');
  }
  return (value, offset + width);
}

List<int> _smithyEncodeLengthDelimited(List<int> value) {
  if (value.length > ${max_value_bytes}) {
    throw const OpenKacheClientException('container entry exceeds the maximum value size');
  }
  final length = ByteData(4)..setUint32(0, value.length, Endian.big);
  return <int>[...length.buffer.asUint8List(), ...value];
}

(List<int>, int) _smithyReadLengthDelimited(List<int> payload, int offset, String operation) {
  if (offset + 4 > payload.length) {
    throw OpenKacheClientException('\$operation container entry length is truncated');
  }
  final length = ByteData.sublistView(Uint8List.fromList(payload), offset, offset + 4)
      .getUint32(0, Endian.big);
  if (length == 0xffffffff || length > ${max_value_bytes} || offset + 4 + length > payload.length) {
    throw OpenKacheClientException('\$operation container entry is malformed');
  }
  return (payload.sublist(offset + 4, offset + 4 + length), offset + 4 + length);
}

List<int> _smithyJoinContainer(Iterable<List<int>> chunks) {
  final output = BytesBuilder(copy: false);
  for (final chunk in chunks) output.add(chunk);
  return output.takeBytes();
}

List<int> _smithyEncodeList(List<List<int>> values) => _smithyJoinContainer(
  <List<int>>[_smithyEncodeVarUInt(values.length), ...values.map(_smithyEncodeLengthDelimited)],
);

List<List<int>> _smithyDecodeList(List<int> payload, String operation) {
  final (count, start) = _smithyDecodeVarUInt(payload, 0, operation);
  final values = <List<int>>[];
  var offset = start;
  for (var index = 0; index < count; index++) {
    final (value, next) = _smithyReadLengthDelimited(payload, offset, operation);
    values.add(value);
    offset = next;
  }
  if (offset != payload.length) throw OpenKacheClientException('\$operation list has trailing bytes');
  return values;
}

List<int> _smithyEncodeMap(List<List<List<int>>> entries) => _smithyJoinContainer(
  <List<int>>[
    _smithyEncodeVarUInt(entries.length),
    for (final entry in entries) ...[
      _smithyEncodeLengthDelimited(entry[0]),
      _smithyEncodeLengthDelimited(entry[1]),
    ],
  ],
);

List<List<List<int>>> _smithyDecodeMap(List<int> payload, String operation) {
  final (count, start) = _smithyDecodeVarUInt(payload, 0, operation);
  final entries = <List<List<int>>>[];
  var offset = start;
  for (var index = 0; index < count; index++) {
    final (key, nextKey) = _smithyReadLengthDelimited(payload, offset, operation);
    final (value, nextValue) = _smithyReadLengthDelimited(payload, nextKey, operation);
    entries.add(<List<int>>[key, value]);
    offset = nextValue;
  }
  if (offset != payload.length) throw OpenKacheClientException('\$operation map has trailing bytes');
  return entries;
}

List<int> _smithyEncodeUnion(List<int> payload, String operation) =>
    _smithyDecodeUnion(payload, operation);

List<int> _smithyDecodeUnion(List<int> payload, String operation) {
  if (payload.length < 5) throw OpenKacheClientException('\$operation union payload is truncated');
  final (_, end) = _smithyReadLengthDelimited(payload, 1, operation);
  if (end != payload.length) throw OpenKacheClientException('\$operation union payload has trailing bytes');
  return payload;
}
`
}

/** Shared TypeScript helpers for ordered field-sequence framing and scalars. */
