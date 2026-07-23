// SG eviction, asynchronous page I/O, checkpoint recovery, and engine statistics.

impl Kvkache {
    async fn evict_region(&mut self, region: usize) -> Result<()> {
        let records = self.read_sg_records(region).await?;
        let mut newest = HashMap::<Vec<u8>, Record>::new();
        for record in records {
            if newest
                .get(&record.key)
                .is_none_or(|current| record.sequence > current.sequence)
            {
                newest.insert(record.key.clone(), record);
            }
        }
        for record in newest
            .into_values()
            .filter(|record| record.kind == RECORD_SET)
        {
            let hash = Key::from(record.key.as_slice()).hashed_key().into_bytes();
            let location = Location {
                region: region as u8,
                page_choice: record.page_choice,
            };
            if self
                .locate(&hash, &record.key)
                .await?
                .is_some_and(|current| {
                    current.location == location && current.record.sequence == record.sequence
                })
            {
                let _ = self.index.remove(&hash, location);
            }
        }
        self.slot_generations[region] = None;
        self.evictions += 1;
        Ok(())
    }

    async fn read_page(&self, region: usize, page: usize) -> Result<Vec<u8>> {
        let offset =
            region as u64 * self.config.sg_size as u64 + page as u64 * self.config.page_size as u64;
        let read = self
            .data
            .read_exact_at(Vec::with_capacity(self.config.page_size), offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            read,
        )
        .await
        .map_err(|_| KvError::Timeout("page read"))?;
        result?;
        self.io
            .data_read
            .set(self.io.data_read.get() + bytes.len() as u64);
        Ok(bytes)
    }

