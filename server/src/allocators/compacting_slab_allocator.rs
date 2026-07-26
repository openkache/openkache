//! Compacting slab allocator for efficient memory allocation.
//!
//! Provides a handle-based slab allocator ([`Slab`]) built on top of
//! [`VirtualPageStack`]. Values are stored in contiguous slots with compacting
//! deletion (swap-remove). Each slot holds a back-pointer to its [`Handle`],
//! enabling O(1) relocation during compaction and constant-time access.

use std::cell::{Cell, UnsafeCell};
use std::marker::{PhantomData, PhantomPinned};
use std::mem::{ManuallyDrop, MaybeUninit};
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::ptr::{self, NonNull};

use crate::allocators::virtual_page_stack::{VirtualPageStack, VirtualPageStackError};

/// The header part of a Handle that is tracked by the Slab.
///
/// This provides a stable memory location for the index that the Slab can
/// update during compactions without needing to know the full layout of `Handle<T>`.
#[repr(C)]
pub struct HandleHeader<T> {
    pub index: Cell<usize>,
    /// Invariant over T: Handle provides &mut T access, so covariance would
    /// be unsound. `fn(T) -> T` makes HandleHeader invariant over T.
    _marker: PhantomData<fn(T) -> T>,
}

/// # Slot State Machine (Conceptual)
///
/// Each slot follows a four-state lifecycle. The states are **not materialized at
/// runtime** (no tag byte) — they are encoded implicitly across
/// `handle_header` and the value's initialization status. Documenting them
/// explicitly makes auditing, panic analysis, and future shrink support tractable.
///
/// ```text
///  ┌───────┐  insert   ┌──────┐ delete (swap)  ┌─────────┐ write moved  ┌──────┐
///  │ Fresh │──────────▶│ Live │───────────────▶│ Moving  │─────────────▶│ Live │
///  └───────┘           └──────┘                └─────────┘              └──────┘
///                        │                                                │
///                        │ delete (tail)                                  │ delete
///                        ▼                                                ▼
///                      ┌─────────┐   drop + poison              ┌───────┐
///                      │ Vacated │─────────────────────────────▶│ Fresh │
///                      └─────────┘                              └───────┘
/// ```
///
/// | State     | `handle_header`     | `value`       | In `0..len`? |
/// |-----------|---------------------|---------------|--------------|
/// | `Fresh`   | zero / stale        | zero / poison | No           |
/// | `Live`    | → valid Handle      | initialized   | Yes          |
/// | `Moving`  | → valid Handle      | being moved   | Yes (transient) |
/// | `Vacated` | stale (tombstoned)  | dropped       | No           |
///
/// **Transitions:**
/// - `Fresh → Live`: [`Slab::insert`] — value written, handle_header linked, added to `0..len`.
/// - `Live → Moving → Live`: Swap branch of [`Slab::delete`] — last slot relocated to hole.
/// - `Live → Vacated`: Value dropped, handle tombstoned. Slot exits `0..len`.
/// - `Vacated → Fresh`: Implicit — vacated slots sit beyond `len` until reused by next insert.
#[repr(C)]
pub(crate) struct Slot<T> {
    // Fields ordered for cache locality: delete path touches metadata first,
    // avoiding dragging the value cacheline for large T.
    pub(crate) handle_header: NonNull<HandleHeader<T>>,
    pub(crate) value: ManuallyDrop<T>,
}
/// A contiguous, high-performance memory store where every element is tracked
/// by exactly one pinned [`Handle`].
///
/// Each slot embeds both the value and the back-pointer to its owning Handle,
/// giving perfect cache locality on deletion (one cache line touches both the
/// data being moved and the handle pointer that needs patching).
///
/// # Structural Invariant
///
/// For every valid index `i`: `slots[i].handle.index == i`
///
/// # Capacity Invariant
///
/// The following chain always holds:
///
/// ```text
/// index < len <= cap <= committed_slots
/// ```
///
/// Where `committed_slots` is the number of slots covered by committed
/// (non-decommitted) virtual pages. **Any future shrink operation** (page
/// decommit) **MUST** guarantee that all live slots (`0..len`) remain within
/// committed memory. Violating this causes silent use-after-unmap UB.
///
/// # Safety Contract (Intrusive Lifetime)
///
/// This is an intrusive container — similar to Linux kernel linked lists or
/// Tokio's intrusive waiters. The following rules are **load-bearing**:
///
/// 1.  **Slab must outlive all Handles.** Dropping a Slab while Handles exist
///     is **undefined behavior**. Debug builds catch this via `debug_assert`.
///     This is an intentional design choice: adding `Arc` or lifetime parameters
///     would defeat the zero-overhead goal.
///
/// # Memory Footprint
///
/// Each [`Handle`] is roughly 24 bytes (16 for header + 8 for Slab ref).
/// This is a deliberate tradeoff: we prioritize ergonomic `handle.get()` and `handle.delete()`
/// over absolute memory minimalism for small types.
///
/// # Single-threaded only
///
/// 2.  **Handles must not be moved in memory** except via [`Handle::relocate_to`].
///     In particular, **never** call `MaybeUninit::assume_init()` on a handle
///     storage — this moves the handle and invalidates the slot back-pointer.
///     Always use `assume_init_ref()` or `assume_init_mut()` instead.
///
/// 3.  **Single-threaded only.** This type is `!Send` and `!Sync`.
///
/// > [!IMPORTANT]
/// > **Handle drop mutates the container.** Be mindful of this when storing Handles
/// > in other collections that might outlive or concurrently access the Slab.
///
/// # Index Stability
///
/// Because Handles store indices (not pointers to data), the Slab's internal
/// `Vec` can safely reallocate (grow) without breaking any Handles.
///
/// # Zero-Fill Contract
///
/// Freshly committed pages are guaranteed zero-filled by the OS
/// (`mmap` `MAP_ANONYMOUS` / `VirtualAlloc`). The allocator relies on
/// `handle_header == null` for never-used slots. If the backing allocator
/// ever changes (hugepages, page recycling, custom allocator), this invariant
/// MUST be preserved — debug builds assert it on every `insert()`.
const DEAD: usize = !0;

