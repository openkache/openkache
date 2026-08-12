//! Policy-only decisions shared by the keyed mutation and capacity paths.
//!
//! Keeping expiration, conditional-write, and value-admission rules here makes
//! the storage coordinator independent from the wire/API operation that
//! requested the mutation.

use crate::protocol::SetCondition;
use crate::{BUCKET_BYTES, KvError, Result};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::{
    BLOB_ITEM_THRESHOLD_BYTES, ITEM_EXPIRATION_BYTES, ITEM_FIXED_BYTES, Item, ItemState, Kvkache,
    STORED_BLOB_REF_BYTES, STORED_LARGE_VALUE_REF_BYTES, STORED_VALUE_TAG_BYTES,
    item_offsets_bytes,
};

impl Kvkache {
    pub(super) fn validate_value(&self, value: &[u8], expiring: bool) -> Result<()> {
        if value.len() > self.config.max_item_bytes {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.max_item_bytes,
            });
        }
        let large = value.len() > self.config.large_value_threshold
            || value.len() > self.config.blob_segment_size;
        let stored_len = if large {
            STORED_LARGE_VALUE_REF_BYTES
        } else if value.len() > BLOB_ITEM_THRESHOLD_BYTES {
            STORED_BLOB_REF_BYTES
        } else {
            STORED_VALUE_TAG_BYTES + value.len()
        };
        let item_len =
            ITEM_FIXED_BYTES + if expiring { ITEM_EXPIRATION_BYTES } else { 0 } + stored_len;
        if item_len + item_offsets_bytes(1) + 1 > BUCKET_BYTES {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.max_item_bytes,
            });
        }
        if large && value.len() > self.config.large_value_capacity {
            return Err(KvError::ItemTooLarge {
                bytes: value.len(),
                capacity: self.config.large_value_capacity,
            });
        }
        Ok(())
    }
}

pub(super) fn item_state_is_live_at(state: ItemState, now_ms: u64) -> bool {
    !state.is_tombstone && (state.expires_at_ms == 0 || state.expires_at_ms > now_ms)
}

pub(super) fn item_state_is_live_now(state: ItemState) -> bool {
    !state.is_tombstone && (state.expires_at_ms == 0 || state.expires_at_ms > unix_time_ms())
}

pub(super) fn item_is_live_now(item: &Item) -> bool {
    item_state_is_live_now(ItemState {
        is_tombstone: item.is_tombstone,
        expires_at_ms: item.expires_at_ms,
        eviction_protected: item.eviction_protected,
    })
}

pub(super) fn set_condition_allows(condition: SetCondition, current_live: bool) -> bool {
    match condition {
        SetCondition::Any => true,
        SetCondition::IfAbsent => !current_live,
        SetCondition::IfPresent => current_live,
    }
}

pub(super) fn validate_ttl(ttl_ms: Option<u64>) -> Result<()> {
    let _ = ttl_deadline(ttl_ms)?;
    Ok(())
}

pub(super) fn ttl_deadline(ttl_ms: Option<u64>) -> Result<u64> {
    let Some(ttl_ms) = ttl_ms else {
        return Ok(0);
    };
    if ttl_ms == 0 {
        return Err(KvError::InvalidRequest(
            "SET TTL must be greater than zero milliseconds".into(),
        ));
    }
    unix_time_ms()
        .checked_add(ttl_ms)
        .ok_or_else(|| KvError::InvalidRequest("SET TTL exceeds the supported time range".into()))
}

/// Returns a Unix-epoch-compatible timestamp driven by a monotonic clock.
///
/// Deadlines are persisted as Unix-epoch milliseconds so they remain
/// comparable after a restart. The wall clock is sampled once per process;
/// subsequent reads use `Instant` and therefore cannot move backwards when
/// the system clock is adjusted.
pub(super) fn unix_time_ms() -> u64 {
    static CLOCK: OnceLock<(u64, Instant)> = OnceLock::new();
    let (anchor_ms, anchor) = CLOCK.get_or_init(|| {
        let anchor_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        (anchor_ms, Instant::now())
    });
    anchor_ms.saturating_add(u64::try_from(anchor.elapsed().as_millis()).unwrap_or(u64::MAX))
}
