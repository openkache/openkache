//! Embedded SG-file metadata used to rebuild process-local indexes after restart.

use std::io::ErrorKind;
use std::path::Path;

use compio::fs::{File, OpenOptions};

use super::{read_exact_direct, write_all_direct};
use crate::BUCKET_BYTES;
use crate::buffer::DirectIoBuffer;
use crate::config::Config;
use crate::error::{KvError, Result};
use crate::table::Table;

const FILE_MAGIC: &[u8; 8] = b"OKSGFILE";
const CONTROL_MAGIC: &[u8; 8] = b"OKSGCTL\0";
pub(crate) const LEGACY_FORMAT_VERSION: u32 = 2;
const FORMAT_VERSION: u32 = 3;
const CHECKPOINT_MAGIC: &[u8; 8] = b"OKTABLE\0";
const CHECKPOINT_FOOTER_MAGIC: &[u8; 8] = b"OKTBLEND";
const CHECKPOINT_VERSION: u32 = 2;
const CHECKPOINT_COMMIT_BYTES: usize = 24;
const CHECKPOINT_IO_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SegmentCommit {
    pub(crate) sg_index: usize,
    pub(crate) generation: u64,
    pub(crate) blob_logical_len: usize,
    pub(crate) bucket_choice_count: usize,
}

pub(crate) struct RecoveryState {
    pub(crate) commits: Vec<SegmentCommit>,
    pub(crate) next_segment_index: usize,
    pub(crate) next_generation: u64,
}

pub(crate) struct TableCheckpoint {
    pub(crate) table: Table,
    pub(crate) recovery_state: RecoveryState,
    pub(crate) stable_live_keys: usize,
}

#[derive(Clone, Copy)]
struct CheckpointHeader {
    entry_count: usize,
    stable_live_keys: usize,
    next_segment_index: usize,
    next_generation: u64,
    commit_bytes: usize,
    table_bytes: usize,
    payload_bytes: usize,
    file_bytes: u64,
}

pub(crate) fn next_sg_generation(generation: u64) -> Result<u64> {
    generation
        .checked_add(1)
        .ok_or_else(|| KvError::Worker("SG generation is exhausted".into()))
}