pub struct Slab<T> {
    stack: UnsafeCell<VirtualPageStack>,
    len: Cell<usize>,
    cap: Cell<usize>,

    /// Tracks mutations (insertions/deletions). Captured by iterators to
    /// detect invalid concurrent mutations in debug builds.
    epoch: Cell<u64>,

    /// Track live handles for leak detection (heuristic, not a correctness invariant).
    ///
    /// This counter can diverge from `len` under edge cases such as relocate,
    /// double-drop, panic during drop, or manual pointer manipulation.
    /// It is checked only in debug builds and should not be relied upon for
    /// correctness — treat it as a best-effort diagnostic.
    ///
    /// **Intentional behavior with `mem::forget`:** If the user calls
    /// `mem::forget(handle)`, `handle_count` remains incremented (the handle
    /// was never dropped), causing the leak detection assert in `Slab::drop`
    /// to fire. This is by design — forgetting a Handle leaks the slot and
    /// is always a bug in correct usage.
    ///
    /// NOTE: We use `Cell<usize>` rather than `AtomicUsize` because `Slab` is
    /// `!Send + !Sync`. This eliminates unnecessary hardware synchronization
    /// overhead for what is primarily a debugging tool.
    handle_count: Cell<usize>,

    _marker: PhantomData<T>,
    _not_send_sync: PhantomData<*mut ()>, // !Send + !Sync
}

/// An owning handle to a value stored in a [`Slab`].
///
/// # Move Safety (`!Unpin`)
///
/// `Handle` is `!Unpin` (enforced by [`PhantomPinned`]). The Slab stores a raw
/// pointer back to this Handle's header, so the Handle's **memory address is
/// load-bearing**. The following rules apply:
///
/// | Operation                                   | Safe? |
/// |---------------------------------------------|-------|
/// | Moving `Pin<Box<Handle>>`                   | ✅ OK — heap pointer stays stable |
/// | Moving `Handle` itself (stack rebind)        | ❌ UB — breaks slot back-pointer |
/// | Moving `MaybeUninit<Handle>` after init      | ❌ UB — same as moving Handle |
/// | `mem::swap` / `mem::replace` on `&mut Handle`| ❌ UB — moves pinned data |
/// | [`Handle::relocate_to`]                      | ✅ OK — updates back-pointer |
///
/// Using `insert_handle()` (which returns `Pin<Box<Handle>>`) is the safest
/// ergonomic path. The `insert()` + `MaybeUninit` path requires careful
/// attention to never call `assume_init()` — only `assume_init_ref()` /
/// `assume_init_mut()`.
#[repr(C)]
pub struct Handle<'a, T> {
    header: HandleHeader<T>,
    slab: &'a Slab<T>,
    /// Enforces `!Unpin` on stable Rust, preventing safe code from moving
    /// this Handle out of a `Pin`. See the Move Safety table above.
    _marker: PhantomPinned,
}

/// Stable storage for a [`Handle`].
///
/// This is a type alias for `MaybeUninit<Handle>` that makes the API more
/// ergonomic. Use with [`Slab::insert_into`] to avoid exposing `MaybeUninit`
/// directly.
///
/// # Example
///
/// ```
/// use openkache::allocators::compacting_slab_allocator::{Handle, HandleStorage, Slab};
///
/// let slab = Slab::new();
/// let mut storage = HandleStorage::uninit();
/// let handle_pin = slab.insert_into(42, &mut storage);
/// assert_eq!(**handle_pin, 42);
/// // SAFETY: Handle was initialized by insert_into; must be dropped before Slab.
/// unsafe { std::ptr::drop_in_place(storage.as_mut_ptr().cast::<Handle<'_, i32>>()); }
/// ```
pub type HandleStorage<'a, T> = MaybeUninit<Handle<'a, T>>;

// Static assertion to ensure HandleHeader is at the start of Handle.
const _: () = assert!(core::mem::offset_of!(Handle<()>, header) == 0);

// Layout guard to ensure HandleHeader fits correctly within Handle.
const _: () = {
    assert!(std::mem::size_of::<HandleHeader<()>>() <= std::mem::size_of::<Handle<()>>());
};

