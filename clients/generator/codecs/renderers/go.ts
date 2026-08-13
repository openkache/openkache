/** go field-sequence and container runtime rendering. */

import type { Field_Sequence_Framing } from "../index"

export function render_go_field_sequence_helpers(
  framing: Field_Sequence_Framing,
): string {
  return `func smithyEncodeFieldVarUInt(value uint64) []byte {
	if value < 0x80 {
		return []byte{byte(value)}
	}
	if value < 0x4000 {
		return []byte{byte(0x80 | (value & 0x3f)), byte(value >> 6)}
	}
	if value < 0x200000 {
		return []byte{byte(0xc0 | (value & 0x1f)), byte(value >> 5), byte(value >> 13)}
	}
	if value < 0x10000000 {
		return []byte{byte(0xe0 | (value & 0x0f)), byte(value >> 4), byte(value >> 12), byte(value >> 20)}
	}
	width := 8
	for width > 1 && value>>uint((width-1)*8) == 0 {
		width--
	}
	output := make([]byte, width+1)
	output[0] = byte(0xf0 | (width - 1))
	for index := 0; index < width; index++ {
		output[index+1] = byte(value >> uint(index*8))
	}
	return output
}

func smithyDecodeFieldVarUInt(payload []byte, offset *int, operation string) (uint64, error) {
	if *offset >= len(payload) {
		return 0, validationError(operation, "field length is truncated")
	}
	start := *offset
	first := payload[start]
	width := 1
	switch {
	case first < 0x80:
	case first < 0xc0:
		width = 2
	case first < 0xe0:
		width = 3
	case first < 0xf0:
		width = 4
	default:
		width = int(first&0x0f) + 2
	}
	if width > 9 || start+width > len(payload) {
		return 0, validationError(operation, "field length is truncated")
	}
	var value uint64
	switch width {
	case 1:
		value = uint64(first)
	case 2:
		value = uint64(first&0x3f) | uint64(payload[start+1])<<6
	case 3:
		value = uint64(first&0x1f) | uint64(payload[start+1])<<5 | uint64(payload[start+2])<<13
	case 4:
		value = uint64(first&0x0f) | uint64(payload[start+1])<<4 | uint64(payload[start+2])<<12 | uint64(payload[start+3])<<20
	default:
		for index := 1; index < width; index++ {
			value |= uint64(payload[start+index]) << uint((index-1)*8)
		}
	}
	if len(smithyEncodeFieldVarUInt(value)) != width {
		return 0, validationError(operation, "field length is non-canonical")
	}
	*offset = start + width
	return value, nil
}

func smithyEncodeFieldSequence(values ...[]byte) ([]byte, error) {
	maskBytes := (len(values) + 7) / 8
	lastPresent := -1
	for index := len(values) - 1; index >= 0; index-- {
		if values[index] != nil {
			lastPresent = index
			break
		}
	}
	total := maskBytes
	for index, value := range values {
		if value != nil && len(value) > ${framing.max_value_bytes} {
			return nil, validationError("field_sequence", "entry exceeds the maximum value size")
		}
		if value != nil {
			encodedLengthLength := 0
			if index != lastPresent {
				encodedLengthLength = len(smithyEncodeFieldVarUInt(uint64(len(value))))
			}
			if encodedLengthLength > ${framing.max_value_bytes}-total ||
				len(value) > ${framing.max_value_bytes}-total-encodedLengthLength {
				return nil, validationError("field_sequence", "payload exceeds the maximum value size")
			}
			total += encodedLengthLength + len(value)
		}
	}
	payload := make([]byte, total)
	offset := maskBytes
	for index, value := range values {
		if value != nil {
			payload[index/8] |= 1 << uint(index%8)
			if index != lastPresent {
				encodedLength := smithyEncodeFieldVarUInt(uint64(len(value)))
				copy(payload[offset:], encodedLength)
				offset += len(encodedLength)
			}
			copy(payload[offset:], value)
			offset += len(value)
		}
	}
	return payload, nil
}

func smithyDecodeFieldSequence(payload []byte, fieldCount int, operation string) ([]*[]byte, error) {
	values := make([]*[]byte, fieldCount)
	maskBytes := (fieldCount + 7) / 8
	if len(payload) < maskBytes {
		return nil, validationError(operation, "field sequence is missing its presence mask")
	}
	if maskBytes > 0 && fieldCount%8 != 0 &&
		payload[maskBytes-1]&^byte((1<<uint(fieldCount%8))-1) != 0 {
		return nil, validationError(operation, "field sequence presence mask has unused bits set")
	}
	lastPresent := -1
	for index := fieldCount - 1; index >= 0; index-- {
		if payload[index/8]&(1<<uint(index%8)) != 0 {
			lastPresent = index
			break
		}
	}
	offset := maskBytes
	for index := 0; index < fieldCount; index++ {
		if payload[index/8]&(1<<uint(index%8)) == 0 {
			continue
		}
		var length uint64
		if index == lastPresent {
			length = uint64(len(payload) - offset)
		} else {
			var err error
			length, err = smithyDecodeFieldVarUInt(payload, &offset, operation)
			if err != nil {
				return nil, err
			}
		}
		if length > ${framing.max_value_bytes} || length > uint64(len(payload)-offset) {
			return nil, validationError(operation, "field sequence entry is truncated")
		}
		value := append([]byte(nil), payload[offset:offset+int(length)]...)
		values[index] = &value
		offset += int(length)
	}
	if offset != len(payload) {
		return nil, validationError(operation, "field sequence contains trailing bytes")
	}
	return values, nil
}

func smithyEncodeDenseFields(values ...[]byte) ([]byte, error) {
	total := 0
	for _, value := range values {
		total += len(value)
	}
	if total > ${framing.max_value_bytes} {
		return nil, validationError("dense_fields", "payload exceeds the maximum value size")
	}
	payload := make([]byte, 0, total)
	for _, value := range values {
		payload = append(payload, value...)
	}
	return payload, nil
}

func smithyDecodeDenseFields(payload []byte, widths []int, operation string) ([]*[]byte, error) {
	values := make([]*[]byte, len(widths))
	offset := 0
	for index, width := range widths {
		if width < 0 || width > len(payload)-offset {
			return nil, validationError(operation, "dense field payload is truncated")
		}
		value := append([]byte(nil), payload[offset:offset+width]...)
		values[index] = &value
		offset += width
	}
	if offset != len(payload) {
		return nil, validationError(operation, "dense field payload has trailing bytes")
	}
	return values, nil
}

func smithyEncodeU64(value uint64) []byte {
	payload := make([]byte, 8)
	binary.BigEndian.PutUint64(payload, value)
	return payload
}

func smithyDecodeU64(payload []byte) (uint64, error) {
	if len(payload) != 8 {
		return 0, validationError("u64", "field must contain exactly eight bytes")
	}
	return binary.BigEndian.Uint64(payload), nil
}

func smithyEncodeBool(value bool) []byte {
	if value {
		return []byte{1}
	}
	return []byte{0}
}

func smithyDecodeBool(payload []byte) (bool, error) {
	if len(payload) != 1 || (payload[0] != 0 && payload[0] != 1) {
		return false, validationError("bool", "field must contain exactly one byte")
	}
	return payload[0] == 1, nil
}

func smithyEncodeF64(value float64) []byte {
	payload := make([]byte, 8)
	binary.BigEndian.PutUint64(payload, math.Float64bits(value))
	return payload
}

func smithyDecodeF64(payload []byte) (float64, error) {
	if len(payload) != 8 {
		return 0, validationError("f64", "field must contain exactly eight bytes")
	}
	value := math.Float64frombits(binary.BigEndian.Uint64(payload))
	if math.IsNaN(value) || math.IsInf(value, 0) {
		return 0, validationError("f64", "field must be finite")
	}
	return value, nil
}

func smithyEncodeI32(value int32) []byte {
	payload := make([]byte, 4)
	binary.BigEndian.PutUint32(payload, uint32(value))
	return payload
}

func smithyDecodeI32(payload []byte) (int32, error) {
	if len(payload) != 4 {
		return 0, validationError("i32", "field must contain exactly four bytes")
	}
	return int32(binary.BigEndian.Uint32(payload)), nil
}

func smithyDecodeOptionalU64(value *[]byte) (*uint64, error) {
	if value == nil {
		return nil, nil
	}
	decoded, err := smithyDecodeU64(*value)
	if err != nil {
		return nil, err
	}
	return &decoded, nil
}

func smithyDecodeOptionalBool(value *[]byte) (*bool, error) {
	if value == nil {
		return nil, nil
	}
	decoded, err := smithyDecodeBool(*value)
	if err != nil {
		return nil, err
	}
	return &decoded, nil
}

func smithyDecodeOptionalF64(value *[]byte) (*float64, error) {
	if value == nil {
		return nil, nil
	}
	decoded, err := smithyDecodeF64(*value)
	if err != nil {
		return nil, err
	}
	return &decoded, nil
}

func smithyDecodeOptionalI32(value *[]byte) (*int32, error) {
	if value == nil {
		return nil, nil
	}
	decoded, err := smithyDecodeI32(*value)
	if err != nil {
		return nil, err
	}
	return &decoded, nil
}
`
}

