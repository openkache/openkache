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

  const encode_length = encode_list || encode_map
    ? `fn smithy_encode_length_delimited(value: &[u8]) -> std::result::Result<Vec<u8>, Error> {
    if value.len() > ${max_value_bytes} {
        return Err(Error::Protocol("container entry exceeds the maximum value size".into()));
    }
    let length = u32::try_from(value.len())
        .map_err(|_| Error::Protocol("container entry is too large".into()))?;
    let mut output = Vec::with_capacity(4 + value.len());
    output.extend_from_slice(&length.to_be_bytes());
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
    let end = (*cursor).checked_add(4).ok_or_else(|| Error::Protocol(format!(
        "{operation} container entry length is truncated",
    )))?;
    let length = u32::from_be_bytes(payload.get(*cursor..end).ok_or_else(|| Error::Protocol(
        format!("{operation} container entry length is truncated"),
    ))?.try_into().map_err(|_| Error::Protocol("invalid container length".into()))?);
    if length == u32::MAX || length as usize > ${max_value_bytes} {
        return Err(Error::Protocol(format!("{operation} container entry is malformed")));
    }
    *cursor = end;
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
    let (count, count_len) = openkache_client_core::encode_varuint(
        u64::try_from(values.len()).map_err(|_| Error::Protocol("list is too large".into()))?,
    );
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
    let (count, count_len) = openkache_client_core::decode_varuint(payload, "container count")
        .map_err(|error| Error::Protocol(error.to_string()))?
        .ok_or_else(|| Error::Protocol(format!("{operation} container count is truncated")))?;
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
    let (count, count_len) = openkache_client_core::encode_varuint(
        u64::try_from(values.len()).map_err(|_| Error::Protocol("map is too large".into()))?,
    );
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
    let (count, count_len) = openkache_client_core::decode_varuint(payload, "container count")
        .map_err(|error| Error::Protocol(error.to_string()))?
        .ok_or_else(|| Error::Protocol(format!("{operation} container count is truncated")))?;
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
    if payload.len() < 5 {
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