// Guard against accidentally adding Drop-bearing fields to Handle.
// Relocation assumes trivial bitwise move semantics for all *fields*.
// Handle itself intentionally implements Drop (for auto-cleanup); relocation
// suppresses it via ManuallyDrop.
const _: () = {
    assert!(!std::mem::needs_drop::<HandleHeader<()>>());
    // Soundness guard: relocation assumes HandleHeader is pure data (no padding).
    assert!(std::mem::size_of::<HandleHeader<()>>() == std::mem::size_of::<Cell<usize>>());
    // Layout guard: ensures Slot packing is predictable and aligned.
    assert!(std::mem::size_of::<Slot<()>>().is_multiple_of(std::mem::align_of::<Slot<()>>()));
};

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Slab<T> {
    /// Creates a new empty Slab using the default page size.
    pub fn new() -> Self {
        let stack = VirtualPageStack::new(VirtualPageStack::get_default_page_size())
            .expect("Failed to initialize VirtualPageStack");
        Self {
            stack: UnsafeCell::new(stack),
            len: Cell::new(0),
            cap: Cell::new(0),
            epoch: Cell::new(0),
            handle_count: Cell::new(0),
            _marker: PhantomData,
            _not_send_sync: PhantomData,
        }
    }

    /// Creates a new Slab with at least the specified capacity in elements.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_with_capacity`](Self::try_with_capacity)
    /// for fallible construction.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::try_with_capacity(capacity).expect("Slab::with_capacity failed: out of memory")
    }

    /// Fallible version of [`with_capacity`](Self::with_capacity).
    pub fn try_with_capacity(capacity: usize) -> Result<Self, VirtualPageStackError> {
        let slab = Self::new();
        slab.try_reserve(capacity)?;
        Ok(slab)
    }

    /// Reserves capacity for at least `additional` more elements.
    ///
    /// Uses geometric growth to minimize commitment syscalls and TLB churn.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_reserve`](Self::try_reserve) for
    /// fallible allocation.
    pub fn reserve(&self, additional: usize) {
        self.try_reserve(additional)
            .expect("Slab::reserve failed: out of memory")
    }

    /// Fallible version of [`reserve`](Self::reserve).
    ///
    /// Uses geometric growth to minimize commitment syscalls and TLB churn.
    /// Returns an error instead of panicking when the OS cannot commit pages.
    pub fn try_reserve(&self, additional: usize) -> Result<(), VirtualPageStackError> {
        let current_len = self.len.get();
        let current_cap = self.cap.get();
        let required = current_len
            .checked_add(additional)
            .expect("Slab capacity overflow");

        if required > current_cap {
            unsafe {
                let stack = &mut *self.stack.get();
                let page_size = stack.page_size();
                let slot_size = std::mem::size_of::<Slot<T>>();
                let slots_per_page = page_size / slot_size;
                assert!(slots_per_page > 0, "Slot size exceeds page size");

                // Geometric growth: at least double current capacity.
                // If capacity is 0, we grow to at least one full page.
                let target_slots = if current_cap == 0 {
                    required.max(slots_per_page)
                } else {
                    required.max(current_cap.saturating_mul(2))
                };
                let needed_pages = target_slots.div_ceil(slots_per_page);
                let new_cap = needed_pages
                    .checked_mul(slots_per_page)
                    .expect("capacity overflow");

                stack.set_committed_pages(needed_pages)?;

                // Update capacity.
                self.cap.set(new_cap);

                // Zero-fill contract:
                // Freshly committed pages from VirtualPageStack are guaranteed
                // zero-filled (see VirtualPageStack::set_committed_pages docs).
                // We rely on this for:
                //   1. Initial handle_header == null in fresh slots (interpreted as Fresh).
            }
        }
        Ok(())
    }

    /// Shrinks the capacity of the slab as much as possible.
    ///
    /// It will drop any unused capacity from the virtual memory stack, while
    /// ensuring that all live elements remain within committed memory.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_shrink_to_fit`](Self::try_shrink_to_fit)
    /// for fallible shrinking.
    pub fn shrink_to_fit(&self) {
        self.try_shrink_to_fit()
            .expect("Slab::shrink_to_fit failed: out of memory")
    }

    /// Fallible version of [`shrink_to_fit`](Self::shrink_to_fit).
    pub fn try_shrink_to_fit(&self) -> Result<(), VirtualPageStackError> {
        let current_len = self.len.get();
        unsafe {
            let stack = &mut *self.stack.get();
            let page_size = stack.page_size();
            let slot_size = std::mem::size_of::<Slot<T>>();
            let slots_per_page = page_size / slot_size;
            assert!(slots_per_page > 0, "Slot size exceeds page size");

            // Minimum needed pages to hold current_len slots.
            let needed_pages = current_len.div_ceil(slots_per_page);
            let new_cap = needed_pages
                .checked_mul(slots_per_page)
                .expect("capacity overflow");

            // Only shrink if it actually reduces capacity.
            if new_cap < self.cap.get() {
                self.epoch.set(self.epoch.get().wrapping_add(1));
                stack.set_committed_pages(needed_pages)?;
                self.cap.set(new_cap);
            }
        }
        Ok(())
    }

    /// Returns the number of elements currently stored.
    pub fn len(&self) -> usize {
        self.len.get()
    }

    /// Returns `true` if the slab contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the given handle was created by this slab.
    pub fn contains(&self, handle: &Handle<T>) -> bool {
        ptr::eq(handle.slab, self)
    }

    #[inline(always)]
    pub(crate) unsafe fn slots_ptr(&self) -> *mut Slot<T> {
        if self.cap.get() == 0 {
            // Return a dangling pointer to satisfy pointer arithmetic requirements.
            // This pointer is never dereferenced when cap == 0.
            return NonNull::<Slot<T>>::dangling().as_ptr();
        }
        unsafe { (*self.stack.get()).page_ptr_unchecked(0) as *mut Slot<T> }
    }

    /// Inserts an element and initializes a Handle at `dest`.
    ///
    /// # Safety Contract
    ///
    /// `dest` **must** point to memory that remains at a fixed address for the
    /// Handle's entire lifetime. After this call, use only `assume_init_ref()`
    /// — never `assume_init()`.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_insert`](Self::try_insert) for
    /// fallible insertion.
    pub fn insert<'a, 'b>(
        &'a self,
        value: T,
        dest: &'b mut MaybeUninit<Handle<'a, T>>,
    ) -> Pin<&'b mut Handle<'a, T>> {
        self.try_insert(value, dest)
            .expect("Slab::insert failed: out of memory")
    }

    /// Fallible version of [`insert`](Self::insert).
    ///
    /// Returns an allocation error instead of panicking when the OS cannot
    /// commit pages for growth. On error, `value` is dropped and `dest`
    /// remains uninitialized.
    pub fn try_insert<'a, 'b>(
        &'a self,
        value: T,
        dest: &'b mut MaybeUninit<Handle<'a, T>>,
    ) -> Result<Pin<&'b mut Handle<'a, T>>, VirtualPageStackError> {
        let mut len = self.len.get();
        if len >= self.cap.get() {
            self.try_reserve(1)?;
            len = self.len.get(); // Refresh len after reserve
        }
        // STATE TRANSITION: Fresh → Live
        // Slot at `len` is either zero-filled (first use) or a recycled Vacated slot.
        // In both cases, we overwrite all fields to produce a fully Live slot.
        self.len.set(len + 1);
        self.epoch.set(self.epoch.get().wrapping_add(1));

        unsafe {
            let slots = self.slots_ptr();

            let slot_ptr = slots.add(len);

            #[cfg(debug_assertions)]
            self.debug_assert_zero_fill(slot_ptr, len);

            let handle = Handle {
                header: HandleHeader {
                    index: Cell::new(len),
                    _marker: PhantomData,
                },
                slab: self,
                _marker: PhantomPinned,
            };

            self.handle_count.set(self.handle_count.get() + 1);
            ptr::write(dest.as_mut_ptr(), handle);
            let handle_ptr = dest.as_mut_ptr() as *const Handle<'a, T>;

            ptr::write(
                slot_ptr,
                Slot {
                    value: ManuallyDrop::new(value),
                    handle_header: NonNull::new_unchecked(
                        &((*handle_ptr).header) as *const _ as *mut _,
                    ),
                },
            );

            self.debug_check_invariant();

            Ok(Pin::new_unchecked(dest.assume_init_mut()))
        }
    }

    /// Ergonomic version of insert that heap-allocates the Handle.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_insert_handle`](Self::try_insert_handle)
    /// for fallible insertion.
    pub fn insert_handle(&self, value: T) -> Pin<Box<Handle<'_, T>>> {
        self.try_insert_handle(value)
            .expect("Slab::insert_handle failed: out of memory")
    }

    /// Fallible version of [`insert_handle`](Self::insert_handle).
    pub fn try_insert_handle(
        &self,
        value: T,
    ) -> Result<Pin<Box<Handle<'_, T>>>, VirtualPageStackError> {
        let h = Box::pin(MaybeUninit::uninit());
        unsafe {
            let raw = Box::into_raw(Pin::into_inner_unchecked(h));
            match self.try_insert(value, &mut *raw) {
                Ok(_pin) => Ok(Pin::new_unchecked(Box::from_raw(raw as *mut Handle<'_, T>))),
                Err(e) => {
                    let _ = Box::from_raw(raw);
                    Err(e)
                }
            }
        }
    }

    /// Inserts an element using a [`HandleStorage`] destination.
    ///
    /// This is an ergonomic wrapper around [`insert`](Self::insert) that hides
    /// the `MaybeUninit` details. The storage must remain at a stable address
    /// for the Handle's lifetime.
    ///
    /// # Panics
    ///
    /// Panics on memory allocation failure. Use [`try_insert_into`](Self::try_insert_into)
    /// for fallible insertion.
    ///
    /// # Example
    ///
    /// ```
    /// use openkache::allocators::compacting_slab_allocator::{Handle, HandleStorage, Slab};
    ///
    /// let slab = Slab::new();
    /// let mut storage = HandleStorage::uninit();
    /// let handle_pin = slab.insert_into(42, &mut storage);
    /// assert_eq!(**handle_pin, 42);
    /// // SAFETY: Handle was initialized by insert_into; must be dropped before Slab.
    /// unsafe { std::ptr::drop_in_place(storage.as_mut_ptr().cast::<Handle<'_, i32>>()); }
    /// ```
    pub fn insert_into<'a, 'b>(
        &'a self,
        value: T,
        dest: &'b mut HandleStorage<'a, T>,
    ) -> Pin<&'b mut Handle<'a, T>> {
        self.insert(value, dest)
    }

    /// Fallible version of [`insert_into`](Self::insert_into).
    pub fn try_insert_into<'a, 'b>(
        &'a self,
        value: T,
        dest: &'b mut HandleStorage<'a, T>,
    ) -> Result<Pin<&'b mut Handle<'a, T>>, VirtualPageStackError> {
        self.try_insert(value, dest)
    }

    /// Retains only the elements specified by the predicate.
    ///
    /// Elements for which `f` returns false are deleted.
    ///
    /// # Ephemeral Semantics
    ///
    /// The predicate `f` receives a shared reference to each value. This
    /// reference is an **ephemeral snapshot view**. It **must not** be stored
    /// or leaked beyond the call. Importantly, the reference may be invalidated
    /// **even if the predicate returns true** (due to later compaction moving
    /// the slot).
    pub fn retain<F>(&self, mut f: F)
    where
        F: FnMut(&T) -> bool,
    {
        let mut i = 0;
        while i < self.len() {
            // Scope the reference to signal that it must not outlive the
            // predicate call. The borrow ends before delete touches the slot.
            let keep = unsafe {
                let slots = self.slots_ptr();
                let v = &*(*slots.add(i)).value;
                f(v)
            };

            if !keep {
                // Delete at i. This will move the last element to i.
                // So we do NOT increment i.
                unsafe {
                    self.delete(i);
                }
            } else {
                i += 1;
            }
        }
    }

    #[inline(always)]
    pub(crate) unsafe fn delete(&self, index: usize) {
        unsafe {
            let slots = self.slots_ptr();
            let len = self.len.get();

            if len == 0 {
                #[cfg(debug_assertions)]
                panic!("delete called on empty Slab — this is a bug in Handle logic");
                #[cfg(not(debug_assertions))]
                return;
            }

            // Lock in the new length before we start moving/dropping data.
            let last_index = len - 1;
            self.len.set(last_index);
            self.epoch.set(self.epoch.get().wrapping_add(1));
            self.handle_count.set(self.handle_count.get() - 1);

            let target_slot = slots.add(last_index);

            let mut dead_slot = if index < last_index {
                // STATE TRANSITIONS (swap-pop):
                //   dst slot (deleted value): Live → Vacated (value dropped, handle tombstoned)
                //   src slot (last element):  Live → Moving → Live (relocated to dst's index)
                let dst = slots.add(index);

                #[cfg(debug_assertions)]
                {
                    let moved_index = (*target_slot).handle_header.as_ref().index.get();
                    debug_assert_eq!(
                        moved_index, last_index,
                        "Invariant violated: moved element at index {} thought it was at {}",
                        last_index, moved_index
                    );
                }

                // 2. Read values from slots.
                let moved_slot = ptr::read(target_slot);
                let dead_slot = ptr::read(dst);

                // 3. Patch handle of moved element BEFORE publishing slot.
                // This makes state consistent even if write faults.
                moved_slot.handle_header.as_ref().index.set(index);

                // 4. Write moved slot to its new home.
                ptr::write(dst, moved_slot);

                #[cfg(debug_assertions)]
                ptr::write_bytes(target_slot, 0xDD, 1);

                dead_slot
            } else {
                // STATE TRANSITION (tail-pop): Live → Vacated
                // No swap needed — the deleted slot IS the last element.
                ptr::read(target_slot)
            };

            // 5. Poison handle pointer on vacated source slot BEFORE drop.
            // This ensures stale detection works for this slot even if it's not immediately reused.
            #[cfg(debug_assertions)]
            ptr::write(
                ptr::addr_of_mut!((*target_slot).handle_header),
                NonNull::dangling(),
            );

            // 6. Tombstone handle BEFORE drop, then drop the dead value.
            //
            // PANIC SAFETY: If `T::drop` panics here, the invariant is:
            //   - handle already tombstoned (index == DEAD)
            //   - len already decremented (step above)
            //   - slot is reusable on next insert
            // The handle can never appear alive after a drop panic.
            dead_slot.handle_header.as_ref().index.set(DEAD);
            ManuallyDrop::drop(&mut dead_slot.value);

            // 7. Debug poison: catch accidental reuse of dead handle header.
            #[cfg(debug_assertions)]
            {
                dead_slot.handle_header = NonNull::dangling();
            }
            let _ = dead_slot; // suppress unused_assignments in release

            self.debug_check_invariant();
        }
    }

    /// Debug-only structural invariant checker.
    ///
    /// Verifies that `slots[i].handle.index == i` for all live slots, and
    /// (for small slabs) that no two slots point to the same handle.
    #[inline]
    fn debug_check_invariant(&self) {
        #[cfg(debug_assertions)]
        unsafe {
            // Optimization: Skip expensive check for very large slabs in debug tests.
            // We use probabilistic checking (1/16 frequency) for large slabs using
            // the mutation epoch as a stable source of entropy.
            let len = self.len();
            if len > 256 && (self.epoch.get() & 0xF) != 0 {
                return;
            }

            let slots = self.slots_ptr();
            for i in 0..len {
                let slot = &*slots.add(i);
                let header = slot.handle_header.as_ref();
                let handle_index = header.index.get();

                debug_assert_eq!(
                    handle_index, i,
                    "Invariant violated: slot[{}].handle_header.get() == {} (expected {})",
                    i, handle_index, i,
                );

                let handle_ptr = header as *const HandleHeader<T> as *const Handle<T>;

                debug_assert!(
                    ptr::eq((*handle_ptr).slab, self),
                    "Invariant violated: slot[{}]'s handle does not belong to this Slab",
                    i
                );
                debug_assert!(
                    ptr::eq(
                        slot.handle_header.as_ptr(),
                        &(*handle_ptr).header as *const _ as *mut _
                    ),
                    "Invariant violated: slot[{}] header pointer mismatch",
                    i
                );
            }

            // Handle pointer uniqueness check: detects double-slot pointing to
            // the same handle. O(n²) so only run for small slabs.
            if len <= 128 {
                for i in 0..len {
                    let ptr_i = (*slots.add(i)).handle_header.as_ptr();
                    for j in (i + 1)..len {
                        let ptr_j = (*slots.add(j)).handle_header.as_ptr();
                        debug_assert!(
                            !ptr::eq(ptr_i, ptr_j),
                            "Invariant violated: slot[{}] and slot[{}] point to the same handle at {:?}",
                            i,
                            j,
                            ptr_i
                        );
                    }
                }
            }
        }
    }

    /// Zero-fill contract: freshly committed pages must be zero-filled.
    /// If the backing allocator ever violates this (hugepages, page
    /// recycling, custom allocator), this debug_assert catches it.
    /// In release builds this is a no-op.
    ///
    /// NOTE: We check raw bytes of the value region rather than the
    /// handle_header field, because reading a zero-filled NonNull as
    /// null pointer is technically UB. The handle_header being null is
    /// the primary zero-fill indicator; this full scan provides maximum
    /// confidence that the contract holds.
    #[cfg(debug_assertions)]
    unsafe fn debug_assert_zero_fill(&self, slot_ptr: *const Slot<T>, len: usize) {
        unsafe {
            let handle_header_ptr = ptr::addr_of!((*slot_ptr).handle_header);
            // In release, we rely on OS zero-fill. In debug, we check that
            // the handle_header is null (initial state).
            let is_fresh = ptr::read_volatile(handle_header_ptr as *const *const ()).is_null();

            if is_fresh && std::mem::size_of::<T>() > 0 {
                let value_ptr = ptr::addr_of!((*slot_ptr).value) as *const u8;
                let value_size = std::mem::size_of::<T>();
                // Full scan of the value region using read_volatile to prevent
                // the optimizer from eliding reads of "uninitialized" memory.
                let mut non_zero_offset: Option<usize> = None;
                for i in 0..value_size {
                    if ptr::read_volatile(value_ptr.add(i)) != 0 {
                        non_zero_offset = Some(i);
                        break;
                    }
                }
                debug_assert!(
                    non_zero_offset.is_none(),
                    "Zero-fill contract violated: slot at index {} has non-zero \
                     byte at offset {} on first use. \
                     The backing allocator may not be zero-filling pages.",
                    len,
                    non_zero_offset.unwrap_or(0),
                );
            }
        }
    }

    /// Iterates over the values currently stored in the slab.
    ///
    /// # Stability Semantics
    ///
    /// The iterator captures a snapshot of `len` at creation time and only
    /// visits indices `0..initial_len`. This means:
    ///
    /// - **Epoch guard:** Both debug and release builds capture the epoch and
    ///   panic if any mutation occurs during iteration.
    /// - **No ordering guarantee under mutation:** If the slab is mutated
    ///   mid-iteration and the guard triggers, iteration panics immediately.
    ///   Under no circumstance does iteration return a dangling reference.
    ///
    /// **TL;DR:** Do not mutate during iteration. Epoch guard enforces this
    /// in all build profiles.
    pub fn iter(&self) -> SlabIter<'_, T> {
        let (slots, len, epoch) = unsafe { (self.slots_ptr(), self.len(), self.epoch.get()) };
        SlabIter {
            ptr: slots,
            remaining: len,
            epoch,
            slab: self,
            _marker: PhantomData,
        }
    }

    /// Iterates over the live handles currently stored in the slab.
    ///
    /// Each item is a `&Handle<T>` that can be used to inspect the handle's
    /// index, slab, and value. The same epoch guard applies: mutating the
    /// slab during iteration causes a panic.
    pub fn handles(&self) -> HandleIter<'_, T> {
        let (slots, len, epoch) = unsafe { (self.slots_ptr(), self.len(), self.epoch.get()) };
        HandleIter {
            ptr: slots,
            remaining: len,
            epoch,
            slab: self,
            _marker: PhantomData,
        }
    }
}

