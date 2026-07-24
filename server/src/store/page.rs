//! Cache-line-friendly slotted pages and the mutable slot-group buffer.
//!
//! Each page keeps a packed `(fingerprint: u8, offset: u12)` directory at the
//! front and grows payloads backward from the end. A payload stores the record
//! kind, the remaining 31 SHA-256 bytes, and the value. The directory
//! fingerprint is the first SHA-256 byte, so the full fixed-size key is stored
//! exactly once across the hot directory and cold payload.

use super::codec::*;
use crate::types::HASHED_KEY_BYTES;
use crate::*;

pub(crate) const PAGE_MAGIC: u32 = 0x4b56_5031;
pub(crate) const PAGE_VERSION: u16 = 2;

pub(crate) const PAGE_HEADER: usize = 32;
pub(crate) const PAGE_HEADER_SIZE_OFFSET: usize = 6;
pub(crate) const PAGE_GENERATION_OFFSET: usize = 8;
pub(crate) const PAGE_RECORD_COUNT_OFFSET: usize = 16;
pub(crate) const PAGE_CHECKSUM_OFFSET: usize = 20;
pub(crate) const PAGE_DIRECTORY_ENTRY_BITS: usize = 20;
pub(crate) const PAGE_FINGERPRINT_BITS: usize = 8;
pub(crate) const PAGE_OFFSET_BITS: usize = 12;
pub(crate) const RECORD_FIXED_BYTES: usize = HASHED_KEY_BYTES;
pub(crate) const RECORD_SET: u8 = 1;
pub(crate) const RECORD_DELETE: u8 = 2;

/// Decoded page record using a fixed SHA-256 key.
#[derive(Clone, Debug)]
pub(crate) struct Record {
    pub(crate) kind: u8,
    pub(crate) page_choice: u8,
    pub(crate) key: [u8; HASHED_KEY_BYTES],
    pub(crate) value: Vec<u8>,
}

impl Record {
    /// Returns bytes consumed by the cold payload, excluding its directory entry.
    pub(crate) fn payload_len(&self) -> usize {
        RECORD_FIXED_BYTES + self.value.len()
    }
}

/// In-memory SG whose pages use the packed directory layout.
pub(crate) struct MutableSg {
    pub(crate) bytes: Vec<u8>,
    pub(crate) region: usize,
    pub(crate) generation: u64,
    pub(crate) page_size: usize,
    pub(crate) record_count: usize,
    pub(crate) logical_bytes: u64,
}

/// Result of replacing the one copy of a key in the mutable SG.
pub(crate) enum MutableReplace {
    NotFound,
    Replaced(Location),
    NoSpace,
}

impl MutableSg {
    /// Creates an empty initialized page for every page position in the SG.
    pub(crate) fn new(config: &Config, region: usize, generation: u64) -> Self {
        let mut sg = Self {
            bytes: vec![0; config.sg_size],
            region,
            generation,
            page_size: config.page_size,
            record_count: 0,
            logical_bytes: 0,
        };
        for page in 0..config.page_count() {
            initialize_page(sg.page_mut(page), generation);
        }
        sg
    }

    /// Returns one immutable page from the contiguous SG buffer.
    fn page(&self, page: usize) -> &[u8] {
        let start = page * self.page_size;
        &self.bytes[start..start + self.page_size]
    }

    /// Returns one mutable page from the contiguous SG buffer.
    fn page_mut(&mut self, page: usize) -> &mut [u8] {
        let start = page * self.page_size;
        &mut self.bytes[start..start + self.page_size]
    }

    /// Selects the less-used candidate page that can fit one payload and entry.
    pub(crate) fn choose_page(&self, hash: &[u8; 32], payload_len: usize) -> Option<(usize, u8)> {
        let pages = self.bytes.len() / self.page_size;
        let first = page_hash(hash, 0, pages);
        let second = page_hash(hash, 1, pages);
        let first_used = page_used(self.page(first));
        let second_used = page_used(self.page(second));
        let first_fits = page_can_fit(self.page(first), payload_len);
        let second_fits = page_can_fit(self.page(second), payload_len);
        match (first_fits, second_fits) {
            (false, false) => None,
            (true, false) => Some((first, 0)),
            (false, true) => Some((second, 1)),
            (true, true) if first_used <= second_used => Some((first, 0)),
            (true, true) => Some((second, 1)),
        }
    }

