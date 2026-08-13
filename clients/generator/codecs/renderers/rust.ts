/** rust field-sequence and container runtime rendering. */

export function render_rust_field_sequence_helpers(): string {
  return `#[allow(dead_code)]
fn smithy_decode_field_sequence(
    payload: &[u8],
    field_count: usize,
    operation: &str,
) -> std::result::Result<Vec<Option<Vec<u8>>>, Error> {
    let mut offsets = vec![(usize::MAX, usize::MAX); field_count];
    let sequence = openkache_client_core::FieldSequence::decode(
        payload,
        field_count,
        &mut offsets,
    )
    .map_err(|error| Error::Protocol(format!("{operation} field sequence: {error}")))?;
    Ok((0..field_count)
        .map(|index| sequence.get(index).map(ToOwned::to_owned))
        .collect())
}

fn smithy_decode_dense_fields(
    payload: &[u8],
    widths: &[usize],
    operation: &str,
) -> std::result::Result<Vec<Option<Vec<u8>>>, Error> {
    let mut values = Vec::with_capacity(widths.len());
    let mut offset = 0usize;
    for width in widths {
        let end = offset.checked_add(*width).ok_or_else(|| {
            Error::Protocol(format!("{operation} dense field payload is truncated"))
        })?;
        let value = payload.get(offset..end).ok_or_else(|| {
            Error::Protocol(format!("{operation} dense field payload is truncated"))
        })?;
        values.push(Some(value.to_vec()));
        offset = end;
    }
    if offset != payload.len() {
        return Err(Error::Protocol(format!(
            "{operation} dense field payload has trailing bytes"
        )));
    }
    Ok(values)
}
`
}