/// Custom iterator over Slab values.
///
/// Uses raw pointer arithmetic for vectorizable loops.
/// Provides ExactSizeIterator and FusedIterator.
pub struct SlabIter<'a, T> {
    ptr: *mut Slot<T>,
    remaining: usize,
    epoch: u64,
    slab: &'a Slab<T>,
    _marker: PhantomData<&'a T>,
}

impl<'a, T> Iterator for SlabIter<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        assert_eq!(
            self.epoch,
            self.slab.epoch.get(),
            "Slab mutated during iteration — iterator invalidated!"
        );
        debug_assert!(
            self.remaining <= self.slab.len(),
            "Iterator corruption: remaining ({}) > slab.len() ({})",
            self.remaining,
            self.slab.len()
        );

        let value = unsafe { &*(*self.ptr).value };
        self.ptr = unsafe { self.ptr.add(1) };
        self.remaining -= 1;
        Some(value)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for SlabIter<'_, T> {}
impl<T> std::iter::FusedIterator for SlabIter<'_, T> {}

/// Custom iterator over Handles in a Slab.
///
/// Each item is a `&Handle<T>` providing access to both the index and the value.
/// The same epoch guard and iterator-invalidation semantics apply as [`SlabIter`].
pub struct HandleIter<'a, T> {
    ptr: *mut Slot<T>,
    remaining: usize,
    epoch: u64,
    slab: &'a Slab<T>,
    _marker: PhantomData<&'a Handle<'a, T>>,
}