    /// Appends a record to one of its two candidate pages.
    pub(crate) fn append(&mut self, mut record: Record, count_logical: bool) -> Option<Location> {
        let (page, choice) = self.choose_page(&record.key, record.payload_len())?;
        record.page_choice = choice;
        if !append_page(self.page_mut(page), &record) {
            return None;
        }
        self.record_count += 1;
        if count_logical {
            self.logical_bytes += (HASHED_KEY_BYTES + record.value.len()) as u64;
        }
        Some(Location {
            region: self.region as u8,
            page_choice: choice,
        })
    }

    /// Replaces every matching mutable copy with one compacted record.
    pub(crate) fn replace(
        &mut self,
        hash: &[u8; 32],
        mut record: Record,
        count_logical: bool,
    ) -> MutableReplace {
        let page_count = self.bytes.len() / self.page_size;
        let first = page_hash(hash, 0, page_count);
        let second = page_hash(hash, 1, page_count);
        let mut candidate_pages = vec![first];
        if second != first {
            candidate_pages.push(second);
        }

        let matches = candidate_pages
            .iter()
            .flat_map(|&page| {
                matching_record_spans(self.page(page), hash)
                    .into_iter()
                    .map(move |span| (page, span))
            })
            .collect::<Vec<_>>();
        let Some(&(page, span)) = matches.last() else {
            return MutableReplace::NotFound;
        };

        if matches.len() == 1 {
            record.page_choice = if page == first { 0 } else { 1 };
            let mut replacement = self.page(page).to_vec();
            if replace_page_record(&mut replacement, span, &record) {
                self.page_mut(page).copy_from_slice(&replacement);
                if count_logical {
                    self.logical_bytes += (HASHED_KEY_BYTES + record.value.len()) as u64;
                }
                return MutableReplace::Replaced(Location {
                    region: self.region as u8,
                    page_choice: record.page_choice,
                });
            }
        }

        let saved_pages = candidate_pages
            .iter()
            .map(|&page| (page, self.page(page).to_vec()))
            .collect::<Vec<_>>();
        let saved_record_count = self.record_count;
        let saved_logical_bytes = self.logical_bytes;
        let removed = candidate_pages
            .iter()
            .map(|&page| remove_key_from_page(self.page_mut(page), hash))
            .sum::<usize>();
        self.record_count -= removed;

        if let Some(location) = self.append(record, count_logical) {
            return MutableReplace::Replaced(location);
        }

        for (page, bytes) in saved_pages {
            self.page_mut(page).copy_from_slice(&bytes);
        }
        self.record_count = saved_record_count;
        self.logical_bytes = saved_logical_bytes;
        MutableReplace::NoSpace
    }

    /// Finds a full-hash match in the selected candidate page.
    pub(crate) fn find(&self, hash: &[u8; 32], choice: u8) -> Option<Record> {
        let page = page_hash(hash, choice, self.bytes.len() / self.page_size);
        let mut record = latest_in_page(self.page(page), hash)?;
        record.page_choice = choice;
        Some(record)
    }

    /// Finalizes every page checksum before the SG is persisted.
    pub(crate) fn finalize(&mut self) {
        let pages = self.bytes.len() / self.page_size;
        for page in 0..pages {
            finalize_page(self.page_mut(page));
        }
    }
}

/// Initializes the durable page header and an empty packed directory.
pub(crate) fn initialize_page(page: &mut [u8], generation: u64) {
    page.fill(0);
    put_u32(page, 0, PAGE_MAGIC);
    put_u16(page, 4, PAGE_VERSION);
    put_u16(page, PAGE_HEADER_SIZE_OFFSET, PAGE_HEADER as u16);
    put_u64(page, PAGE_GENERATION_OFFSET, generation);
    page[PAGE_RECORD_COUNT_OFFSET] = 0;
    put_u64(page, PAGE_CHECKSUM_OFFSET, 0);
}

/// Returns the number of decoded records stored in a page.
pub(crate) fn page_record_count(page: &[u8]) -> usize {
    page.get(PAGE_RECORD_COUNT_OFFSET)
        .copied()
        .unwrap_or_default() as usize
}

/// Returns directory plus payload bytes currently occupied by the page.
pub(crate) fn page_used(page: &[u8]) -> usize {
    let count = page_record_count(page);
    directory_end(count) + page.len().saturating_sub(payload_start(page, count))
}

/// Returns the byte size needed for `count` packed 20-bit directory entries.
pub(crate) const fn directory_bytes(count: usize) -> usize {
    (count * PAGE_DIRECTORY_ENTRY_BITS).div_ceil(8)
}

/// Returns the first byte after the packed directory.
fn directory_end(count: usize) -> usize {
    PAGE_HEADER + directory_bytes(count)
}