export function render_go_container_helpers(max_value_bytes: number): string {
  return `func smithyEncodeVarUInt(value uint64) []byte {
	if value < 0x80 {
		return []byte{byte(value)}
	}
	if value < 0x4000 {
		return []byte{byte(0x80 | (value & 0x3f)), byte(value >> 6)}
	}
	if value < 0x200000 {
		return []byte{byte(0xc0 | (value & 0x1f)), byte(value >> 5), byte(value >> 13)}
	}
	if value < 0x10000000 {
		return []byte{byte(0xe0 | (value & 0x0f)), byte(value >> 4), byte(value >> 12), byte(value >> 20)}
	}
	width := 8
	for width > 1 && value>>uint((width-1)*8) == 0 {
		width--
	}
	result := make([]byte, width+1)
	result[0] = byte(0xf0 | (width - 1))
	for index := 0; index < width; index++ {
		result[index+1] = byte(value >> uint(index*8))
	}
	return result
}

func smithyDecodeVarUInt(payload []byte, offset *int, operation string) (uint64, error) {
	if *offset >= len(payload) {
		return 0, validationError(operation, "container count is truncated")
	}
	first := payload[*offset]
	width := 1
	switch {
	case first < 0x80:
		width = 1
	case first < 0xc0:
		width = 2
	case first < 0xe0:
		width = 3
	case first < 0xf0:
		width = 4
	default:
		width = int(first&0x0f) + 2
	}
	if width > 9 || *offset+width > len(payload) {
		return 0, validationError(operation, "container count is truncated")
	}
	var value uint64
	switch width {
	case 1:
		value = uint64(first)
	case 2:
		value = uint64(first&0x3f) | uint64(payload[*offset+1])<<6
	case 3:
		value = uint64(first&0x1f) | uint64(payload[*offset+1])<<5 | uint64(payload[*offset+2])<<13
	case 4:
		value = uint64(first&0x0f) | uint64(payload[*offset+1])<<4 | uint64(payload[*offset+2])<<12 | uint64(payload[*offset+3])<<20
	default:
		for index := 1; index < width; index++ {
			value |= uint64(payload[*offset+index]) << uint((index-1)*8)
		}
	}
	if len(smithyEncodeVarUInt(value)) != width {
		return 0, validationError(operation, "container count is non-canonical")
	}
	*offset += width
	return value, nil
}

func smithyEncodeLengthDelimited(value []byte) ([]byte, error) {
	if len(value) > ${max_value_bytes} {
		return nil, validationError("container", "entry exceeds the maximum value size")
	}
	encodedLength := smithyEncodeVarUInt(uint64(len(value)))
	result := make([]byte, len(encodedLength)+len(value))
	copy(result, encodedLength)
	copy(result[len(encodedLength):], value)
	return result, nil
}

func smithyReadLengthDelimited(payload []byte, offset *int, operation string) ([]byte, error) {
	length, err := smithyDecodeVarUInt(payload, offset, operation)
	if err != nil {
		return nil, err
	}
	if length > ${max_value_bytes} || length > uint64(len(payload)-*offset) {
		return nil, validationError(operation, "container entry is malformed")
	}
	start := *offset
	*offset = start + int(length)
	return append([]byte(nil), payload[start:*offset]...), nil
}

func smithyEncodeList(values [][]byte) ([]byte, error) {
	output := append([]byte(nil), smithyEncodeVarUInt(uint64(len(values)))...)
	for _, value := range values {
		encoded, err := smithyEncodeLengthDelimited(value)
		if err != nil {
			return nil, err
		}
		output = append(output, encoded...)
	}
	return output, nil
}

func smithyDecodeList(payload []byte) ([][]byte, error) {
	offset := 0
	count, err := smithyDecodeVarUInt(payload, &offset, "list")
	if err != nil {
		return nil, err
	}
	values := make([][]byte, int(count))
	for index := range values {
		values[index], err = smithyReadLengthDelimited(payload, &offset, "list")
		if err != nil {
			return nil, err
		}
	}
	if offset != len(payload) {
		return nil, validationError("list", "payload has trailing bytes")
	}
	return values, nil
}

func smithyEncodeMap(values [][2][]byte) ([]byte, error) {
	output := append([]byte(nil), smithyEncodeVarUInt(uint64(len(values)))...)
	for _, entry := range values {
		for _, value := range entry {
			encoded, err := smithyEncodeLengthDelimited(value)
			if err != nil {
				return nil, err
			}
			output = append(output, encoded...)
		}
	}
	return output, nil
}

func smithyDecodeMap(payload []byte) ([][2][]byte, error) {
	offset := 0
	count, err := smithyDecodeVarUInt(payload, &offset, "map")
	if err != nil {
		return nil, err
	}
	values := make([][2][]byte, int(count))
	for index := range values {
		values[index][0], err = smithyReadLengthDelimited(payload, &offset, "map")
		if err != nil {
			return nil, err
		}
		values[index][1], err = smithyReadLengthDelimited(payload, &offset, "map")
		if err != nil {
			return nil, err
		}
	}
	if offset != len(payload) {
		return nil, validationError("map", "payload has trailing bytes")
	}
	return values, nil
}

func smithyEncodeEnum(value string, allowed []string) ([]byte, error) {
	for _, candidate := range allowed {
		if value == candidate {
			return []byte(value), nil
		}
	}
	return nil, validationError("enum", "value is not declared by its shape")
}

func smithyDecodeEnum(payload []byte, allowed []string) (string, error) {
	value := string(payload)
	for _, candidate := range allowed {
		if value == candidate {
			return value, nil
		}
	}
	return "", validationError("enum", "value is not declared by its shape")
}

func smithyDecodeOptionalEnum(payload *[]byte, allowed []string) (*string, error) {
	if payload == nil {
		return nil, nil
	}
	value, err := smithyDecodeEnum(*payload, allowed)
	if err != nil {
		return nil, err
	}
	return &value, nil
}

func smithyEncodeUnion(payload []byte) ([]byte, error) {
	return smithyDecodeUnion(payload)
}

func smithyDecodeUnion(payload []byte) ([]byte, error) {
	if len(payload) < 2 {
		return nil, validationError("union", "payload is truncated")
	}
	offset := 1
	if _, err := smithyReadLengthDelimited(payload, &offset, "union"); err != nil {
		return nil, err
	}
	if offset != len(payload) {
		return nil, validationError("union", "payload has trailing bytes")
	}
	return payload, nil
}

func smithyDecodeOptionalUnion(payload *[]byte) (*[]byte, error) {
	if payload == nil {
		return nil, nil
	}
	value, err := smithyDecodeUnion(*payload)
	if err != nil {
		return nil, err
	}
	return &value, nil
}
`
}

/** Shared Rust helpers for ordered field-sequence framing. */