impl<'a, T> Iterator for HandleIter<'a, T> {
    type Item = &'a Handle<'a, T>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        assert_eq!(
            self.epoch,
            self.slab.epoch.get(),
            "Slab mutated during Handle iteration — iterator invalidated!"
        );
        debug_assert!(
            self.remaining <= self.slab.len(),
            "Handle iterator corruption: remaining ({}) > slab.len() ({})",
            self.remaining,
            self.slab.len()
        );

        let handle = unsafe {
            let slot = &*self.ptr;
            // HandleHeader is at offset 0 of Handle (enforced by static assertion),
            // so we can safely cast from handle_header pointer to full Handle.
            &*(slot.handle_header.as_ptr() as *const Handle<'a, T>)
        };
        self.ptr = unsafe { self.ptr.add(1) };
        self.remaining -= 1;
        Some(handle)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for HandleIter<'_, T> {}
impl<T> std::iter::FusedIterator for HandleIter<'_, T> {}

/// # Panic Safety (Slab::drop)
///
/// If `T::drop` panics during `Slab::drop`, some handles may remain live
/// and point to freed (decommitted) slab memory. This is acceptable because
/// panic during drop is an abort-like scenario — Rust does not guarantee
/// cleanup ordering after a panic unwind through destructors.
///
/// The ordering is intentional:
///   1. Tombstone all handles (debug only) — prevents use-after-free reads
///   2. Drop all values — the dangerous step
///   3. Assert no leaks (debug only)
///
/// If step 2 panics partway through, remaining values are leaked (not
/// double-freed) because `ManuallyDrop` suppresses implicit drops.
impl<T> Drop for Slab<T> {
    fn drop(&mut self) {
        let len = self.len.get();
        let handle_count = self.handle_count.get();

        #[cfg(debug_assertions)]
        if !std::thread::panicking() {
            debug_assert_eq!(
                handle_count, len,
                "Invariant violated: handle_count ({}) != len ({}) at drop! \
                 Possible causes: double-drop, lost handle, or manual tombstone corruption.",
                handle_count, len
            );
        }

        // STATE TRANSITION: All Live slots → Vacated (handles tombstoned, values dropped).
        // After this, all slots beyond index 0 are logically Fresh/Vacated but the
        // backing memory is about to be decommitted by VirtualPageStack::drop.
        if len > 0 || handle_count > 0 {
            #[cfg(debug_assertions)]
            {
                // Fail-fast logic: Invalidate all live handles by setting their
                // tracked index to DEAD. This turns use-after-free UB
                // into a predictable panic if debug assertions are enabled.
                unsafe {
                    let slots = self.slots_ptr();
                    for i in 0..len {
                        let slot = &*slots.add(i);
                        slot.handle_header.as_ref().index.set(DEAD);
                    }
                }
            }

            // Drop all values remaining in the slab.
            unsafe {
                let slots = self.slots_ptr();
                for i in 0..len {
                    ManuallyDrop::drop(&mut (*slots.add(i)).value);
                }
            }

            #[cfg(debug_assertions)]
            {
                if handle_count > 0 {
                    debug_assert!(
                        false,
                        "Slab dropped with {} live handle(s) — this is undefined behavior! (slab len was {})",
                        handle_count, len
                    );
                } else {
                    debug_assert!(
                        false,
                        "Slab dropped with {} live element(s) — this is undefined behavior!",
                        len
                    );
                }
            }
        }
    }
}

impl<'a, T> Deref for Handle<'a, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        unsafe { &(*self.slot_ptr()).value }
    }
}