/// Returns the lowest occupied payload byte, or the page end when empty.
fn payload_start(page: &[u8], count: usize) -> usize {
    if count == 0 {
        page.len()
    } else {
        directory_entry(page, count - 1)
            .map(|entry| entry.offset)
            .unwrap_or_default()
    }
}

/// Tests whether one more entry and payload fit without crossing.
fn page_can_fit(page: &[u8], payload_len: usize) -> bool {
    let count = page_record_count(page);
    count < u8::MAX as usize
        && payload_len >= RECORD_FIXED_BYTES
        && payload_start(page, count)
            .checked_sub(payload_len)
            .is_some_and(|start| start >= directory_end(count + 1))
}

/// Decoded view of one packed 20-bit directory entry.
#[derive(Clone, Copy)]
struct DirectoryEntry {
    fingerprint: u8,
    offset: usize,
}

/// Decodes one directory entry from its unaligned bit position.
fn directory_entry(page: &[u8], slot: usize) -> Option<DirectoryEntry> {
    let bit = PAGE_HEADER * 8 + slot * PAGE_DIRECTORY_ENTRY_BITS;
    let fingerprint = get_packed_bits(page, bit, PAGE_FINGERPRINT_BITS)? as u8;
    let offset = get_packed_bits(page, bit + PAGE_FINGERPRINT_BITS, PAGE_OFFSET_BITS)? as usize;
    Some(DirectoryEntry {
        fingerprint,
        offset,
    })
}

/// Encodes one directory entry at its unaligned bit position.
fn write_directory_entry(page: &mut [u8], slot: usize, entry: DirectoryEntry) {
    let bit = PAGE_HEADER * 8 + slot * PAGE_DIRECTORY_ENTRY_BITS;
    set_packed_bits(page, bit, PAGE_FINGERPRINT_BITS, entry.fingerprint as u16);
    set_packed_bits(
        page,
        bit + PAGE_FINGERPRINT_BITS,
        PAGE_OFFSET_BITS,
        entry.offset as u16,
    );
}

/// Reads a little-endian bit field that may cross byte boundaries.
fn get_packed_bits(bytes: &[u8], bit: usize, width: usize) -> Option<u16> {
    if bit + width > bytes.len() * 8 {
        return None;
    }
    let mut value = 0u16;
    for offset in 0..width {
        value |= (((bytes[(bit + offset) / 8] >> ((bit + offset) % 8)) & 1) as u16) << offset;
    }
    Some(value)
}

/// Writes a little-endian bit field that may cross byte boundaries.
fn set_packed_bits(bytes: &mut [u8], bit: usize, width: usize, value: u16) {
    for offset in 0..width {
        let target = bit + offset;
        let mask = 1u8 << (target % 8);
        if value & (1u16 << offset) == 0 {
            bytes[target / 8] &= !mask;
        } else {
            bytes[target / 8] |= mask;
        }
    }
}

