// Cache API and mutable-SG lifecycle, including compacting hot-key updates.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetOutcome {
    Created,
    Replaced,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct KvkacheIoStats {
    pub(crate) data_written: u64,
    pub(crate) data_read: u64,
    pub(crate) index_written: u64,
    pub(crate) index_read: u64,
}

#[derive(Default)]
struct IoCounters {
    data_written: Cell<u64>,
    data_read: Cell<u64>,
    index_written: Cell<u64>,
    index_read: Cell<u64>,
}

#[derive(Clone)]
struct LocatedRecord {
    location: Location,
    record: Record,
}

pub(crate) struct Kvkache {
    config: Config,
    data: File,
    index: LocationBreadcrumb,
    active: Option<MutableSg>,
    slot_generations: Vec<Option<u64>>,
    next_slot: usize,
    next_generation: u64,
    next_sequence: u64,
    pub(crate) data_flushes: u64,
    evictions: u64,
    io: IoCounters,
}

impl Kvkache {
    pub(crate) async fn open(config: Config) -> Result<Self> {
        config.validate()?;
        if let Some(parent) = config.data_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = config.index_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&config.data_path)
            .await?;
        data.set_len(config.data_bytes()).await?;

        let mut cache = Self {
            index: LocationBreadcrumb::new(&config)?,
            active: None,
            slot_generations: vec![None; config.sg_count],
            next_slot: 0,
            next_generation: 0,
            next_sequence: 0,
            data_flushes: 0,
            evictions: 0,
            io: IoCounters::default(),
            config,
            data,
        };
        if cache.config.recovery_enabled && !cache.load_checkpoint().await? {
            if !cache.config.fallback_to_sg_scan {
                return Err(KvError::Corrupt(
                    "checkpoint is absent or invalid and SG fallback is disabled".into(),
                ));
            }
            cache.rebuild_from_data().await?;
            if cache.slot_generations.iter().any(Option::is_some) {
                cache.save_checkpoint().await?;
            }
        }
        Ok(cache)
    }

    pub(crate) async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let hash = Key::from(key).hashed_key().into_bytes();
        Ok(self
            .locate(&hash, key)
            .await?
            .filter(|located| located.record.kind == RECORD_SET)
            .map(|located| located.record.value))
    }

    async fn get_many(&self, keys: Vec<Vec<u8>>) -> Vec<Result<Option<Vec<u8>>>> {
        let count = keys.len();
        let mut pending = FuturesUnordered::new();
        for (index, key) in keys.into_iter().enumerate() {
            pending.push(async move { (index, self.get(&key).await) });
        }
        let mut results = (0..count).map(|_| None).collect::<Vec<_>>();
        while let Some((index, result)) = pending.next().await {
            results[index] = Some(result);
        }
        results
            .into_iter()
            .map(|result| result.expect("every get future completes"))
            .collect()
    }

    pub(crate) async fn set(&mut self, key: &[u8], value: &[u8]) -> Result<SetOutcome> {
        if key.len() > u16::MAX as usize || value.len() > u32::MAX as usize {
            return Err(KvError::RecordTooLarge {
                bytes: RECORD_HEADER + key.len() + value.len(),
                capacity: self.config.page_size - PAGE_HEADER,
            });
        }
        let record_len = RECORD_HEADER + key.len() + value.len();
        if record_len > self.config.page_size - PAGE_HEADER {
            return Err(KvError::RecordTooLarge {
                bytes: record_len,
                capacity: self.config.page_size - PAGE_HEADER,
            });
        }
        let hash = Key::from(key).hashed_key().into_bytes();
        let previous = self.locate(&hash, key).await?;
        let sequence = self.take_sequence();
        let record = Record {
            kind: RECORD_SET,
            page_choice: 0,
            sequence,
            key: key.to_vec(),
            value: value.to_vec(),
        };
        let previous_is_active = previous.as_ref().is_some_and(|previous| {
            self.active
                .as_ref()
                .is_some_and(|active| active.region == previous.location.region as usize)
        });
        let location = if previous_is_active {
            match self
                .active
                .as_mut()
                .unwrap()
                .replace(&hash, record.clone(), true)
            {
                MutableReplace::Replaced(location) => location,
                MutableReplace::NotFound | MutableReplace::NoSpace => {
                    self.append_with_retry(record, true).await?
                }
            }
        } else {
            self.append_with_retry(record, true).await?
        };
        if let Some(previous) = &previous {
            if !self
                .index
                .replace_location(&hash, previous.location, location)
            {
                return Err(KvError::Corrupt(
                    "updated key is missing from the Breadcrumb".into(),
                ));
            }
        } else {
            self.index.insert(&hash, location)?;
        }
        Ok(if previous.is_some() {
            SetOutcome::Replaced
        } else {
            SetOutcome::Created
        })
    }

    pub(crate) async fn delete(&mut self, key: &[u8]) -> Result<bool> {
        let hash = Key::from(key).hashed_key().into_bytes();
        let Some(previous) = self.locate(&hash, key).await? else {
            return Ok(false);
        };
        let sequence = self.take_sequence();
        let tombstone = Record {
            kind: RECORD_DELETE,
            page_choice: 0,
            sequence,
            key: key.to_vec(),
            value: Vec::new(),
        };
        self.append_with_retry(tombstone, true).await?;
        let removed = self.index.remove(&hash, previous.location);
        debug_assert!(removed);
        Ok(true)
    }

    pub(crate) async fn sync(&mut self) -> Result<()> {
        let checkpointed_by_flush = self.active.is_some() && self.config.checkpoint_on_sg_flush;
        self.flush_active().await?;
        if !checkpointed_by_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }

    async fn locate(&self, hash: &[u8; 32], key: &[u8]) -> Result<Option<LocatedRecord>> {
        let mut latest: Option<LocatedRecord> = None;
        for location in self.index.candidates(hash) {
            let record = if self
                .active
                .as_ref()
                .is_some_and(|active| active.region == location.region as usize)
            {
                self.active
                    .as_ref()
                    .unwrap()
                    .find(hash, key, location.page_choice)
            } else {
                self.read_location(hash, key, location).await?
            };
            if let Some(record) = record
                && latest
                    .as_ref()
                    .is_none_or(|current| record.sequence > current.record.sequence)
            {
                latest = Some(LocatedRecord { location, record });
            }
        }
        Ok(latest)
    }

    async fn read_location(
        &self,
        hash: &[u8; 32],
        key: &[u8],
        location: Location,
    ) -> Result<Option<Record>> {
        if self.slot_generations[location.region as usize].is_none() {
            return Ok(None);
        }
        let page = page_hash(hash, location.page_choice, self.config.page_count());
        let bytes = self.read_page(location.region as usize, page).await?;
        Ok(latest_in_page(&bytes, key))
    }

    async fn append_with_retry(&mut self, record: Record, count_logical: bool) -> Result<Location> {
        loop {
            self.ensure_active().await?;
            if let Some(location) = self
                .active
                .as_mut()
                .unwrap()
                .append(record.clone(), count_logical)
            {
                return Ok(location);
            }
            self.flush_active().await?;
        }
    }

    async fn ensure_active(&mut self) -> Result<()> {
        if self.active.is_some() {
            return Ok(());
        }
        let region = self.next_slot;
        if self.slot_generations[region].is_some() {
            self.evict_region(region).await?;
        }
        let generation = self.next_generation;
        self.next_generation += 1;
        self.active = Some(MutableSg::new(&self.config, region, generation));
        Ok(())
    }

    async fn flush_active(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        if active.record_count == 0 {
            return Ok(());
        }
        active.finalize();
        let offset = active.region as u64 * self.config.sg_size as u64;
        let bytes = active.bytes;
        let write = self.data.write_all_at(bytes, offset);
        let BufResult(result, bytes) = compio::runtime::time::timeout(
            Duration::from_micros(self.config.write_max_time_us),
            write,
        )
        .await
        .map_err(|_| KvError::Timeout("SG write"))?;
        result?;
        self.io
            .data_written
            .set(self.io.data_written.get() + bytes.len() as u64);
        self.data.sync_data().await?;
        self.slot_generations[active.region] = Some(active.generation);
        self.next_slot = (active.region + 1) % self.config.sg_count;
        self.data_flushes += 1;
        if self.config.checkpoint_on_sg_flush {
            self.save_checkpoint().await?;
        }
        Ok(())
    }
}
