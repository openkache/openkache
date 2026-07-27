//! Embedded SG-file metadata used to rebuild process-local indexes after restart.

use std::time::Duration;

use compio::BufResult;
use compio::fs::File;
use compio::io::{AsyncReadAt, AsyncWriteAt};

use crate::*;

const FILE_MAGIC: &[u8; 8] = b"OKSGFILE";
const CONTROL_MAGIC: &[u8; 8] = b"OKSGCTL\0";
const FORMAT_VERSION: u32 = 1;
const REGULAR_OCCUPIED: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SegmentCommit {
    pub(crate) sg_index: usize,
    pub(crate) generation: u64,
    pub(crate) regular_occupied: bool,
    pub(crate) blob_logical_len: usize,
}

pub(crate) struct RecoveryState {
    pub(crate) commits: Vec<SegmentCommit>,
    pub(crate) next_segment_index: usize,
    pub(crate) next_generation: u64,
}

pub(crate) async fn initialize_segment_file(
    file: &mut File,
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<()> {
    file.set_len(config.segment_file_bytes()?).await?;
    write_page(
        file,
        encode_file_header(config, storage_key_id),
        0,
        config.write_max_time_us,
        "Segment file header write",
    )
    .await?;
    file.sync_data().await?;
    Ok(())
}

pub(crate) async fn recover_state(
    file: &File,
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<RecoveryState> {
    let header = read_page(file, 0, config.read_max_time_us, "Segment file header read").await?;
    validate_file_header(&header, config, storage_key_id)?;

    let mut commits = Vec::new();
    for sg_index in 0..config.segment_count {
        let bytes = read_page(
            file,
            config.segment_control_offset(sg_index),
            config.read_max_time_us,
            "SG control page read",
        )
        .await?;
        if bytes.iter().all(|byte| *byte == 0) {
            continue;
        }
        commits.push(decode_control_page(
            &bytes,
            config,
            storage_key_id,
            sg_index,
        )?);
    }
    commits.sort_unstable_by_key(|commit| commit.generation);
    if commits
        .windows(2)
        .any(|pair| pair[0].generation == pair[1].generation)
    {
        return Err(KvError::Worker(
            "SG control pages contain duplicate generations".into(),
        ));
    }
    let newest = commits.last().copied();
    let next_generation = match newest {
        Some(commit) => commit
            .generation
            .checked_add(1)
            .ok_or_else(|| KvError::Worker("SG generation is exhausted".into()))?,
        None => 1,
    };
    Ok(RecoveryState {
        commits,
        next_segment_index: newest.map_or(0, |commit| (commit.sg_index + 1) % config.segment_count),
        next_generation,
    })
}

pub(crate) async fn commit_segment(
    file: &mut File,
    config: &Config,
    storage_key_id: [u8; 16],
    commit: SegmentCommit,
) -> Result<u64> {
    write_page(
        file,
        encode_control_page(config, storage_key_id, commit)?,
        config.segment_control_offset(commit.sg_index),
        config.write_max_time_us,
        "SG control page write",
    )
    .await?;
    file.sync_data().await?;
    Ok(BUCKET_BYTES as u64)
}

pub(crate) async fn invalidate_segment(
    file: &mut File,
    config: &Config,
    sg_index: usize,
) -> Result<u64> {
    write_page(
        file,
        DirectIoBuffer::zeroed(BUCKET_BYTES),
        config.segment_control_offset(sg_index),
        config.write_max_time_us,
        "SG control page invalidation",
    )
    .await?;
    file.sync_data().await?;
    Ok(BUCKET_BYTES as u64)
}

fn encode_file_header(config: &Config, storage_key_id: [u8; 16]) -> DirectIoBuffer {
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(FILE_MAGIC);
    bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    bytes[28..36].copy_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes[36..40].copy_from_slice(&(config.segment_count as u32).to_le_bytes());
    bytes[40..44].copy_from_slice(&(BUCKET_BYTES as u32).to_le_bytes());
    let checksum = checksum(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn validate_file_header(bytes: &[u8], config: &Config, storage_key_id: [u8; 16]) -> Result<()> {
    if &bytes[..8] != FILE_MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != FORMAT_VERSION
        || u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize != BUCKET_BYTES
        || checksum(&bytes[..BUCKET_BYTES - 4])
            != u32::from_le_bytes(bytes[BUCKET_BYTES - 4..].try_into().unwrap())
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
    if segment_size != config.segment_size as u64 || segment_count != config.segment_count {
        return Err(KvError::Worker(
            "Segment file does not match the configured Segment geometry".into(),
        ));
    }
    Ok(())
}

fn encode_control_page(
    config: &Config,
    storage_key_id: [u8; 16],
    commit: SegmentCommit,
) -> Result<DirectIoBuffer> {
    let sg_index = u32::try_from(commit.sg_index)
        .map_err(|_| KvError::Worker("SG index does not fit the control page".into()))?;
    let blob_logical_len = u64::try_from(commit.blob_logical_len)
        .map_err(|_| KvError::Worker("Blob length does not fit the control page".into()))?;
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(CONTROL_MAGIC);
    bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    bytes[28..32].copy_from_slice(&sg_index.to_le_bytes());
    bytes[32..40].copy_from_slice(&commit.generation.to_le_bytes());
    bytes[40..48].copy_from_slice(&blob_logical_len.to_le_bytes());
    let flags = if commit.regular_occupied {
        REGULAR_OCCUPIED
    } else {
        0
    };
    bytes[48..52].copy_from_slice(&flags.to_le_bytes());
    bytes[52..60].copy_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes[60..64].copy_from_slice(&(config.segment_count as u32).to_le_bytes());
    let checksum = checksum(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

fn decode_control_page(
    bytes: &[u8],
    config: &Config,
    storage_key_id: [u8; 16],
    sg_index: usize,
) -> Result<SegmentCommit> {
    let stored_sg_index = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
    let generation = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let blob_logical_len =
        usize::try_from(u64::from_le_bytes(bytes[40..48].try_into().unwrap()))
            .map_err(|_| KvError::Worker("stored Blob length is too large".into()))?;
    let flags = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
    let segment_size = u64::from_le_bytes(bytes[52..60].try_into().unwrap());
    let segment_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    if &bytes[..8] != CONTROL_MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().unwrap()) != FORMAT_VERSION
        || bytes[12..28] != storage_key_id
        || stored_sg_index != sg_index
        || generation == 0
        || flags & !REGULAR_OCCUPIED != 0
        || blob_logical_len > config.segment_size
        || segment_size != config.segment_size as u64
        || segment_count != config.segment_count
        || checksum(&bytes[..BUCKET_BYTES - 4])
            != u32::from_le_bytes(bytes[BUCKET_BYTES - 4..].try_into().unwrap())
    {
        return Err(KvError::Worker(format!(
            "SG control page {sg_index} is invalid"
        )));
    }
    Ok(SegmentCommit {
        sg_index,
        generation,
        regular_occupied: flags & REGULAR_OCCUPIED != 0,
        blob_logical_len,
    })
}

async fn read_page(
    file: &File,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    let read = file.read_at(DirectIoBuffer::for_read(BUCKET_BYTES), offset);
    let BufResult(result, bytes) =
        compio::runtime::time::timeout(Duration::from_micros(timeout_us), read)
            .await
            .map_err(|_| KvError::Timeout(operation))?;
    require_complete_direct_io(operation, result?, BUCKET_BYTES)?;
    Ok(bytes)
}

async fn write_page(
    file: &mut File,
    bytes: DirectIoBuffer,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<()> {
    let write = file.write_at(bytes, offset);
    let BufResult(result, bytes) =
        compio::runtime::time::timeout(Duration::from_micros(timeout_us), write)
            .await
            .map_err(|_| KvError::Timeout(operation))?;
    require_complete_direct_io(operation, result?, BUCKET_BYTES)?;
    debug_assert_eq!(bytes.len(), BUCKET_BYTES);
    Ok(())
}

fn checksum(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
