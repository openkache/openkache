//! Logical SG identity and backing lifetime transitions.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use crate::*;

#[derive(Debug)]
pub(crate) struct RamBacking {
    pub(crate) sequence: u64,
    pub(crate) segment: Arc<DirectIoBuffer>,
    pub(crate) blob_arena: BlobArena,
    pub(crate) large_value_arena: BlobArena,
}

#[derive(Debug)]
pub(crate) struct JobPin {
    count: Rc<Cell<usize>>,
}

impl JobPin {
    fn acquire(count: &Rc<Cell<usize>>) -> Self {
        count.set(count.get() + 1);
        Self {
            count: Rc::clone(count),
        }
    }
}

impl Drop for JobPin {
    fn drop(&mut self) {
        let count = self.count.get();
        debug_assert!(count != 0, "an SG job pin was released twice");
        self.count.set(count.saturating_sub(1));
    }
}

pub(crate) enum DirectoryEntry {
    Free,
    Mutable {
        lane: usize,
        job_pins: Rc<Cell<usize>>,
    },
    Closing {
        readable: Rc<RamBacking>,
        job_pins: Rc<Cell<usize>>,
    },
    InFlight {
        readable: Rc<RamBacking>,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    },
    Stable {
        backing: Rc<CommittedGenerationState>,
        readable: Option<Rc<RamBacking>>,
    },
    Evicting(Rc<CommittedGenerationState>),
    Retiring(Rc<CommittedGenerationState>),
}

pub(crate) enum ReadBacking {
    Mutable {
        lane: usize,
        _job_pin: JobPin,
    },
    Ram {
        backing: Rc<RamBacking>,
        retirement_guard: Option<Rc<CommittedGenerationState>>,
    },
    Ssd(Rc<CommittedGenerationState>),
}

pub(crate) struct SgDirectory {
    entries: Vec<DirectoryEntry>,
    free_ids: Vec<u32>,
}

impl SgDirectory {
    pub(crate) fn new(logical_id_capacity: usize) -> Result<Self> {
        if logical_id_capacity == 0 || logical_id_capacity > u32::MAX as usize {
            return Err(KvError::InvalidConfig(
                "logical SG directory capacity must fit in u32".into(),
            ));
        }
        Ok(Self {
            entries: (0..logical_id_capacity)
                .map(|_| DirectoryEntry::Free)
                .collect(),
            free_ids: (0..logical_id_capacity as u32).rev().collect(),
        })
    }

    pub(crate) fn allocate_mutable(&mut self, lane: usize) -> Result<u32> {
        let logical_sg_id = self
            .free_ids
            .pop()
            .ok_or_else(|| KvError::Worker("logical SG directory is exhausted".into()))?;
        self.entries[logical_sg_id as usize] = DirectoryEntry::Mutable {
            lane,
            job_pins: Rc::new(Cell::new(0)),
        };
        Ok(logical_sg_id)
    }

    pub(crate) fn restore_stable(&mut self, state: CommittedGenerationState) -> Result<()> {
        let logical_sg_id = state.location.logical_sg_id;
        if !matches!(
            self.entries.get(logical_sg_id as usize),
            Some(DirectoryEntry::Free)
        ) {
            return Err(KvError::Worker(format!(
                "persisted logical SG {logical_sg_id} is not free during recovery"
            )));
        }
        let removed = self
            .free_ids
            .iter()
            .position(|candidate| *candidate == logical_sg_id)
            .map(|index| self.free_ids.swap_remove(index));
        if removed.is_none() {
            return Err(KvError::Worker(format!(
                "persisted logical SG {logical_sg_id} is missing from the free directory"
            )));
        }
        let entry = self.entry_mut(logical_sg_id)?;
        *entry = DirectoryEntry::Stable {
            backing: Rc::new(state),
            readable: None,
        };
        Ok(())
    }