export function render_rust_container_helpers(
  max_value_bytes: number,
  operations: string,
): string {
  const encode_list = operations.includes("smithy_encode_list(")
  const decode_list = operations.includes("smithy_decode_list(")
  const encode_map = operations.includes("smithy_encode_map(")
  const decode_map = operations.includes("smithy_decode_map(")
  const encode_union = operations.includes("smithy_encode_union(")
  const decode_union = operations.includes("smithy_decode_union(")
  const needs_varuint =
    encode_list || decode_list || encode_map || decode_map || encode_union || decode_union

  const encode_length = needs_varuint
    ? `fn smithy_encode_varuint(value: u64) -> [u8; 9] {
    let mut encoded = [0u8; 9];
    if value < 0x80 {
        encoded[0] = value as u8;
        return encoded;
    }
    if value < 0x4000 {
        encoded[0] = 0x80 | (value & 0x3f) as u8;
        encoded[1] = (value >> 6) as u8;
        return encoded;
    }
    if value < 0x20_0000 {
        encoded[0] = 0xc0 | (value & 0x1f) as u8;
        encoded[1] = (value >> 5) as u8;
        encoded[2] = (value >> 13) as u8;
        return encoded;
    }
    if value < 0x1000_0000 {
        encoded[0] = 0xe0 | (value & 0x0f) as u8;
        encoded[1] = (value >> 4) as u8;
        encoded[2] = (value >> 12) as u8;
        encoded[3] = (value >> 20) as u8;
        return encoded;
    }
    let mut width = 8;
    while width > 1 && value >> ((width - 1) * 8) == 0 {
        width -= 1;
    }
    encoded[0] = 0xf0 | (width - 1) as u8;
    for index in 0..width {
        encoded[index + 1] = (value >> (index * 8)) as u8;
    }
    encoded
}

fn smithy_decode_varuint(
    payload: &[u8],
    operation: &str,
) -> std::result::Result<(u64, usize), Error> {
    let Some(&first) = payload.first() else {
        return Err(Error::Protocol(format!("{operation} container length is truncated")));
    };
    let width = if first < 0x80 {
        1
    } else if first < 0xc0 {
        2
    } else if first < 0xe0 {
        3
    } else if first < 0xf0 {
        4
    } else {
        (first & 0x0f) as usize + 2
    };
    if width > 9 || payload.len() < width {
        return Err(Error::Protocol(format!("{operation} container length is truncated")));
    }
    let value = match width {
        1 => first as u64,
        2 => (first & 0x3f) as u64 | (payload[1] as u64) << 6,
        3 => (first & 0x1f) as u64
            | (payload[1] as u64) << 5
            | (payload[2] as u64) << 13,
        4 => (first & 0x0f) as u64
            | (payload[1] as u64) << 4
            | (payload[2] as u64) << 12
            | (payload[3] as u64) << 20,
        _ => {
            let mut value = 0u64;
            for index in 1..width {
                value |= (payload[index] as u64) << ((index - 1) * 8);
            }
            value
        }
    };
    let canonical = smithy_encode_varuint(value);
    if canonical[0..width] != payload[..width] {
        return Err(Error::Protocol(format!("{operation} container length is non-canonical")));
    }
    Ok((value, width))
}

fn smithy_encode_length_delimited(value: &[u8]) -> std::result::Result<Vec<u8>, Error> {
    if value.len() > ${max_value_bytes} {
        return Err(Error::Protocol("container entry exceeds the maximum value size".into()));
    }
    let length = smithy_encode_varuint(u64::try_from(value.len())
        .map_err(|_| Error::Protocol("container entry is too large".into()))?);
    let length_len = if length[0] < 0x80 { 1 } else if length[0] < 0xc0 {
        2
    } else if length[0] < 0xe0 {
        3
    } else if length[0] < 0xf0 {
        4
    } else {
        (length[0] & 0x0f) as usize + 2
    };
    let mut output = Vec::with_capacity(length_len + value.len());
    output.extend_from_slice(&length[..length_len]);
    output.extend_from_slice(value);
    Ok(output)
}
`
    : ""

  const read_length = decode_list || decode_map || encode_union || decode_union
    ? `fn smithy_read_length_delimited(
    payload: &[u8],
    cursor: &mut usize,
    operation: &str,
) -> std::result::Result<Vec<u8>, Error> {
    let encoded = payload.get(*cursor..).ok_or_else(|| Error::Protocol(
        format!("{operation} container entry length is truncated"),
    ))?;
    let (length, length_len) = smithy_decode_varuint(encoded, operation)?;
    let length = usize::try_from(length)
        .map_err(|_| Error::Protocol(format!("{operation} container entry is malformed")))?;
    if length > ${max_value_bytes} {
        return Err(Error::Protocol(format!("{operation} container entry is malformed")));
    }
    *cursor = (*cursor).checked_add(length_len).ok_or_else(|| {
        Error::Protocol(format!("{operation} container entry length is truncated"))
    })?;
    let value_end = (*cursor).checked_add(length as usize).ok_or_else(|| Error::Protocol(
        format!("{operation} container entry is truncated"),
    ))?;
    let value = payload.get(*cursor..value_end).ok_or_else(|| Error::Protocol(
        format!("{operation} container entry is truncated"),
    ))?.to_vec();
    *cursor = value_end;
    Ok(value)
}
`
    : ""

  const list_encoder = encode_list
    ? `fn smithy_encode_list(values: &[Vec<u8>]) -> std::result::Result<Vec<u8>, Error> {
    let count = smithy_encode_varuint(
        u64::try_from(values.len()).map_err(|_| Error::Protocol("list is too large".into()))?,
    );
    let count_len = if count[0] < 0x80 { 1 } else if count[0] < 0xc0 {
        2
    } else if count[0] < 0xe0 {
        3
    } else if count[0] < 0xf0 {
        4
    } else {
        (count[0] & 0x0f) as usize + 2
    };
    let mut output = Vec::new();
    output.extend_from_slice(&count[..count_len]);
    for value in values {
        output.extend_from_slice(&smithy_encode_length_delimited(value)?);
    }
    Ok(output)
}
`
    : ""

  const list_decoder = decode_list
    ? `fn smithy_decode_list(
    payload: &[u8],
    operation: &str,
) -> std::result::Result<Vec<Vec<u8>>, Error> {
    let (count, count_len) = smithy_decode_varuint(payload, operation)?;
    let mut cursor = count_len;
    let mut values = Vec::with_capacity(usize::try_from(count).map_err(|_| {
        Error::Protocol(format!("{operation} list is too large"))
    })?);
    for _ in 0..count {
        values.push(smithy_read_length_delimited(payload, &mut cursor, operation)?);
    }
    if cursor != payload.len() {
        return Err(Error::Protocol(format!("{operation} list has trailing bytes")));
    }
    Ok(values)
}
`
    : ""

  const map_encoder = encode_map
    ? `fn smithy_encode_map(
    values: &[(Vec<u8>, Vec<u8>)],
) -> std::result::Result<Vec<u8>, Error> {
    let count = smithy_encode_varuint(
        u64::try_from(values.len()).map_err(|_| Error::Protocol("map is too large".into()))?,
    );
    let count_len = if count[0] < 0x80 { 1 } else if count[0] < 0xc0 {
        2
    } else if count[0] < 0xe0 {
        3
    } else if count[0] < 0xf0 {
        4
    } else {
        (count[0] & 0x0f) as usize + 2
    };
    let mut output = Vec::new();
    output.extend_from_slice(&count[..count_len]);
    for (key, value) in values {
        output.extend_from_slice(&smithy_encode_length_delimited(key)?);
        output.extend_from_slice(&smithy_encode_length_delimited(value)?);
    }
    Ok(output)
}
`
    : ""

  const map_decoder = decode_map
    ? `fn smithy_decode_map(
    payload: &[u8],
    operation: &str,
) -> std::result::Result<Vec<(Vec<u8>, Vec<u8>)>, Error> {
    let (count, count_len) = smithy_decode_varuint(payload, operation)?;
    let mut cursor = count_len;
    let mut values = Vec::with_capacity(usize::try_from(count).map_err(|_| {
        Error::Protocol(format!("{operation} map is too large"))
    })?);
    for _ in 0..count {
        values.push((
            smithy_read_length_delimited(payload, &mut cursor, operation)?,
            smithy_read_length_delimited(payload, &mut cursor, operation)?,
        ));
    }
    if cursor != payload.len() {
        return Err(Error::Protocol(format!("{operation} map has trailing bytes")));
    }
    Ok(values)
}
`
    : ""

  const union_decoder = encode_union || decode_union
    ? `fn smithy_decode_union(payload: &[u8], operation: &str) -> std::result::Result<Vec<u8>, Error> {
    if payload.len() < 2 {
        return Err(Error::Protocol(format!("{operation} union payload is truncated")));
    }
    let mut cursor = 1;
    smithy_read_length_delimited(payload, &mut cursor, operation)?;
    if cursor != payload.len() {
        return Err(Error::Protocol(format!("{operation} union payload has trailing bytes")));
    }
    Ok(payload.to_vec())
}
`
    : ""

  const union_encoder = encode_union
    ? `fn smithy_encode_union(payload: &[u8], operation: &str) -> std::result::Result<Vec<u8>, Error> {
    smithy_decode_union(payload, operation)
}
`
    : ""

  return [
    encode_length,
    read_length,
    list_encoder,
    list_decoder,
    map_encoder,
    map_decoder,
    union_decoder,
    union_encoder,
  ].filter((helper) => helper.length > 0).join("\n")
}
