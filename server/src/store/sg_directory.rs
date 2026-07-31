//! Logical SG identity and backing lifetime transitions.

use std::rc::Rc;

use crate::*;

#[derive(Debug)]
pub(crate) struct RamBacking {
    pub(crate) sequence: u64,
    pub(crate) segment: DirectIoBuffer,
    pub(crate) blob_arena: BlobArena,
    pub(crate) large_value_arena: BlobArena,
}

#[derive(Debug)]
pub(crate) struct SsdBacking {
    pub(crate) sequence: u64,
    pub(crate) location: GenerationLocation,
    pub(crate) large_value_location: Option<LargeValueLocation>,
}

pub(crate) enum DirectoryEntry {
    Free,
    Mutable {
        lane: usize,
    },
    Sealed(Rc<RamBacking>),
    InFlight {
        readable: Rc<RamBacking>,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    },
    Stable(Rc<SsdBacking>),
    Evicting(Rc<SsdBacking>),
    Retiring(Rc<SsdBacking>),
}

pub(crate) enum ReadBacking {
    Mutable { lane: usize },
    Ram(Rc<RamBacking>),
    Ssd(Rc<SsdBacking>),
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
        self.entries[logical_sg_id as usize] = DirectoryEntry::Mutable { lane };
        Ok(logical_sg_id)
    }

    pub(crate) fn read_backing(&self, logical_sg_id: u32) -> Option<ReadBacking> {
        match self.entries.get(logical_sg_id as usize)? {
            DirectoryEntry::Mutable { lane } => Some(ReadBacking::Mutable { lane: *lane }),
            DirectoryEntry::Sealed(readable) | DirectoryEntry::InFlight { readable, .. } => {
                Some(ReadBacking::Ram(Rc::clone(readable)))
            }
            DirectoryEntry::Stable(backing) | DirectoryEntry::Evicting(backing) => {
                Some(ReadBacking::Ssd(Rc::clone(backing)))
            }
            DirectoryEntry::Free | DirectoryEntry::Retiring(_) => None,
        }
    }

    pub(crate) fn seal(&mut self, logical_sg_id: u32, backing: RamBacking) -> Result<()> {
        let entry = self.entry_mut(logical_sg_id)?;
        if !matches!(entry, DirectoryEntry::Mutable { .. }) {
            return Err(invalid_transition(logical_sg_id, "Mutable", "Sealed"));
        }
        *entry = DirectoryEntry::Sealed(Rc::new(backing));
        Ok(())
    }

    pub(crate) fn publish_inflight(
        &mut self,
        logical_sg_id: u32,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    ) -> Result<Rc<RamBacking>> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Sealed(readable) = entry else {
            return Err(invalid_transition(logical_sg_id, "Sealed", "InFlight"));
        };
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
        logical_sg_id: u32,
        location: GenerationLocation,
        large_value_location: Option<LargeValueLocation>,
    ) -> Result<()> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::InFlight {
            readable,
            location: reserved,
            large_value_location: reserved_large,
        } = entry
        else {
            return Err(invalid_transition(logical_sg_id, "InFlight", "Stable"));
        };
        if *reserved != location {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different physical reservation"
            )));
        }
        if *reserved_large != large_value_location {
            return Err(KvError::Worker(format!(
                "logical SG {logical_sg_id} completed a different large-value reservation"
            )));
        }
        *entry = DirectoryEntry::Stable(Rc::new(SsdBacking {
            sequence: readable.sequence,
            location,
            large_value_location,
        }));
        Ok(())
    }

    pub(crate) fn begin_eviction(&mut self, logical_sg_id: u32) -> Result<Rc<SsdBacking>> {
        let entry = self.entry_mut(logical_sg_id)?;
        let DirectoryEntry::Stable(backing) = entry else {
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
            Some(DirectoryEntry::Stable(_))
        )
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
