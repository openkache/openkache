//! Versioned on-disk format boundaries for the Segment generation ring.
//!
//! The header is the first aligned page in every `.data` file.  It is written
//! before the first generation and validated before any committed bytes are
//! opened.  A mismatched or corrupt header is a startup error; callers never
//! silently reinterpret an older layout.

use crate::{BUCKET_BYTES, Config, DirectIoBuffer, KvError, Result};

pub(crate) const SEGMENT_FILE_HEADER_BYTES: u64 = BUCKET_BYTES as u64;
const FILE_MAGIC: &[u8; 8] = b"OKSGV1\0\0";
const FORMAT_VERSION: u32 = 1;

pub(crate) fn encode_segment_file_header(
    config: &Config,
    storage_key_id: [u8; 16],
) -> DirectIoBuffer {
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(FILE_MAGIC);
    bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    bytes[28..36].copy_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes[36..40].copy_from_slice(&(config.segment_count as u32).to_le_bytes());
    bytes[40..44].copy_from_slice(&(BUCKET_BYTES as u32).to_le_bytes());
    bytes[44..52].copy_from_slice(&(config.blob_segment_size as u64).to_le_bytes());
    bytes[52..60].copy_from_slice(&(config.large_value_capacity as u64).to_le_bytes());
    bytes[60..64].copy_from_slice(&(config.bucket_choice_count as u32).to_le_bytes());
    let checksum = crc32fast::hash(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

pub(crate) fn validate_segment_file_header(
    bytes: &[u8],
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<()> {
    if bytes.len() != BUCKET_BYTES {
        return Err(KvError::Worker(
            "Segment file header has an invalid length".into(),
        ));
    }
    let checksum = u32::from_le_bytes(
        bytes[BUCKET_BYTES - 4..]
            .try_into()
            .expect("the fixed header checksum is four bytes"),
    );
    if &bytes[..8] != FILE_MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != FORMAT_VERSION
        || crc32fast::hash(&bytes[..BUCKET_BYTES - 4]) != checksum
    {
        return Err(KvError::Worker(
            "Segment file header is invalid or uses an unsupported format".into(),
        ));
    }
    if bytes[12..28] != storage_key_id {
        return Err(KvError::Worker(
            "Segment file was created with a different server key".into(),
        ));
    }
    let segment_size = u64::from_le_bytes(bytes[28..36].try_into().unwrap());
    let segment_count = u32::from_le_bytes(bytes[36..40].try_into().unwrap()) as usize;
    let bucket_bytes = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    let blob_segment_size = u64::from_le_bytes(bytes[44..52].try_into().unwrap());
    let large_value_capacity = u64::from_le_bytes(bytes[52..60].try_into().unwrap());
    let bucket_choice_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    if segment_size != config.segment_size as u64
        || segment_count != config.segment_count
        || bucket_bytes != BUCKET_BYTES
        || blob_segment_size != config.blob_segment_size as u64
        || large_value_capacity != config.large_value_capacity as u64
        || bucket_choice_count != config.bucket_choice_count
    {
        return Err(KvError::Worker(
            "Segment file does not match the configured Segment geometry".into(),
        ));
    }
    Ok(())
}