    pub(crate) fn stable_states(
        &self,
    ) -> impl Iterator<Item = (u32, &CommittedGenerationState)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(logical_sg_id, entry)| {
                let DirectoryEntry::Stable { backing, .. } = entry else {
                    return None;
                };
                Some((logical_sg_id as u32, backing.as_ref()))
            })
    }

    pub(crate) fn read_backing(&self, logical_sg_id: u32) -> Option<ReadBacking> {
        match self.entries.get(logical_sg_id as usize)? {
            DirectoryEntry::Mutable { lane, job_pins } => Some(ReadBacking::Mutable {
                lane: *lane,
                _job_pin: JobPin::acquire(job_pins),
            }),
            DirectoryEntry::Closing { readable, .. }
            | DirectoryEntry::InFlight { readable, .. } => Some(ReadBacking::Ram {
                backing: Rc::clone(readable),
                retirement_guard: None,
            }),
            DirectoryEntry::Stable {
                backing,
                readable: Some(readable),
            } => Some(ReadBacking::Ram {
                backing: Rc::clone(readable),
                retirement_guard: Some(Rc::clone(backing)),
            }),
            DirectoryEntry::Stable {
                backing,
                readable: None,
            }
            | DirectoryEntry::Evicting(backing) => Some(ReadBacking::Ssd(Rc::clone(backing))),
            DirectoryEntry::Free | DirectoryEntry::Retiring(_) => None,
        }
    }

    pub(crate) fn close(&mut self, logical_sg_id: u32, backing: RamBacking) -> Result<()> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Mutable { job_pins, .. } = entry else {
            return Err(invalid_transition(logical_sg_id, "Mutable", "Closing"));
        };
        let job_pins = Rc::clone(job_pins);
        *entry = DirectoryEntry::Closing {
            readable: Rc::new(backing),
            job_pins,
        };
        Ok(())
    }

    pub(crate) fn closing_backing_if_ready(
        &self,
        logical_sg_id: u32,
    ) -> Result<Option<Rc<RamBacking>>> {
        let entry = self.entries.get(logical_sg_id as usize).ok_or_else(|| {
            KvError::Worker(format!("logical SG {logical_sg_id} is out of range"))
        })?;
        let DirectoryEntry::Closing { readable, job_pins } = entry else {
            return Err(invalid_transition(logical_sg_id, "Closing", "InFlight"));
        };
        Ok((job_pins.get() == 0).then(|| Rc::clone(readable)))
    }

    pub(crate) fn publish_inflight(
        &mut self,
        logical_sg_id: u32,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    ) -> Result<Rc<RamBacking>> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Closing { readable, job_pins } = entry else {
            return Err(invalid_transition(logical_sg_id, "Closing", "InFlight"));
        };
        if job_pins.get() != 0 {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} still has active job pins"
            )));
        }
        let readable = Rc::clone(readable);
        *entry = DirectoryEntry::InFlight {
            readable: Rc::clone(&readable),
            location,
            large_value_location,
        };
        Ok(readable)
    }

    pub(crate) fn publish_stable(
        &mut self,
        state: CommittedGenerationState,
        retain_ram: bool,
    ) -> Result<()> {
        let logical_sg_id = state.location.logical_sg_id;
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::InFlight {
            readable,
            location: reserved,
            large_value_location: reserved_large,
        } = entry
        else {
            return Err(invalid_transition(logical_sg_id, "InFlight", "Stable"));
        };
        if readable.sequence != state.sequence {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different generation sequence"
            )));
        }
        if *reserved != state.location {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different physical reservation"
            )));
        }
        if *reserved_large != state.large_value_location {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different large-value reservation"
            )));
        }
        if state
            .large_value_location
            .is_some_and(|location| location.logical_sg_id != logical_sg_id)
        {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different large-value generation"
            )));
        }
        *entry = DirectoryEntry::Stable {
            backing: Rc::new(state),
            readable: retain_ram.then(|| Rc::clone(readable)),
        };
        Ok(())
    }

    pub(crate) fn drop_stable_ram(&mut self, logical_sg_id: u32) -> Result<bool> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Stable { readable, .. } = entry else {
            return Ok(false);
        };
        Ok(readable.take().is_some())
    }

    pub(crate) fn begin_eviction(
        &mut self,
        logical_sg_id: u32,
    ) -> Result<Rc<CommittedGenerationState>> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Stable { backing, .. } = entry else {
            return Err(invalid_transition(logical_sg_id, "Stable", "Evicting"));
        };
        let backing = Rc::clone(backing);
        *entry = DirectoryEntry::Evicting(Rc::clone(&backing));
        Ok(backing)
    }

    pub(crate) fn begin_retiring(&mut self, logical_sg_id: u32) -> Result<()> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Evicting(backing) = entry else {
            return Err(invalid_transition(logical_sg_id, "Evicting", "Retiring"));
        };
        let backing = Rc::clone(backing);
        *entry = DirectoryEntry::Retiring(backing);
        Ok(())
    }

    pub(crate) fn try_free_retiring(&mut self, logical_sg_id: u32) -> Result<bool> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Retiring(backing) = entry else {
            return Err(invalid_transition(logical_sg_id, "Retiring", "Free"));
        };
        if Rc::strong_count(backing) != 1 {
            return Ok(false);
        }
        *entry = DirectoryEntry::Free;
        self.free_ids.push(logical_sg_id);
        Ok(true)
    }

    #[allow(dead_code)]
    pub(crate) fn entry(&self, logical_sg_id: u32) -> Option<&DirectoryEntry> {
        self.entries.get(logical_sg_id as usize)
    }

    pub(crate) fn is_stable(&self, logical_sg_id: u32) -> bool {
        matches!(
            self.entries.get(logical_sg_id as usize),
            Some(DirectoryEntry::Stable { .. })
        )
    }

    pub(crate) fn ram_bytes(&self) -> usize {
        self.entries
            .iter()
            .filter_map(|entry| match entry {
                DirectoryEntry::Closing {
                    readable: backing, ..
                }
                | DirectoryEntry::InFlight {
                    readable: backing, ..
                } => Some(backing),
                DirectoryEntry::Stable {
                    readable: Some(backing),
                    ..
                } => Some(backing),
                _ => None,
            })
            .map(|backing| {
                backing.segment.capacity()
                    + backing.blob_arena.allocated_bytes()
                    + backing.large_value_arena.allocated_bytes()
            })
            .sum()
    }

    fn entry_mut(&mut self, logical_sg_id: u32) -> Result<&mut DirectoryEntry> {
        self.entries
            .get_mut(logical_sg_id as usize)
            .ok_or_else(|| KvError::Worker(format!("logical SG {logical_sg_id} is out of range")))
    }
}

fn invalid_transition(logical_sg_id: u32, from: &str, to: &str) -> KvError {
    KvError::Worker(format!(
        "logical SG {logical_sg_id} cannot transition from a non-{from} state to {to}"
    ))
}
