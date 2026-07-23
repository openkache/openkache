// Mutable SG placement plus packed page record encoding, compaction, and checksums.

#[derive(Clone, Debug)]
struct Record {
    kind: u8,
    page_choice: u8,
    sequence: u64,
    key: Vec<u8>,
    value: Vec<u8>,
}

impl Record {
    fn encoded_len(&self) -> usize {
        RECORD_HEADER + self.key.len() + self.value.len()
    }
}

struct MutableSg {
    bytes: Vec<u8>,
    region: usize,
    generation: u64,
    page_size: usize,
    record_count: usize,
    logical_bytes: u64,
}

enum MutableReplace {
    NotFound,
    Replaced(Location),
    NoSpace,
}

impl MutableSg {
    fn new(config: &Config, region: usize, generation: u64) -> Self {
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

    fn page(&self, page: usize) -> &[u8] {
        let start = page * self.page_size;
        &self.bytes[start..start + self.page_size]
    }

    fn page_mut(&mut self, page: usize) -> &mut [u8] {
        let start = page * self.page_size;
        &mut self.bytes[start..start + self.page_size]
    }

    fn choose_page(&self, hash: &[u8; 32], record_len: usize) -> Option<(usize, u8)> {
        let pages = self.bytes.len() / self.page_size;
        let first = page_hash(hash, 0, pages);
        let second = page_hash(hash, 1, pages);
        let first_used = page_used(self.page(first));
        let second_used = page_used(self.page(second));
        let first_fits = first_used + record_len <= self.page_size;
        let second_fits = second_used + record_len <= self.page_size;
        match (first_fits, second_fits) {
            (false, false) => None,
            (true, false) => Some((first, 0)),
            (false, true) => Some((second, 1)),
            (true, true) if first_used <= second_used => Some((first, 0)),
            (true, true) => Some((second, 1)),
        }
    }

    fn append(&mut self, mut record: Record, count_logical: bool) -> Option<Location> {
        let (page, choice) = self.choose_page(
            &Key::from(record.key.as_slice()).hashed_key().into_bytes(),
            record.encoded_len(),
        )?;
        record.page_choice = choice;
        append_page(self.page_mut(page), &record);
        self.record_count += 1;
        if count_logical {
            self.logical_bytes += (record.key.len() + record.value.len()) as u64;
        }
        Some(Location {
            region: self.region as u8,
            page_choice: choice,
        })
    }

    fn replace(
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

        let mut matches = Vec::new();
        for &page in &candidate_pages {
            matches.extend(
                matching_record_spans(self.page(page), &record.key)
                    .into_iter()
                    .map(|span| (page, span)),
            );
        }
        let Some(&(page, span)) = matches.iter().max_by_key(|(_, span)| span.sequence) else {
            return MutableReplace::NotFound;
        };

        if matches.len() == 1
            && page_used(self.page(page)) - span.len() + record.encoded_len() <= self.page_size
        {
            record.page_choice = span.page_choice;
            replace_page_record(self.page_mut(page), span, &record);
            if count_logical {
                self.logical_bytes += (record.key.len() + record.value.len()) as u64;
            }
            return MutableReplace::Replaced(Location {
                region: self.region as u8,
                page_choice: record.page_choice,
            });
        }

        let saved_pages = candidate_pages
            .iter()
            .map(|&page| (page, self.page(page).to_vec()))
            .collect::<Vec<_>>();
        let saved_record_count = self.record_count;
        let saved_logical_bytes = self.logical_bytes;
        let removed = candidate_pages
            .iter()
            .map(|&page| remove_key_from_page(self.page_mut(page), &record.key))
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

    fn find(&self, hash: &[u8; 32], key: &[u8], choice: u8) -> Option<Record> {
        let page = page_hash(hash, choice, self.bytes.len() / self.page_size);
        latest_in_page(self.page(page), key)
    }

    fn finalize(&mut self) {
        let pages = self.bytes.len() / self.page_size;
        for page in 0..pages {
            finalize_page(self.page_mut(page));
        }
    }
}

fn initialize_page(page: &mut [u8], generation: u64) {
    page.fill(0);
    put_u32(page, 0, PAGE_MAGIC);
    put_u16(page, 4, PAGE_VERSION);
    put_u16(page, 6, PAGE_HEADER as u16);
    put_u64(page, 8, generation);
    put_u16(page, 16, PAGE_HEADER as u16);
    put_u16(page, 18, 0);
    put_u64(page, 20, 0);
}

fn page_used(page: &[u8]) -> usize {
    get_u16(page, 16) as usize
}

#[derive(Clone, Copy)]
struct RecordSpan {
    start: usize,
    end: usize,
    sequence: u64,
    page_choice: u8,
}

impl RecordSpan {
    fn len(self) -> usize {
        self.end - self.start
    }
}

fn append_page(page: &mut [u8], record: &Record) {
    let used = page_used(page);
    let end = used + record.encoded_len();
    write_record(page, used, record);
    put_u16(page, 16, end as u16);
    put_u16(page, 18, get_u16(page, 18) + 1);
    put_u64(page, 20, 0);
}

fn write_record(page: &mut [u8], offset: usize, record: &Record) {
    let end = offset + record.encoded_len();
    page[offset] = record.kind;
    page[offset + 1] = record.page_choice;
    put_u16(page, offset + 2, record.key.len() as u16);
    put_u32(page, offset + 4, record.value.len() as u32);
    put_u64(page, offset + 8, record.sequence);
    let key_end = offset + RECORD_HEADER + record.key.len();
    page[offset + RECORD_HEADER..key_end].copy_from_slice(&record.key);
    page[key_end..end].copy_from_slice(&record.value);
}

fn matching_record_spans(page: &[u8], key: &[u8]) -> Vec<RecordSpan> {
    let used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut offset = PAGE_HEADER;
    let mut result = Vec::new();
    for _ in 0..count {
        if offset + RECORD_HEADER > used {
            break;
        }
        let key_len = get_u16(page, offset + 2) as usize;
        let value_len = get_u32(page, offset + 4) as usize;
        let end = offset + RECORD_HEADER + key_len + value_len;
        if end > used {
            break;
        }
        if &page[offset + RECORD_HEADER..offset + RECORD_HEADER + key_len] == key {
            result.push(RecordSpan {
                start: offset,
                end,
                sequence: get_u64(page, offset + 8),
                page_choice: page[offset + 1],
            });
        }
        offset = end;
    }
    result
}

fn replace_page_record(page: &mut [u8], span: RecordSpan, record: &Record) {
    let old_used = page_used(page);
    let new_end = span.start + record.encoded_len();
    let new_used = old_used - span.len() + record.encoded_len();
    page.copy_within(span.end..old_used, new_end);
    if new_used < old_used {
        page[new_used..old_used].fill(0);
    }
    write_record(page, span.start, record);
    put_u16(page, 16, new_used as u16);
    put_u64(page, 20, 0);
}

fn remove_key_from_page(page: &mut [u8], key: &[u8]) -> usize {
    let old_used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut read = PAGE_HEADER;
    let mut write = PAGE_HEADER;
    let mut removed = 0usize;
    for _ in 0..count {
        if read + RECORD_HEADER > old_used {
            break;
        }
        let key_len = get_u16(page, read + 2) as usize;
        let value_len = get_u32(page, read + 4) as usize;
        let end = read + RECORD_HEADER + key_len + value_len;
        if end > old_used {
            break;
        }
        let matches = &page[read + RECORD_HEADER..read + RECORD_HEADER + key_len] == key;
        if matches {
            removed += 1;
        } else {
            if write != read {
                page.copy_within(read..end, write);
            }
            write += end - read;
        }
        read = end;
    }
    page[write..old_used].fill(0);
    put_u16(page, 16, write as u16);
    put_u16(page, 18, (count - removed) as u16);
    put_u64(page, 20, 0);
    removed
}

fn records(page: &[u8]) -> Vec<Record> {
    if page.len() < PAGE_HEADER
        || get_u32(page, 0) != PAGE_MAGIC
        || get_u16(page, 4) != PAGE_VERSION
    {
        return Vec::new();
    }
    let used = page_used(page).min(page.len());
    let count = get_u16(page, 18) as usize;
    let mut offset = PAGE_HEADER;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + RECORD_HEADER > used {
            break;
        }
        let key_len = get_u16(page, offset + 2) as usize;
        let value_len = get_u32(page, offset + 4) as usize;
        let end = offset + RECORD_HEADER + key_len + value_len;
        if end > used {
            break;
        }
        let key_end = offset + RECORD_HEADER + key_len;
        result.push(Record {
            kind: page[offset],
            page_choice: page[offset + 1],
            sequence: get_u64(page, offset + 8),
            key: page[offset + RECORD_HEADER..key_end].to_vec(),
            value: page[key_end..end].to_vec(),
        });
        offset = end;
    }
    result
}

fn latest_in_page(page: &[u8], key: &[u8]) -> Option<Record> {
    records(page)
        .into_iter()
        .filter(|record| record.key == key)
        .max_by_key(|record| record.sequence)
}

fn finalize_page(page: &mut [u8]) {
    put_u64(page, 20, 0);
    let checksum = checksum64(page);
    put_u64(page, 20, checksum);
}

fn verify_page(page: &[u8]) -> bool {
    if page.len() < PAGE_HEADER || get_u32(page, 0) != PAGE_MAGIC {
        return false;
    }
    let expected = get_u64(page, 20);
    let mut copy = page.to_vec();
    put_u64(&mut copy, 20, 0);
    expected != 0 && expected == checksum64(&copy)
}

fn page_hash(hash: &[u8; 32], choice: u8, pages: usize) -> usize {
    // hash[0..8] routes to a worker and hash[8..16] feeds the Breadcrumb.
    // The two page choices use independent portions of the digest.
    let start = if choice == 0 { 16 } else { 24 };
    u64::from_le_bytes(hash[start..start + 8].try_into().unwrap()) as usize % pages
}