impl<'a, T> DerefMut for Handle<'a, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

impl<'a, T> Handle<'a, T> {
    /// Helper to resolve the slot pointer and check invariants.
    #[inline(always)]
    unsafe fn slot_ptr(&self) -> *mut Slot<T> {
        let index = self.header.index.get();
        #[cfg(debug_assertions)]
        assert!(index != DEAD, "Attempted to dereference a DEAD handle!");

        unsafe {
            let slot_ptr = self.slab.slots_ptr().add(index);

            #[cfg(debug_assertions)]
            {
                let slot = &*slot_ptr;
                assert!(
                    ptr::eq(
                        slot.handle_header.as_ptr(),
                        &self.header as *const _ as *mut _
                    ),
                    "Stale handle access detected! (index={})",
                    index
                );
            }

            slot_ptr
        }
    }

    /// Returns the current index of this handle's data within the Slab.
    pub fn index(&self) -> usize {
        self.header.index.get()
    }

    /// Returns a reference to the Slab that owns this handle.
    pub fn slab(&self) -> &Slab<T> {
        self.slab
    }

    /// Explicitly destroys the handle and its associated data in the Slab.
    pub fn delete(self) {
        self.destroy();
    }

    /// Returns a reference to the stored value.
    #[inline(always)]
    pub fn get(&self) -> &T {
        self.deref()
    }

