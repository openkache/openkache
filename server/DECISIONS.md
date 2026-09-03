//! Storage state ownership and placement invariants.
//!
//! # Resume context
//!
//! This file and the current `src/storage/storage_state.rs` subtree are the source
//! of truth for the rewrite. Do not reconstruct this design from agent memory.
//! `storage_legacy` is reference code for concrete low-level mechanics only; its
//! ownership model, configuration surface, and completed-looking behavior are not
//! contracts for the rewrite.
//!
//! The rewrite is intentionally incomplete and may not compile while interfaces
//! are being settled. `table.rs` has the current Breadcrumb implementation,
//! `storage_state.rs` has the current SG lifecycle and flush direction, and
//! `sg.rs` still contains the unfinished Segment+Blob representation. The current
//! discussion is configuration and hardware geometry; it has not authorized a
//! config implementation. Preserve unrelated and uncommitted work.
//!
//! `StorageState` is the only owner allowed to change the Table, SG lifecycle,
//! physical-record allocation, read pins, flushing, or eviction. `sg.rs` and
//! `table.rs` provide storage primitives; they do not coordinate transitions.
//!
//! # Interfaces required from child modules
//!
//! `table.rs` must provide:
//!
//! - `Table::new(&StorageConfig)`;
//! - `Table::candidates(&StorageKey)`, returning an owned, newest-first snapshot
//!   that may live across an SSD `await`;
//! - `Table::insert(&StorageKey, TableLocation)`, adding one candidate for one
//!   live full key even when an identical encoded candidate already exists;
//! - `Table::remove_one(&StorageKey, TableLocation)`, used after removing one
//!   exact full key from a Mutable SG;
//! - `Table::remove_all(&StorageKey, TableLocation)`, used when immutable bytes
//!   cannot be changed and every route to the old bytes must be invalidated.
//!
//! `sg.rs` must provide:
//!
//! - `MutableSg::new(&StorageConfig)` and `value_fits_in_empty_sg`;
//! - `MutableSg::{lookup, try_insert_into_best_bucket, replace, remove}` using a
//!   full `StorageKey`;
//! - `MutableSg::try_insert_into_best_bucket` must reject the previous physical
//!   Bucket so SET never appends a new value beside the old value it will remove;
//! - `bucket_index_for_choice` must derive physical placement from the full
//!   `StorageKey` and a choice even when the SG is no longer Mutable;
//! - `read_candidate`, returning the same owned `CandidateLookup` as RAM lookup.
//!
//! `CandidateLookup::Value` means the full key matched.
//! `TableIdentityCollision` means the compact Table candidate did not resolve to
//! that full key. Bucket lookup never
//! removes a Table entry: it cannot distinguish a live compact-identity collision
//! from an invariant-breaking stale entry using only the queried key's Bucket.
//!
//! The later flush implementation in this file must provide the two state
//! transitions called below: opening reusable Mutable space after sealing the
//! oldest Mutable SG, and waking the detached flush future immediately after its
//! last overlapping SSD read is unpinned.
//!
//! # Table and Bucket placement
//!
//! A Table identity is `(table_index, fingerprint)`, and a Table candidate stores
//! `(sg_index, bucket_choice)`. Physical Bucket placement is derived from the
//! full [`StorageKey`] and `bucket_choice`, using key bits independent from the
//! compact Table identity. Therefore equal `TableLocation`s belonging to two
//! different full keys do not imply an equal physical Bucket. Lookup always
//! interprets a candidate using the full key being queried.
//!
//! Choices for one full key must resolve to distinct Buckets within an SG. Keys
//! sharing one compact Table identity use their remaining independent key bits,
//! so they are unlikely to collide on the same physical Bucket. The already-hashed
//! StorageKey is mixed with the choice; Bucket placement must not run BLAKE3 again.
//!
//! Each Bucket keeps one contiguous byte tag per Item, separate from its packed
//! offsets. The tag is selected directly from the already-hashed [`StorageKey`];
//! Bucket lookup does not hash the key again. It uses `memchr` to SIMD-scan the
//! tag bytes, then checks the Table identity and full key only at matching Item
//! slots. There is no additional Bucket-wide hash bitmap: the tag scan is the
//! negative filter as well as the candidate-slot search. A short-tag collision
//! may cause an extra full-key comparison but must never produce a false miss.
//!
//! Each admitted full key adds one Table entry. Different full keys may therefore
//! produce duplicate encoded `(table_index, fingerprint, sg_index, bucket_choice)`
//! entries. The duplicates preserve multiplicity but cannot identify which full
//! key owns which entry. Mutable deletion removes the exact bytes and one entry.
//! Immutable deletion removes every copy of the exact encoded route used by the
//! deleted key. This is route invalidation, not an assertion that equal
//! `TableLocation`s for different full keys imply one physical Bucket.
//!
//! The Table stores no recency metadata. If insertion exhausts its fixed
//! capacity, it admits the new candidate by displacing an existing candidate
//! selected by the Table's fixed policy; the victim is not called the oldest.
//! Its unreachable bytes remain garbage until their physical record is reused.
//! Table capacity, fingerprint width, and Bucket-choice count are configuration;
//! Table-index width, SG-index width, choice bits, location bits, and packed
//! Subtable capacity are derived.
//!
//! # Hardware baseline and terminology
//!
//! The production sizing baseline under discussion is one AWS `i8ge.xlarge`:
//! four Graviton4 vCPUs, 32 GiB RAM, and one 2,500 GB decimal local Nitro NVMe
//! instance-store device. The guest sees the automatically attached device as an
//! NVMe namespace such as `/dev/nvme1n1`; bare metal is not required to obtain or
//! measure that namespace. Instance-store data is ephemeral and is lost when
//! the instance is stopped or terminated.
//!
//! Do not use the phrase "NVMe arena" as if it were an NVMe or AWS object. In
//! this design it meant only the byte range OpenKache may use. Call that range
//! the storage file range, `[0, storage_file_bytes)`, or the usable NVMe bytes.
//! The exact namespace bytes must be read on the target host with
//! `blockdev --getsize64`; AWS's 2,500 GB figure is the hardware-sizing baseline.
//!
//! The current `openkache-remote` machine is a roughly 19.53 GiB x86_64 host,
//! not an `i8ge.xlarge`, and its idle OS or process measurements are not an
//! authority for the production geometry. In particular, an earlier unexplained
//! "10 GiB for OS, runtime, and I/O" remainder was rejected. Do not carry that
//! number forward as a configuration input or sizing requirement.
//!
//! `i8ge` is AArch64, so the x86 AVX-512 fingerprint path is unavailable on the
//! target. Fingerprint benchmarking there must use the scalar implementation or
//! a future ARM NEON path. Eight- and sixteen-bit fingerprints map naturally to
//! byte- and halfword-wide SIMD comparisons; twelve-bit fingerprints are packed
//! and require more extraction work.
//!
//! # Configuration derivation and unresolved sizing
//!
//! Configuration must flow from physical and format decisions into derived
//! metadata widths. Do not expose mutually redundant capacity knobs and then let
//! them disagree. The intended dependency order is:
//!
//! ```text
//! usable storage bytes and storage shard count
//!     -> bytes per shard and SG capacity
//!     -> logical SG-ID capacity and sg_index_bits
//!     -> bucket-choice bits and packed Table value bits
//!     -> fingerprint bits and Table byte budget
//!     -> maximum Table entries for the selected 64-byte layout
//! ```
//!
//! The external inputs currently worth retaining are the storage path and usable
//! bytes, SG capacity, Table byte budget, fingerprint width, Bucket-choice count,
//! and I/O queue depth. Exactly three Mutable SGs and the current one-buffer
//! reusable/Sealed/Flushing allowance are structural constants, not free tuning
//! knobs. `table_max_entries` should become a result of `table_bytes` plus the
//! derived field widths, rather than another independent external input.
//!
//! `storage_sg_count` is not yet derivable solely from `storage_file_bytes` and
//! the configured SG capacity. An SG record contains a fixed Segment and a
//! variable-length Blob, so SG capacity is a maximum buffer size, not necessarily
//! the bytes written by every sealed record. The full-size quotient is useful for
//! comparing layouts but is not an exact bound on simultaneously live records.
//! Until `sg.rs` proves a minimum sealed-record length or the outer record becomes
//! fixed-size, the logical SG-ID capacity or `sg_index_bits` must remain explicit.
//! Never use an average fill ratio as a correctness proof that a wrapped logical
//! SG index is already `Unused`.
//!
//! A provisional single-shard comparison uses 2,469,606,195,200 usable bytes,
//! three Bucket choices (two encoded bits), and four allocated SG buffers: the
//! three Mutable buffers plus one reusable/Sealed/Flushing buffer.
//!
//! ```text
//! SG capacity   full-size quotient   provisional SG bits   four buffers
//! 256 MiB       9,200                14                    1 GiB
//! 512 MiB       4,600                13                    2 GiB
//!   1 GiB       2,300                12                    4 GiB
//! ```
//!
//! These SG counts are full-size equivalents, not final logical slot counts.
//! With a 512 MiB capacity, 13 SG bits provide 8,192 identities. Filling the
//! stated storage range without exhausting those identities would require sealed
//! records to average at least about 302 MiB, or 56% of capacity. That observation
//! does not establish the required worst-case lower bound. Choosing 512 MiB is a
//! working benchmark point because moving to 1 GiB saves only one Table value bit
//! while doubling RAM per buffer and the flush/eviction scan unit. It is not a
//! finalized format decision.
//!
//! CPU count affects this derivation only through storage sharding. The current
//! `StorageState` is one single-thread-owned shard, so host CPU count does not
//! alter its SG-index width. Two storage shards would each own an independent
//! Table, logical SG space, physical byte range, and four-buffer pool; request
//! routing would choose a shard before Table lookup. That is an architecture
//! change, not a dormant config field to add in advance. Decide sharding before
//! freezing SG-index bits, but keep one shard while the current ownership model
//! remains unchanged.
//!
//! For the provisional 512 MiB, one-shard geometry, 13 SG bits plus two choice
//! bits produce a 15-bit Table value. The current 88%-target Table layout gives
//! the following modeled capacities for a 20 GiB Table:
//!
//! ```text
//! fingerprint   maximum entries   change from 8-bit   modeled extra candidates
//! 8 bits        6.13 billion      baseline            about 0.025 per miss
//! 12 bits       5.25 billion      -14%                about 0.00068 per miss
//! 16 bits       4.40 billion      -28%                about 0.00011 per miss
//! ```
//!
//! The last column assumes uniform hashes and the layout selected by the current
//! implementation; it is a model, not a benchmark result. A fingerprint collision
//! cannot return a wrong value because Bucket lookup validates the full key. Its
//! cost is an extra candidate Bucket read. The lookup-wide collision rate is not
//! simply `1 / 2^fingerprint_bits`; it also depends on how many entries share the
//! queried Table coordinate. Eight bits is the current working default because it
//! preserves the most indexed keys and is easy to optimize with NEON. Keep the
//! width configurable and benchmark 8, 12, and 16 bits on the actual `i8ge` before
//! freezing it.
//!
//! The existing config still couples
//! `storage_file_bytes = storage_sg_count * sg_bytes`, and the current Table
//! constructor is driven by `max_entries`. Those are legacy directions to replace,
//! not contracts to preserve. No sizing decision is complete until `sg.rs` defines
//! the Segment/Blob layout and a target-host benchmark measures sealed-record fill,
//! eviction scan time, false candidate reads, and Table throughput.
//!
//! I/O queue depth is not a memory bound. The current request loop removes queued
//! requests and detaches their futures without a separate in-flight admission cap,
//! so request payloads and read buffers can grow independently of the fixed SPSC
//! slots. Request intake remains a later design task, but final Table memory sizing
//! must not hide that unbounded state inside an arbitrary RAM-reserve number.
//!
//! # SG buffers and flushing
//!
//! One SG owns its fixed Bucket Segment and associated variable-length Blob as
//! a single allocation, flush, read-pin, and reuse unit. Mutable SG buffers are
//! allocated up front and reused. Resetting a returned buffer clears
//! authoritative Bucket/Blob metadata and logical lengths; bytes outside those
//! lengths need not be zeroed and must never be interpreted.
//! Multiple sealed or flushing SGs are allowed only within this fixed buffer
//! pool. Exhausting the pool applies backpressure instead of allocating.
//!
//! Mutable, Sealed, and Flushing SGs remain readable synchronously from RAM, so
//! those reads need no pin or read counter. Sealed freezes the buffer while it
//! waits for a writable physical range. Flushing begins only after that range is
//! claimed and its write SQE is submitted. Its frozen buffer remains in RAM
//! until the write CQE succeeds; only then is the SG published as Stable and the
//! buffer reset and returned to the pool. A failed write retains the buffer
//! because it is still the only valid copy.
//!
//! # Circular records and SSD read pins
//!
//! The storage file is a circular log of contiguous `[start, end)` records. One
//! record contains one SG's Segment and Blob; their internal order is an SG
//! implementation detail. Records never wrap across the file boundary: if a
//! record does not fit at the tail, the unused tail is skipped and the whole
//! record starts at offset zero.
//!
//! Only asynchronous SSD reads pin storage. A pin covers the entire physical
//! record from the Bucket read through any dependent Blob read and validation.
//! No generation identifier is used. Allocation order and each record's
//! `(start, end, pin_count)` are sufficient because a pinned record cannot be
//! overwritten.
//!
//! A sealed SG may target a range overlapping several old records. Selecting a
//! Stable victim changes it to Evicting and immediately rejects every new SSD
//! read, while reads that already own pins remain valid and drain normally.
//! Eviction reads each victim and removes one exact location candidate for every
//! stored full key; the removal is a no-op when that key already points elsewhere.
//! A victim becomes Unused only after both its eviction scan and Table cleanup
//! are complete and its existing pin count is zero. Every victim for one flush
//! owns a clone of that flush's `Event`; the last unpin notifies the detached
//! flush future, which rechecks all victims before claiming the complete range
//! and submitting the write. Physical victims are selected by byte-range overlap,
//! not by assuming that one new record replaces one old SG.
//!
//! Owns the storage-wide state transitions and their correctness invariants.
//!
//! # Table candidate removal
//!
//! DELETE does not write tombstones. When the old item is Mutable, its full-key
//! bytes are removed first and `Table::remove_one` removes one matching candidate.
//! Immutable bytes cannot be edited, so `Table::remove_all` removes every copy of
//! the old key's exact encoded `TableLocation`. Choices for one queried full key
//! map to distinct Buckets, so no different choice can route that key back to the
//! old Bucket. Routes to another choice, SG, or a SET's newly published location
//! remain intact.
//!
//! Another key may use the same encoded route while resolving it to a different
//! Bucket. `Table::remove_all` cannot distinguish those two full keys, so removing
//! the encoded route may make the other key return a false miss. This is
//! intentional: cache misses are allowed, but a deleted or stale value must never
//! be returned. The unreachable physical bytes remain garbage until eager
//! eviction scans and reclaims their SG record.
//!
//! # Reusable buffer backpressure
//!
//! A SET that cannot append first opens a returned buffer as a new Mutable SG.
//! If the pool is empty, it registers a listener and yields. Flush completion
//! returns one buffer and wakes every listener because one new Mutable SG can
//! accept several waiting SETs. Every woken SET retries append before attempting
//! to take another reusable buffer.
//!
//! # Mutable SG lookup window
//!
//! Mutable SGs occupy one contiguous window in the circular `sgs` array.
//! `StorageState` stores only `oldest_mutable_sg_index`. The three Mutable SGs are
//! that index and the next two circular indexes; the next logical SG is
//! `(oldest_mutable_sg_index + MUTABLE_SG_COUNT) % sgs.len()`. SET tries the oldest
//! Mutable first and advances only when its candidate Buckets cannot accept the
//! value. This intentionally fills the front SG before it is sealed and flushed.
//! SET examines exactly those three derived slots and never discovers Mutable SGs
//! by scanning other states.
//!
//! The circular `sgs` array is sized for the maximum simultaneously live logical
//! SGs: every Stable record that can fit in the file, exactly three Mutable SGs,
//! and the fixed maximum number of Sealed/Flushing SGs. Logical indexes are
//! allocated in cursor order and old physical records are evicted in allocation
//! order, so a wrapped next index must already be Unused. Finding any other state
//! there is an invariant violation, not an alternate placement case.
//!
//! Whether the previous value belongs to the SG currently being examined is an
//! explicit `match`, not an `Option::filter().map()` chain. When it does, the
//! previous Bucket choice is passed into `MutableSg`, which must exclude every
//! candidate choice mapping to that same physical Bucket before inserting.
