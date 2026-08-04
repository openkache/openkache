//! Inline-value and generation-relative BlobRef encoding.

use crate::*;

pub(crate) const BLOB_ITEM_THRESHOLD_BYTES: usize = 2 * 1024;
pub(crate) const BLOB_REF_BYTES: usize = 8;
pub(crate) const LARGE_VALUE_REF_BYTES: usize = 8;
pub(crate) const STORED_VALUE_TAG_BYTES: usize = 1;
pub(crate) const STORED_BLOB_REF_BYTES: usize = STORED_VALUE_TAG_BYTES + BLOB_REF_BYTES;
pub(crate) const STORED_LARGE_VALUE_REF_BYTES: usize =
    STORED_VALUE_TAG_BYTES + LARGE_VALUE_REF_BYTES;

pub(crate) const INLINE_VALUE_TAG: u8 = 0;
const BLOB_VALUE_TAG: u8 = 1;
const LARGE_VALUE_TAG: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlobRef {
    pub(crate) value_offset: u32,
    pub(crate) value_len: u32,
}

impl BlobRef {
    pub(crate) fn new(value_offset: usize, value_len: usize) -> Result<Self> {
        Ok(Self {
            value_offset: u32::try_from(value_offset)
                .map_err(|_| KvError::Usage("Blob value offset does not fit in u32".into()))?,
            value_len: u32::try_from(value_len)
                .map_err(|_| KvError::Usage("Blob value length does not fit in u32".into()))?,
        })
    }

    fn encode(self) -> [u8; BLOB_REF_BYTES] {
        let mut bytes = [0; BLOB_REF_BYTES];
        bytes[..4].copy_from_slice(&self.value_offset.to_le_bytes());
        bytes[4..].copy_from_slice(&self.value_len.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        (bytes.len() == BLOB_REF_BYTES).then(|| Self {
            value_offset: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            value_len: u32::from_le_bytes(bytes[4..].try_into().unwrap()),
        })
    }
}

pub(crate) enum StoredValue<'a> {
    Inline(&'a [u8]),
    Blob(BlobRef),
    Large(BlobRef),
}

pub(crate) fn encode_inline_value(value: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(STORED_VALUE_TAG_BYTES + value.len());
    encoded.push(INLINE_VALUE_TAG);
    encoded.extend_from_slice(value);
    encoded
}

pub(crate) fn encode_blob_ref(blob_ref: BlobRef) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(STORED_BLOB_REF_BYTES);
    encoded.push(BLOB_VALUE_TAG);
    encoded.extend_from_slice(&blob_ref.encode());
    encoded
}

pub(crate) fn encode_large_value_ref(value_ref: BlobRef) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(STORED_LARGE_VALUE_REF_BYTES);
    encoded.push(LARGE_VALUE_TAG);
    encoded.extend_from_slice(&value_ref.encode());
    encoded
}

pub(crate) fn decode_stored_value(encoded: &[u8]) -> Result<StoredValue<'_>> {
    let Some((&tag, body)) = encoded.split_first() else {
        return Err(KvError::Worker(
            "Segment Item has no stored-value tag".into(),
        ));
    };
    let value = match tag {
        INLINE_VALUE_TAG => StoredValue::Inline(body),
        BLOB_VALUE_TAG => BlobRef::decode(body)
            .map(StoredValue::Blob)
            .ok_or_else(|| KvError::Worker("Segment Item has a malformed BlobRef".into()))?,
        LARGE_VALUE_TAG => BlobRef::decode(body)
            .map(StoredValue::Large)
            .ok_or_else(|| {
                KvError::Worker("Segment Item has a malformed large-value ref".into())
            })?,
        _ => {
            return Err(KvError::Worker(format!(
                "Segment Item has unknown stored-value tag {tag}"
            )));
        }
    };
    Ok(value)
}

pub(crate) fn remove_stored_value_tag(encoded: &mut Vec<u8>) {
    debug_assert!(encoded.len() >= STORED_VALUE_TAG_BYTES);
    encoded.copy_within(STORED_VALUE_TAG_BYTES.., 0);
    encoded.truncate(encoded.len() - STORED_VALUE_TAG_BYTES);
}