/// Location of a decoded payload and its directory slot.
#[derive(Clone, Copy)]
pub(crate) struct RecordSpan {
    pub(crate) slot: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

/// Infers one payload span from its offset and the preceding entry.
fn record_span(page: &[u8], slot: usize) -> Option<RecordSpan> {
    let count = page_record_count(page);
    if slot >= count {
        return None;
    }
    let start = directory_entry(page, slot)?.offset;
    let end = if slot == 0 {
        page.len()
    } else {
        directory_entry(page, slot - 1)?.offset
    };
    (start >= directory_end(count) && start + RECORD_FIXED_BYTES <= end && end <= page.len())
        .then_some(RecordSpan { slot, start, end })
}

/// Decodes one structurally valid record from a directory slot.
fn record_at(page: &[u8], slot: usize) -> Option<Record> {
    let entry = directory_entry(page, slot)?;
    let span = record_span(page, slot)?;
    let kind = page[span.start];
    if kind != RECORD_SET && kind != RECORD_DELETE {
        return None;
    }
    let mut key = [0u8; HASHED_KEY_BYTES];
    key[0] = entry.fingerprint;
    key[1..].copy_from_slice(&page[span.start + 1..span.start + RECORD_FIXED_BYTES]);
    Some(Record {
        kind,
        page_choice: 0,
        key,
        value: page[span.start + RECORD_FIXED_BYTES..span.end].to_vec(),
    })
}

/// Appends a record if its new directory and payload regions do not overlap.
pub(crate) fn append_page(page: &mut [u8], record: &Record) -> bool {
    if (record.kind != RECORD_SET && record.kind != RECORD_DELETE)
        || !page_can_fit(page, record.payload_len())
    {
        return false;
    }
    let count = page_record_count(page);
    let end = payload_start(page, count);
    let start = end - record.payload_len();
    page[start] = record.kind;
    page[start + 1..start + RECORD_FIXED_BYTES].copy_from_slice(&record.key[1..]);
    page[start + RECORD_FIXED_BYTES..end].copy_from_slice(&record.value);
    write_directory_entry(
        page,
        count,
        DirectoryEntry {
            fingerprint: record.key[0],
            offset: start,
        },
    );
    page[PAGE_RECORD_COUNT_OFFSET] = (count + 1) as u8;
    put_u64(page, PAGE_CHECKSUM_OFFSET, 0);
    true
}

/// Returns full-key matches after screening the hot fingerprint directory.
pub(crate) fn matching_record_spans(page: &[u8], key: &[u8; 32]) -> Vec<RecordSpan> {
    (0..page_record_count(page))
        .filter_map(|slot| {
            let entry = directory_entry(page, slot)?;
            if entry.fingerprint != key[0] {
                return None;
            }
            let span = record_span(page, slot)?;
            (page[span.start + 1..span.start + RECORD_FIXED_BYTES] == key[1..]).then_some(span)
        })
        .collect()
}

/// Reinitializes and densely appends records while preserving generation.
fn rebuild_page(page: &mut [u8], records: &[Record]) -> bool {
    let generation = get_u64(page, PAGE_GENERATION_OFFSET);
    initialize_page(page, generation);
    records.iter().all(|record| append_page(page, record))
}

/// Rebuilds a page with one slot replaced, compacting its backward payloads.
pub(crate) fn replace_page_record(page: &mut [u8], span: RecordSpan, record: &Record) -> bool {
    let mut decoded = records(page);
    if span.slot >= decoded.len() {
        return false;
    }
    decoded[span.slot] = record.clone();
    let mut rebuilt = page.to_vec();
    if !rebuild_page(&mut rebuilt, &decoded) {
        return false;
    }
    page.copy_from_slice(&rebuilt);
    true
}

/// Removes all full-hash matches and compacts the page.
pub(crate) fn remove_key_from_page(page: &mut [u8], key: &[u8; 32]) -> usize {
    let mut decoded = records(page);
    let old_len = decoded.len();
    decoded.retain(|record| record.key != *key);
    let removed = old_len - decoded.len();
    if removed > 0 {
        let rebuilt = rebuild_page(page, &decoded);
        debug_assert!(rebuilt);
    }
    removed
}

/// Decodes every structurally valid record in directory order.
pub(crate) fn records(page: &[u8]) -> Vec<Record> {
    if page.len() < PAGE_HEADER
        || page.len() > (1usize << PAGE_OFFSET_BITS)
        || get_u32(page, 0) != PAGE_MAGIC
        || get_u16(page, 4) != PAGE_VERSION
    {
        return Vec::new();
    }
    (0..page_record_count(page))
        .map_while(|slot| record_at(page, slot))
        .collect()
}

/// Returns the newest matching record under the one-key-per-SG invariant.
pub(crate) fn latest_in_page(page: &[u8], key: &[u8; 32]) -> Option<Record> {
    let slot = matching_record_spans(page, key).last()?.slot;
    record_at(page, slot)
}

/// Writes the page checksum after zeroing its checksum field.
pub(crate) fn finalize_page(page: &mut [u8]) {
    put_u64(page, PAGE_CHECKSUM_OFFSET, 0);
    let checksum = checksum64(page);
    put_u64(page, PAGE_CHECKSUM_OFFSET, checksum);
}

/// Verifies the format version, directory boundaries, and whole-page checksum.
pub(crate) fn verify_page(page: &[u8]) -> bool {
    if page.len() < PAGE_HEADER
        || page.len() > (1usize << PAGE_OFFSET_BITS)
        || get_u32(page, 0) != PAGE_MAGIC
        || get_u16(page, 4) != PAGE_VERSION
        || get_u16(page, PAGE_HEADER_SIZE_OFFSET) as usize != PAGE_HEADER
        || records(page).len() != page_record_count(page)
    {
        return false;
    }
    let expected = get_u64(page, PAGE_CHECKSUM_OFFSET);
    let mut copy = page.to_vec();
    put_u64(&mut copy, PAGE_CHECKSUM_OFFSET, 0);
    expected != 0 && expected == checksum64(&copy)
}

/// Maps one of a key's two page choices into an SG page index.
pub(crate) fn page_hash(hash: &[u8; 32], choice: u8, pages: usize) -> usize {
    let start = if choice == 0 { 16 } else { 24 };
    u64::from_le_bytes(hash[start..start + 8].try_into().unwrap()) as usize % pages
}