    /// Returns a mutable reference to the stored value.
    #[inline(always)]
    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut (*self.slot_ptr()).value }
    }

    /// Relocates the handle to a new stable memory location.
    ///
    /// Bitwise copies the handle, updates the slot's back-pointer, and forgets
    /// the original so that `Drop` does not fire.
    ///
    /// # Safety
    ///
    /// `dest` must not alias `self`. Relocating a handle to its own address is
    /// undefined behavior (`ptr::copy_nonoverlapping` requires non-overlapping
    /// source and destination).
    pub fn relocate_to(self: Pin<&mut Self>, dest: *mut MaybeUninit<Handle<'a, T>>) {
        // NOTE: This does NOT change the slot's state — it remains Live.
        // Only the handle_header back-pointer is patched to the new Handle
        // location. The old Handle is tombstoned (index = DEAD) to prevent
        // its Drop from firing a spurious delete.
        unsafe {
            let handle_ref = self.get_unchecked_mut();
            let index = handle_ref.header.index.get();
            if index == DEAD {
                return; // Already relocated or deleted
            }

            // Guard: source and destination must not overlap (byte-range check).
            {
                let src_start = handle_ref as *mut _ as usize;
                let src_end = src_start + std::mem::size_of::<Handle<'a, T>>();
                let dst_start = dest as usize;
                let dst_end = dst_start + std::mem::size_of::<Handle<'a, T>>();
                debug_assert!(
                    !(src_start < dst_end && dst_start < src_end),
                    "relocate_to called with overlapping source and destination — this is UB"
                );
            }

            let slots = handle_ref.slab.slots_ptr();

            // 1. Bitwise copy directly from source to destination.
            //    No intermediate ptr::read needed — we copy in place and
            //    tombstone the source afterwards. If copy faults, source
            //    is still authoritative and valid.
            ptr::copy_nonoverlapping(
                handle_ref as *const Handle<'a, T>,
                dest as *mut Handle<'a, T>,
                1,
            );

            let new_handle_ptr = dest as *mut Handle<'a, T>;
            let slot_ptr = slots.add(index);

            // 2. Patch the slab's slot back-pointer to point to the NEW handle header.
            //    From this point on, the system recognizes the new location as authoritative.
            (*slot_ptr).handle_header =
                NonNull::new_unchecked(&(*new_handle_ptr).header as *const _ as *mut _);

            // 3. Tombstone the SOURCE handle so its Drop is a no-op.
            handle_ref.header.index.set(DEAD);
        }
    }

    /// Explicitly destroys the handle and its associated data in the Slab.
    pub fn destroy(self) {
        // Drop implementation will call slab.delete()
        drop(self);
    }

    /// Returns true if the handle is still valid and not stale.
    ///
    /// # Panic Safety
    ///
    /// The handle is tombstoned (index set to `DEAD`) **before** the value is
    /// dropped. If `T::drop` panics, the handle will correctly report as dead.
    /// This is a strictly better invariant than tombstoning after drop.
    #[inline]
    pub fn is_alive(&self) -> bool {
        let index = self.header.index.get();
        if index == DEAD {
            return false;
        }

        let len = self.slab.len.get();
        if index >= len {
            return false;
        }

        unsafe {
            // CAPACITY INVARIANT: index < len <= cap <= committed_slots.
            // If the Slab ever gains support for shrinking (releasing pages),
            // the shrink operation MUST ensure all live slots (0..len) remain
            // within committed memory. Violating this causes silent UB here.
            debug_assert!(
                index < self.slab.len.get(),
                "is_alive index out of live bounds!"
            );

            let slots = self.slab.slots_ptr();
            let slot = &*slots.add(index);

            // Perfect stale detection: structural identity.
            ptr::eq(
                slot.handle_header.as_ptr(),
                &self.header as *const _ as *mut _,
            )
        }
    }
}

impl<'a, T> Drop for Handle<'a, T> {
    fn drop(&mut self) {
        let index = self.header.index.replace(DEAD);
        if index != DEAD {
            unsafe {
                self.slab.delete(index);
            }
        }
    }
}