pub(crate) async fn initialize_segment_file(
    file: &mut File,
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<()> {
    file.set_len(config.segment_file_bytes()?).await?;
    let _ = write_page(
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
    let mut bytes = read_page(file, 0, config.read_max_time_us, "Segment file header read").await?;
    validate_file_header(&bytes, config, storage_key_id)?;

    let mut commits = Vec::new();
    for sg_index in 0..config.segment_count {
        bytes = read_page_into(
            file,
            bytes,
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
    recovery_state_from_commits(commits, config.segment_count)
}

fn recovery_state_from_commits(
    mut commits: Vec<SegmentCommit>,
    segment_count: usize,
) -> Result<RecoveryState> {
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
        Some(commit) => next_sg_generation(commit.generation)?,
        None => 1,
    };
    Ok(RecoveryState {
        commits,
        next_segment_index: newest.map_or(0, |commit| (commit.sg_index + 1) % segment_count),
        next_generation,
    })
}

pub(crate) async fn validate_segment_file(
    file: &File,
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<()> {
    let header = read_page(file, 0, config.read_max_time_us, "Segment file header read").await?;
    validate_file_header(&header, config, storage_key_id)
}

pub(crate) async fn write_table_checkpoint(
    config: &Config,
    storage_key_id: [u8; 16],
    table: &Table,
    stable_live_keys: usize,
    segment_commits: &[Option<SegmentCommit>],
    next_segment_index: usize,
    next_generation: u64,
) -> Result<()> {
    if segment_commits.len() != config.segment_count
        || stable_live_keys > table.entry_count
        || table.restored_entry_count() != table.entry_count
    {
        return Err(KvError::Worker(
            "Table checkpoint state is internally inconsistent".into(),
        ));
    }

    let commits = segment_commits.iter().flatten().copied().collect();
    let recovery_state = recovery_state_from_commits(commits, config.segment_count)?;
    if recovery_state.next_segment_index != next_segment_index
        || recovery_state.next_generation != next_generation
    {
        return Err(KvError::Worker(
            "Table checkpoint generation does not match committed Segments".into(),
        ));
    }

    let commit_bytes = encode_checkpoint_commits(segment_commits)?;
    let table_bytes = table.memory_bytes();
    let logical_payload_bytes = commit_bytes
        .len()
        .checked_add(table_bytes)
        .ok_or_else(|| KvError::Worker("Table checkpoint size overflowed".into()))?;
    let payload_bytes = logical_payload_bytes
        .checked_next_multiple_of(BUCKET_BYTES)
        .ok_or_else(|| KvError::Worker("Table checkpoint size overflowed".into()))?;
    let file_bytes = BUCKET_BYTES
        .checked_add(payload_bytes)
        .and_then(|bytes| bytes.checked_add(BUCKET_BYTES))
        .ok_or_else(|| KvError::Worker("Table checkpoint size overflowed".into()))?;

    let checkpoint_header = CheckpointHeader {
        entry_count: table.entry_count,
        stable_live_keys,
        next_segment_index,
        next_generation,
        commit_bytes: commit_bytes.len(),
        table_bytes,
        payload_bytes,
        file_bytes: file_bytes as u64,
    };
    let header = encode_checkpoint_header(config, storage_key_id, table, checkpoint_header)?;
    let next_path = config.next_checkpoint_path();
    let checkpoint_path = config.checkpoint_path();
    let mut file = open_checkpoint_file(&next_path, true).await?;
    file.set_len(file_bytes as u64).await?;

    let mut checkpoint_checksum = crc32fast::Hasher::new();
    checkpoint_checksum.update(&header);
    write_checkpoint_extent(&mut file, header, 0, config.write_max_time_us).await?;

    let table_slices = table.checkpoint_bytes();
    let mut payload_offset = 0usize;
    let mut payload_buffer = DirectIoBuffer::zeroed(CHECKPOINT_IO_BYTES.min(payload_bytes));
    while payload_offset < payload_bytes {
        let extent_bytes = CHECKPOINT_IO_BYTES.min(payload_bytes - payload_offset);
        copy_checkpoint_payload(
            &commit_bytes,
            table_slices,
            payload_offset,
            &mut payload_buffer[..extent_bytes],
            logical_payload_bytes,
        );
        checkpoint_checksum.update(&payload_buffer[..extent_bytes]);
        payload_buffer = write_all_direct(
            &file,
            payload_buffer,
            (BUCKET_BYTES + payload_offset) as u64,
            extent_bytes,
            config.write_max_time_us,
            "Table checkpoint write",
        )
        .await?;
        payload_offset += extent_bytes;
    }

    let footer = encode_checkpoint_footer(file_bytes as u64, checkpoint_checksum.finalize());
    write_checkpoint_extent(
        &mut file,
        footer,
        (BUCKET_BYTES + payload_bytes) as u64,
        config.write_max_time_us,
    )
    .await?;
    file.sync_data().await?;
    drop(file);
    replace_checkpoint(next_path, checkpoint_path).await?;
    Ok(())
}

pub(crate) async fn load_table_checkpoint(
    config: &Config,
    storage_key_id: [u8; 16],
) -> Result<Option<TableCheckpoint>> {
    let path = config.checkpoint_path();
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() < (2 * BUCKET_BYTES) as u64
        || !metadata.len().is_multiple_of(BUCKET_BYTES as u64)
    {
        return Ok(None);
    }

    let file = open_checkpoint_file(&path, false).await?;
    let header_bytes = read_checkpoint_extent(
        &file,
        BUCKET_BYTES,
        0,
        config.read_max_time_us,
        "Table checkpoint header read",
    )
    .await?;
    let mut table = Table::new(config)?;
    let Some(header) = decode_checkpoint_header(
        &header_bytes,
        config,
        storage_key_id,
        &table,
        metadata.len(),
    ) else {
        return Ok(None);
    };

    let mut commit_bytes = vec![0; header.commit_bytes];
    let logical_payload_bytes = header
        .commit_bytes
        .checked_add(header.table_bytes)
        .ok_or_else(|| KvError::Worker("Table checkpoint size overflowed".into()))?;
    let mut checkpoint_checksum = crc32fast::Hasher::new();
    checkpoint_checksum.update(&header_bytes);
    let mut payload_offset = 0usize;
    let mut payload_buffer =
        DirectIoBuffer::for_read(CHECKPOINT_IO_BYTES.min(header.payload_bytes));
    while payload_offset < header.payload_bytes {
        let extent_bytes = CHECKPOINT_IO_BYTES.min(header.payload_bytes - payload_offset);
        payload_buffer = read_exact_direct(
            &file,
            payload_buffer,
            (BUCKET_BYTES + payload_offset) as u64,
            extent_bytes,
            config.read_max_time_us,
            "Table checkpoint payload read",
        )
        .await?;
        checkpoint_checksum.update(&payload_buffer[..extent_bytes]);
        restore_checkpoint_payload(
            &mut commit_bytes,
            &mut table,
            payload_offset,
            &payload_buffer[..extent_bytes],
            logical_payload_bytes,
        );
        payload_offset += extent_bytes;
    }

    let footer = read_checkpoint_extent(
        &file,
        BUCKET_BYTES,
        (BUCKET_BYTES + header.payload_bytes) as u64,
        config.read_max_time_us,
        "Table checkpoint footer read",
    )
    .await?;
    if !validate_checkpoint_footer(&footer, header.file_bytes, checkpoint_checksum.finalize())
        || table.restored_entry_count() != header.entry_count
    {
        return Ok(None);
    }
    table.entry_count = header.entry_count;

    let commits = decode_checkpoint_commits(&commit_bytes, config)?;
    let recovery_state = recovery_state_from_commits(commits, config.segment_count)?;
    if recovery_state.next_segment_index != header.next_segment_index
        || recovery_state.next_generation != header.next_generation
    {
        return Ok(None);
    }
    Ok(Some(TableCheckpoint {
        table,
        recovery_state,
        stable_live_keys: header.stable_live_keys,
    }))
}

fn encode_checkpoint_header(
    config: &Config,
    storage_key_id: [u8; 16],
    table: &Table,
    header: CheckpointHeader,
) -> Result<DirectIoBuffer> {
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(CHECKPOINT_MAGIC);
    bytes[8..12].copy_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    put_u64(&mut bytes, 28, BUCKET_BYTES)?;
    put_u64(&mut bytes, 36, config.segment_size)?;
    put_u64(&mut bytes, 44, config.segment_count)?;
    put_u64(&mut bytes, 52, config.blob_segment_size)?;
    put_u64(&mut bytes, 60, config.table_capacity)?;
    put_u64(&mut bytes, 68, config.table_target_load_percent)?;
    put_u64(&mut bytes, 76, config.fingerprint_bits)?;
    put_u64(&mut bytes, 84, config.unary_count)?;
    put_u64(&mut bytes, 92, config.front_back_ratio)?;
    put_u64(&mut bytes, 100, config.bucket_choice_count)?;
    put_u64(&mut bytes, 108, config.sg_index_bits)?;
    put_u64(&mut bytes, 116, config.fingerprint_hash_offset_bits)?;
    put_u64(&mut bytes, 124, table.front_table.len())?;
    put_u64(&mut bytes, 132, table.back_table.len())?;
    put_u64(&mut bytes, 140, header.entry_count)?;
    put_u64(&mut bytes, 148, header.stable_live_keys)?;
    put_u64(&mut bytes, 156, header.next_segment_index)?;
    bytes[164..172].copy_from_slice(&header.next_generation.to_le_bytes());
    put_u64(&mut bytes, 172, header.commit_bytes)?;
    put_u64(&mut bytes, 180, header.table_bytes)?;
    put_u64(&mut bytes, 188, header.payload_bytes)?;
    bytes[196..204].copy_from_slice(&header.file_bytes.to_le_bytes());
    let header_checksum = checksum(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&header_checksum.to_le_bytes());
    Ok(bytes)
}

fn decode_checkpoint_header(
    bytes: &[u8],
    config: &Config,
    storage_key_id: [u8; 16],
    table: &Table,
    actual_file_bytes: u64,
) -> Option<CheckpointHeader> {
    let expected_checksum = u32::from_le_bytes(bytes[BUCKET_BYTES - 4..].try_into().ok()?);
    if &bytes[..8] != CHECKPOINT_MAGIC
        || u32::from_le_bytes(bytes[8..12].try_into().ok()?) != CHECKPOINT_VERSION
        || bytes[12..28] != storage_key_id
        || checksum(&bytes[..BUCKET_BYTES - 4]) != expected_checksum
        || get_usize(bytes, 28)? != BUCKET_BYTES
        || get_usize(bytes, 36)? != config.segment_size
        || get_usize(bytes, 44)? != config.segment_count
        || get_usize(bytes, 52)? != config.blob_segment_size
        || get_usize(bytes, 60)? != config.table_capacity
        || get_usize(bytes, 68)? != config.table_target_load_percent
        || get_usize(bytes, 76)? != config.fingerprint_bits
        || get_usize(bytes, 84)? != config.unary_count
        || get_usize(bytes, 92)? != config.front_back_ratio
        || get_usize(bytes, 100)? != config.bucket_choice_count
        || get_usize(bytes, 108)? != config.sg_index_bits
        || get_usize(bytes, 116)? != config.fingerprint_hash_offset_bits
        || get_usize(bytes, 124)? != table.front_table.len()
        || get_usize(bytes, 132)? != table.back_table.len()
    {
        return None;
    }

    let entry_count = get_usize(bytes, 140)?;
    let stable_live_keys = get_usize(bytes, 148)?;
    let next_segment_index = get_usize(bytes, 156)?;
    let next_generation = u64::from_le_bytes(bytes[164..172].try_into().ok()?);
    let commit_bytes = get_usize(bytes, 172)?;
    let table_bytes = get_usize(bytes, 180)?;
    let payload_bytes = get_usize(bytes, 188)?;
    let file_bytes = u64::from_le_bytes(bytes[196..204].try_into().ok()?);
    if stable_live_keys > entry_count
        || next_segment_index >= config.segment_count
        || next_generation == 0
        || commit_bytes != config.segment_count.checked_mul(CHECKPOINT_COMMIT_BYTES)?
        || table_bytes != table.memory_bytes()
        || payload_bytes
            != commit_bytes
                .checked_add(table_bytes)?
                .checked_next_multiple_of(BUCKET_BYTES)?
        || file_bytes
            != (BUCKET_BYTES as u64)
                .checked_add(payload_bytes as u64)?
                .checked_add(BUCKET_BYTES as u64)?
        || file_bytes != actual_file_bytes
    {
        return None;
    }
    Some(CheckpointHeader {
        entry_count,
        stable_live_keys,
        next_segment_index,
        next_generation,
        commit_bytes,
        table_bytes,
        payload_bytes,
        file_bytes,
    })
}

fn encode_checkpoint_footer(file_bytes: u64, checkpoint_checksum: u32) -> DirectIoBuffer {
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(CHECKPOINT_FOOTER_MAGIC);
    bytes[8..12].copy_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
    bytes[12..16].copy_from_slice(&checkpoint_checksum.to_le_bytes());
    bytes[16..24].copy_from_slice(&file_bytes.to_le_bytes());
    let footer_checksum = checksum(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&footer_checksum.to_le_bytes());
    bytes
}

fn validate_checkpoint_footer(bytes: &[u8], file_bytes: u64, checkpoint_checksum: u32) -> bool {
    &bytes[..8] == CHECKPOINT_FOOTER_MAGIC
        && u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == CHECKPOINT_VERSION
        && u32::from_le_bytes(bytes[12..16].try_into().unwrap()) == checkpoint_checksum
        && u64::from_le_bytes(bytes[16..24].try_into().unwrap()) == file_bytes
        && checksum(&bytes[..BUCKET_BYTES - 4])
            == u32::from_le_bytes(bytes[BUCKET_BYTES - 4..].try_into().unwrap())
}

fn encode_checkpoint_commits(commits: &[Option<SegmentCommit>]) -> Result<Vec<u8>> {
    let mut bytes = vec![
        0;
        commits
            .len()
            .checked_mul(CHECKPOINT_COMMIT_BYTES)
            .ok_or_else(|| KvError::Worker(
                "Table checkpoint commit metadata overflowed".into()
            ))?
    ];
    for (sg_index, commit) in commits.iter().enumerate() {
        let Some(commit) = commit else {
            continue;
        };
        if commit.sg_index != sg_index
            || commit.generation == 0
            || !(1..=32).contains(&commit.bucket_choice_count)
            || !commit.bucket_choice_count.is_power_of_two()
        {
            return Err(KvError::Worker(
                "Table checkpoint contains an invalid Segment commit".into(),
            ));
        }
        let offset = sg_index * CHECKPOINT_COMMIT_BYTES;
        bytes[offset..offset + 8].copy_from_slice(&commit.generation.to_le_bytes());
        bytes[offset + 8..offset + 16].copy_from_slice(
            &u64::try_from(commit.blob_logical_len)
                .map_err(|_| KvError::Worker("Blob length does not fit checkpoint".into()))?
                .to_le_bytes(),
        );
        bytes[offset + 16..offset + 24].copy_from_slice(
            &u64::try_from(commit.bucket_choice_count)
                .map_err(|_| KvError::Worker("Bucket choice count does not fit checkpoint".into()))?
                .to_le_bytes(),
        );
    }
    Ok(bytes)
}

fn decode_checkpoint_commits(bytes: &[u8], config: &Config) -> Result<Vec<SegmentCommit>> {
    let (encoded_commits, remainder) = bytes.as_chunks::<CHECKPOINT_COMMIT_BYTES>();
    if !remainder.is_empty() {
        return Err(KvError::Worker(
            "Table checkpoint commit metadata is misaligned".into(),
        ));
    }
    let mut commits = Vec::new();
    for (sg_index, encoded) in encoded_commits.iter().enumerate() {
        let generation = u64::from_le_bytes(encoded[..8].try_into().unwrap());
        let blob_logical_len =
            usize::try_from(u64::from_le_bytes(encoded[8..16].try_into().unwrap()))
                .map_err(|_| KvError::Worker("checkpoint Blob length is too large".into()))?;
        let bucket_choice_count = usize::try_from(u64::from_le_bytes(
            encoded[16..24].try_into().unwrap(),
        ))
        .map_err(|_| KvError::Worker("checkpoint Bucket choice count is too large".into()))?;
        if generation == 0 {
            if blob_logical_len != 0 || bucket_choice_count != 0 {
                return Err(KvError::Worker(
                    "Table checkpoint contains invalid empty Segment metadata".into(),
                ));
            }
            continue;
        }
        if blob_logical_len > config.blob_segment_size {
            return Err(KvError::Worker(
                "Table checkpoint Blob length exceeds its Segment".into(),
            ));
        }
        validate_stored_bucket_choice_count(
            bucket_choice_count,
            config,
            "Table checkpoint Segment",
        )?;
        commits.push(SegmentCommit {
            sg_index,
            generation,
            blob_logical_len,
            bucket_choice_count,
        });
    }
    Ok(commits)
}

fn copy_checkpoint_payload(
    commit_bytes: &[u8],
    table_slices: [&[u8]; 2],
    payload_offset: usize,
    destination: &mut [u8],
    logical_payload_bytes: usize,
) {
    let logical_end = (payload_offset + destination.len()).min(logical_payload_bytes);
    if payload_offset >= logical_end {
        return;
    }
    let commit_end = logical_end.min(commit_bytes.len());
    if payload_offset < commit_end {
        destination[..commit_end - payload_offset]
            .copy_from_slice(&commit_bytes[payload_offset..commit_end]);
    }
    let table_start = payload_offset.max(commit_bytes.len());
    if table_start < logical_end {
        copy_segmented_bytes(
            table_slices,
            table_start - commit_bytes.len(),
            &mut destination[table_start - payload_offset..logical_end - payload_offset],
        );
    }
}

fn restore_checkpoint_payload(
    commit_bytes: &mut [u8],
    table: &mut Table,
    payload_offset: usize,
    source: &[u8],
    logical_payload_bytes: usize,
) {
    let logical_end = (payload_offset + source.len()).min(logical_payload_bytes);
    if payload_offset >= logical_end {
        return;
    }
    let commit_end = logical_end.min(commit_bytes.len());
    if payload_offset < commit_end {
        commit_bytes[payload_offset..commit_end]
            .copy_from_slice(&source[..commit_end - payload_offset]);
    }
    let table_start = payload_offset.max(commit_bytes.len());
    if table_start < logical_end {
        copy_into_segmented_bytes(
            table.checkpoint_bytes_mut(),
            table_start - commit_bytes.len(),
            &source[table_start - payload_offset..logical_end - payload_offset],
        );
    }
}

fn copy_segmented_bytes(slices: [&[u8]; 2], mut offset: usize, mut destination: &mut [u8]) {
    for bytes in slices {
        if offset >= bytes.len() {
            offset -= bytes.len();
            continue;
        }
        let copied = destination.len().min(bytes.len() - offset);
        destination[..copied].copy_from_slice(&bytes[offset..offset + copied]);
        destination = &mut destination[copied..];
        offset = 0;
        if <[u8]>::is_empty(destination) {
            return;
        }
    }
}

fn copy_into_segmented_bytes(slices: [&mut [u8]; 2], mut offset: usize, mut source: &[u8]) {
    for bytes in slices {
        if offset >= bytes.len() {
            offset -= bytes.len();
            continue;
        }
        let copied = source.len().min(bytes.len() - offset);
        bytes[offset..offset + copied].copy_from_slice(&source[..copied]);
        source = &source[copied..];
        offset = 0;
        if <[u8]>::is_empty(source) {
            return;
        }
    }
}

fn put_u64(bytes: &mut [u8], offset: usize, value: usize) -> Result<()> {
    bytes[offset..offset + 8].copy_from_slice(
        &u64::try_from(value)
            .map_err(|_| KvError::Worker("checkpoint value does not fit u64".into()))?
            .to_le_bytes(),
    );
    Ok(())
}

fn get_usize(bytes: &[u8], offset: usize) -> Option<usize> {
    usize::try_from(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
    .ok()
}

async fn open_checkpoint_file(path: &Path, create: bool) -> std::io::Result<File> {
    OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(create)
        .custom_flags(libc::O_DIRECT | libc::O_NOFOLLOW)
        .open(path)
        .await
}

async fn read_checkpoint_extent(
    file: &File,
    len: usize,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    read_exact_direct(
        file,
        DirectIoBuffer::for_read(len),
        offset,
        len,
        timeout_us,
        operation,
    )
    .await
}

async fn write_checkpoint_extent(
    file: &mut File,
    bytes: DirectIoBuffer,
    offset: u64,
    timeout_us: u64,
) -> Result<()> {
    let expected = bytes.len();
    write_all_direct(
        file,
        bytes,
        offset,
        expected,
        timeout_us,
        "Table checkpoint write",
    )
    .await
    .map(drop)
}

async fn replace_checkpoint(
    next_path: std::path::PathBuf,
    checkpoint_path: std::path::PathBuf,
) -> Result<()> {
    compio::runtime::spawn_blocking(move || {
        std::fs::rename(&next_path, &checkpoint_path)?;
        let parent = checkpoint_path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::File::open(parent)?.sync_all()
    })
    .await
    .map_err(std::io::Error::from)??;
    Ok(())
}

pub(crate) async fn commit_segment(
    file: &mut File,
    config: &Config,
    storage_key_id: [u8; 16],
    commit: SegmentCommit,
    buffer: Option<DirectIoBuffer>,
) -> Result<(u64, DirectIoBuffer)> {
    let buffer = encode_control_page(config, storage_key_id, commit, buffer)?;
    let buffer = write_page(
        file,
        buffer,
        config.segment_control_offset(commit.sg_index),
        config.write_max_time_us,
        "SG control page write",
    )
    .await?;
    file.sync_data().await?;
    Ok((BUCKET_BYTES as u64, buffer))
}

pub(crate) async fn invalidate_segment(
    file: &mut File,
    config: &Config,
    sg_index: usize,
    buffer: Option<DirectIoBuffer>,
) -> Result<(u64, DirectIoBuffer)> {
    let mut buffer = buffer.unwrap_or_else(|| DirectIoBuffer::zeroed(BUCKET_BYTES));
    debug_assert_eq!(buffer.len(), BUCKET_BYTES);
    buffer.fill(0);
    let buffer = write_page(
        file,
        buffer,
        config.segment_control_offset(sg_index),
        config.write_max_time_us,
        "SG control page invalidation",
    )
    .await?;
    file.sync_data().await?;
    Ok((BUCKET_BYTES as u64, buffer))
}

fn encode_file_header(config: &Config, storage_key_id: [u8; 16]) -> DirectIoBuffer {
    let mut bytes = DirectIoBuffer::zeroed(BUCKET_BYTES);
    bytes[..8].copy_from_slice(FILE_MAGIC);
    bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    bytes[28..36].copy_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes[36..40].copy_from_slice(&(config.segment_count as u32).to_le_bytes());
    bytes[40..44].copy_from_slice(&(BUCKET_BYTES as u32).to_le_bytes());
    bytes[44..52].copy_from_slice(&(config.blob_segment_size as u64).to_le_bytes());
    bytes[52..56].copy_from_slice(&(config.bucket_choice_count as u32).to_le_bytes());
    let checksum = checksum(&bytes[..BUCKET_BYTES - 4]);
    bytes[BUCKET_BYTES - 4..].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn validate_file_header(bytes: &[u8], config: &Config, storage_key_id: [u8; 16]) -> Result<()> {
    let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if &bytes[..8] != FILE_MAGIC
        || (format_version != LEGACY_FORMAT_VERSION && format_version != FORMAT_VERSION)
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
    let blob_segment_size = u64::from_le_bytes(bytes[44..52].try_into().unwrap());
    let bucket_choice_count = if format_version == LEGACY_FORMAT_VERSION {
        // Version 2 did not persist this field. Its production default was two;
        // nondefault experimental v2 storage requires repopulation.
        2
    } else {
        u32::from_le_bytes(bytes[52..56].try_into().unwrap()) as usize
    };
    if segment_size != config.segment_size as u64
        || segment_count != config.segment_count
        || blob_segment_size != config.blob_segment_size as u64
    {
        return Err(KvError::Worker(
            "Segment file does not match the configured Segment geometry".into(),
        ));
    }
    validate_bucket_choice_count(bucket_choice_count, "Segment file")?;
    Ok(())
}

fn validate_bucket_choice_count(stored: usize, source: &str) -> Result<()> {
    if !(1..=32).contains(&stored) || !stored.is_power_of_two() {
        return Err(KvError::Worker(format!(
            "{source} contains an invalid Bucket choice count"
        )));
    }
    Ok(())
}

fn validate_stored_bucket_choice_count(stored: usize, config: &Config, source: &str) -> Result<()> {
    validate_bucket_choice_count(stored, source)?;
    if stored > config.bucket_choice_count {
        return Err(KvError::Worker(format!(
            "{source} uses {stored} Bucket choices, but the configured {} cannot safely locate its records",
            config.bucket_choice_count
        )));
    }
    Ok(())
}

fn encode_control_page(
    config: &Config,
    storage_key_id: [u8; 16],
    commit: SegmentCommit,
    buffer: Option<DirectIoBuffer>,
) -> Result<DirectIoBuffer> {
    if commit.bucket_choice_count != config.bucket_choice_count {
        return Err(KvError::Worker(
            "Segment commit Bucket choice count does not match the active configuration".into(),
        ));
    }
    let sg_index = u32::try_from(commit.sg_index)
        .map_err(|_| KvError::Worker("SG index does not fit the control page".into()))?;
    let blob_logical_len = u64::try_from(commit.blob_logical_len)
        .map_err(|_| KvError::Worker("Blob length does not fit the control page".into()))?;
    let mut bytes = buffer.unwrap_or_else(|| DirectIoBuffer::zeroed(BUCKET_BYTES));
    debug_assert_eq!(bytes.len(), BUCKET_BYTES);
    bytes.fill(0);
    bytes[..8].copy_from_slice(CONTROL_MAGIC);
    bytes[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[12..28].copy_from_slice(&storage_key_id);
    bytes[28..32].copy_from_slice(&sg_index.to_le_bytes());
    bytes[32..40].copy_from_slice(&commit.generation.to_le_bytes());
    bytes[40..48].copy_from_slice(&blob_logical_len.to_le_bytes());
    bytes[48..52].copy_from_slice(&(commit.bucket_choice_count as u32).to_le_bytes());
    bytes[52..60].copy_from_slice(&(config.segment_size as u64).to_le_bytes());
    bytes[60..64].copy_from_slice(&(config.segment_count as u32).to_le_bytes());
    bytes[64..72].copy_from_slice(&(config.blob_segment_size as u64).to_le_bytes());
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
    let format_version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    let stored_sg_index = u32::from_le_bytes(bytes[28..32].try_into().unwrap()) as usize;
    let generation = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let blob_logical_len =
        usize::try_from(u64::from_le_bytes(bytes[40..48].try_into().unwrap()))
            .map_err(|_| KvError::Worker("stored Blob length is too large".into()))?;
    let bucket_choice_count = if format_version == LEGACY_FORMAT_VERSION {
        // See validate_file_header for the legacy-default compatibility rule.
        2
    } else {
        u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize
    };
    let segment_size = u64::from_le_bytes(bytes[52..60].try_into().unwrap());
    let segment_count = u32::from_le_bytes(bytes[60..64].try_into().unwrap()) as usize;
    let blob_segment_size = u64::from_le_bytes(bytes[64..72].try_into().unwrap());
    if &bytes[..8] != CONTROL_MAGIC
        || (format_version != LEGACY_FORMAT_VERSION && format_version != FORMAT_VERSION)
        || bytes[12..28] != storage_key_id
        || stored_sg_index != sg_index
        || generation == 0
        || blob_logical_len > config.blob_segment_size
        || segment_size != config.segment_size as u64
        || segment_count != config.segment_count
        || blob_segment_size != config.blob_segment_size as u64
        || checksum(&bytes[..BUCKET_BYTES - 4])
            != u32::from_le_bytes(bytes[BUCKET_BYTES - 4..].try_into().unwrap())
    {
        return Err(KvError::Worker(format!(
            "SG control page {sg_index} is invalid"
        )));
    }
    validate_stored_bucket_choice_count(
        bucket_choice_count,
        config,
        &format!("SG control page {sg_index}"),
    )?;
    Ok(SegmentCommit {
        sg_index,
        generation,
        blob_logical_len,
        bucket_choice_count,
    })
}

async fn read_page(
    file: &File,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    read_page_into(
        file,
        DirectIoBuffer::for_read(BUCKET_BYTES),
        offset,
        timeout_us,
        operation,
    )
    .await
}

async fn read_page_into(
    file: &File,
    bytes: DirectIoBuffer,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    read_exact_direct(file, bytes, offset, BUCKET_BYTES, timeout_us, operation).await
}

async fn write_page(
    file: &mut File,
    bytes: DirectIoBuffer,
    offset: u64,
    timeout_us: u64,
    operation: &'static str,
) -> Result<DirectIoBuffer> {
    let bytes = write_all_direct(file, bytes, offset, BUCKET_BYTES, timeout_us, operation).await?;
    debug_assert_eq!(bytes.len(), BUCKET_BYTES);
    Ok(bytes)
}

fn checksum(bytes: &[u8]) -> u32 {
    crc32fast::hash(bytes)
}