    async fn read_sg_records(&self, region: usize) -> Result<Vec<Record>> {
        let mut result = Vec::new();
        for page in 0..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if verify_page(&bytes) {
                result.extend(records(&bytes));
            }
        }
        Ok(result)
    }

    async fn scan_slot_generation(&self, region: usize) -> Result<Option<u64>> {
        let first = self.read_page(region, 0).await?;
        if !verify_page(&first) {
            return Ok(None);
        }
        let generation = get_u64(&first, 8);
        for page in 1..self.config.page_count() {
            let bytes = self.read_page(region, page).await?;
            if !verify_page(&bytes) || get_u64(&bytes, 8) != generation {
                return Ok(None);
            }
        }
        Ok(Some(generation))
    }

    async fn rebuild_from_data(&mut self) -> Result<()> {
        let mut occupied = Vec::new();
        for region in 0..self.config.sg_count {
            if let Some(generation) = self.scan_slot_generation(region).await? {
                self.slot_generations[region] = Some(generation);
                occupied.push((generation, region));
            }
        }
        occupied.sort_unstable();
        let mut latest = HashMap::<Vec<u8>, (Record, Location)>::new();
        for (_, region) in &occupied {
            for record in self.read_sg_records(*region).await? {
                let location = Location {
                    region: *region as u8,
                    page_choice: record.page_choice,
                };
                if latest
                    .get(&record.key)
                    .is_none_or(|(current, _)| record.sequence > current.sequence)
                {
                    latest.insert(record.key.clone(), (record, location));
                }
            }
        }
        self.index = LocationBreadcrumb::new(&self.config)?;
        self.next_sequence = 0;
        for (record, location) in latest.into_values() {
            self.next_sequence = self.next_sequence.max(record.sequence + 1);
            if record.kind == RECORD_SET {
                let hash = Key::from(record.key.as_slice()).hashed_key().into_bytes();
                self.index.insert(&hash, location)?;
            }
        }
        if let Some((generation, region)) = occupied.last().copied() {
            self.next_generation = generation + 1;
            self.next_slot = (region + 1) % self.config.sg_count;
        }
        Ok(())
    }

    async fn save_checkpoint(&self) -> Result<()> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CHECKPOINT_MAGIC);
        push_u32(&mut bytes, CHECKPOINT_VERSION);
        for value in self.config.signature() {
            push_u64(&mut bytes, value);
        }
        push_u64(&mut bytes, self.next_slot as u64);
        push_u64(&mut bytes, self.next_generation);
        push_u64(&mut bytes, self.next_sequence);
        push_u64(&mut bytes, self.index.len as u64);
        push_u64(&mut bytes, self.index.front.len() as u64);
        push_u64(&mut bytes, self.index.back.len() as u64);
        push_u64(&mut bytes, self.index.back_group_count as u64);
        for generation in &self.slot_generations {
            push_u64(&mut bytes, generation.unwrap_or(NONE_GENERATION));
        }
        for bucket in &self.index.front {
            bytes.extend_from_slice(&bucket.bytes);
        }
        for bucket in &self.index.back {
            bytes.extend_from_slice(&bucket.bytes);
        }
        let checksum = checksum64(&bytes);
        push_u64(&mut bytes, checksum);

        let temporary = self.config.index_path.with_extension("index.tmp");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)
                .await?;
            let write = file.write_all_at(bytes, 0);
            let BufResult(result, returned) = compio::runtime::time::timeout(
                Duration::from_micros(self.config.write_max_time_us),
                write,
            )
            .await
            .map_err(|_| KvError::Timeout("checkpoint write"))?;
            result?;
            bytes = returned;
            file.sync_all().await?;
            file.close().await?;
        }
        compio::fs::rename(&temporary, &self.config.index_path).await?;
        self.io
            .index_written
            .set(self.io.index_written.get() + bytes.len() as u64);
        if let Some(parent) = self.config.index_path.parent() {
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY)
                .open(parent)
                .await?;
            directory.sync_all().await?;
            directory.close().await?;
        }
        Ok(())
    }

    async fn load_checkpoint(&mut self) -> Result<bool> {
        let checkpoint_read = compio::fs::read(&self.config.index_path);
        let mut bytes = match compio::runtime::time::timeout(
            Duration::from_micros(self.config.read_max_time_us),
            checkpoint_read,
        )
        .await
        .map_err(|_| KvError::Timeout("checkpoint read"))?
        {
            Ok(bytes) => {
                self.io
                    .index_read
                    .set(self.io.index_read.get() + bytes.len() as u64);
                bytes
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() < 16 {
            return Ok(false);
        }
        let stored_checksum = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
        bytes.truncate(bytes.len() - 8);
        if checksum64(&bytes) != stored_checksum {
            return Ok(false);
        }
        let mut cursor = Cursor::new(&bytes);
        if cursor.take(8)? != CHECKPOINT_MAGIC || cursor.u32()? != CHECKPOINT_VERSION {
            return Ok(false);
        }
        for expected in self.config.signature() {
            if cursor.u64()? != expected {
                return Ok(false);
            }
        }
        let next_slot = cursor.u64()? as usize;
        let next_generation = cursor.u64()?;
        let next_sequence = cursor.u64()?;
        let len = cursor.u64()? as usize;
        let front_count = cursor.u64()? as usize;
        let back_count = cursor.u64()? as usize;
        let back_group_count = cursor.u64()? as usize;
        if front_count != self.index.front.len()
            || back_count != self.index.back.len()
            || back_group_count != self.index.back_group_count
        {
            return Ok(false);
        }
        let mut generations = Vec::with_capacity(self.config.sg_count);
        for region in 0..self.config.sg_count {
            let value = cursor.u64()?;
            let generation = (value != NONE_GENERATION).then_some(value);
            if self.scan_slot_generation(region).await? != generation {
                return Ok(false);
            }
            generations.push(generation);
        }
        for bucket in &mut self.index.front {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        for bucket in &mut self.index.back {
            bucket.bytes.copy_from_slice(cursor.take(BUCKET_BYTES)?);
        }
        if !cursor.remaining().is_empty() {
            return Ok(false);
        }
        self.index.len = len;
        self.slot_generations = generations;
        self.next_slot = next_slot;
        self.next_generation = next_generation;
        self.next_sequence = next_sequence;
        Ok(true)
    }

    pub(crate) fn stats(&self) -> String {
        let io = self.io_stats();
        format!(
            "keys={} index_load={:.2}% index_memory={:.2}MiB ({:.3}B/planned-key) modeled_resident={:.2}MiB front_buckets={} front_capacity={} back_buckets={} back_capacity={} next_slot={} generations={} flushes={} evictions={} data_read={} data_written={} index_read={} index_written={}",
            self.index.len(),
            self.index.load_factor() * 100.0,
            self.index.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.index.memory_bytes() as f64 / self.config.index_capacity as f64,
            self.memory_bytes() as f64 / (1024.0 * 1024.0),
            self.index.front.len(),
            self.index.front_layout.capacity,
            self.index.back.len(),
            self.index.back_layout.capacity,
            self.next_slot,
            self.next_generation,
            self.data_flushes,
            self.evictions,
            io.data_read,
            io.data_written,
            io.index_read,
            io.index_written,
        )
    }

    // Used by the cross-prototype benchmark; the standalone CLI only reports
    // cumulative counters and therefore does not reset them.
    #[allow(dead_code)]
    pub(crate) fn reset_io_stats(&self) {
        self.io.data_written.set(0);
        self.io.data_read.set(0);
        self.io.index_written.set(0);
        self.io.index_read.set(0);
    }

    pub(crate) fn io_stats(&self) -> KvkacheIoStats {
        KvkacheIoStats {
            data_written: self.io.data_written.get(),
            data_read: self.io.data_read.get(),
            index_written: self.io.index_written.get(),
            index_read: self.io.index_read.get(),
        }
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.index.memory_bytes()
            // One mutable SG is the steady-state write buffer. It is released
            // after `sync`, but must be budgeted for active operation.
            + self.config.sg_size
            + self.slot_generations.capacity() * std::mem::size_of::<Option<u64>>()
    }
}
