use super::memory::{self, MemoryFlags, PageSizeInfo};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::OnceLock;
use thiserror::Error;

/// Custom error type for VirtualPageStack operations.
#[derive(Debug, Error)]
pub enum VirtualPageStackError {
    #[error("Capacity calculation overflowed usize")]
    CapacityOverflow,
    #[error("Address offset overflowed usize")]
    OffsetOverflow,
    #[error("Address size overflowed usize")]
    SizeOverflow,
    #[error("Committed page count {0} exceeds capacity {1}")]
    CommitExceedsCapacity(usize, usize),
    #[error("Access index {0} is out of bounds (committed: {1})")]
    IndexOutOfBounds(usize, usize),
    #[error("Memory reservation failed for {size} bytes: {source}")]
    ReservationFailed { size: usize, source: std::io::Error },
    #[error("Memory commitment failed: {0}")]
    CommitFailed(#[from] super::memory::MemoryError),
    #[error("Memory decommitment failed: {0}")]
    DecommitFailed(super::memory::MemoryError),
    #[error("Guard page protection failed: {0}")]
    GuardProtectFailed(super::memory::MemoryError),
    #[error("No supported page size found <= {0} bytes")]
    NoSupportedPageSize(usize),
    #[error("Shrink delta {delta} exceeds committed page count {committed}")]
    ShrinkUnderflow { delta: usize, committed: usize },
}

/// A managed stack of virtual memory pages with a contiguous commitment model.
///
/// It reserves a large chunk of virtual address space upfront and allows
/// committing and decommitting physical memory pages on-demand in a contiguous
/// range starting from index 0.
///
/// # Invariants
/// - **Commit Granularity**: Any commitment or decommitment operation MUST align
///   exactly with the `page_size`. No partial page operations are allowed.
/// - **NUMA Policy**: The NUMA affinity (if specified) is frozen at construction
///   and applies to all subsequent commitment operations.
///
/// This implementation is strictly single-threaded.
pub struct VirtualPageStack {
    base: NonNull<u8>,
    max_pages: usize,
    page_size: usize,
    reserved_size: usize,
    flags: MemoryFlags,
    /// The number of currently committed pages (0..N range).
    committed_count: usize,
    /// Marker to enforce !Sync. This implementation is strictly single-threaded
    /// and relies on &mut self for exclusive access to the virtual range.
    _marker: PhantomData<*const u8>,
}

impl VirtualPageStack {
    /// Creates a new VirtualPageStack by reserving virtual address space.
    ///
    /// The `PageSizeInfo` determines the page size and any necessary OS-specific flags.
    /// Capacity is automatically aligned with the host's physical RAM.
    /// No physical memory is committed at this point.
    ///
    /// # Invariants
    /// - `PageSizeInfo.size` defines the mandatory commitment granularity.
    /// - Reserved `base` address is guaranteed to be aligned to `page_size`.
    pub fn new(info: PageSizeInfo) -> Result<Self, VirtualPageStackError> {
        // We require it to be positive to avoid ZST division-by-zero or pointer arithmetic bugs.
        assert!(info.size > 0, "Page size must be positive");

        let total_ram = memory::get_total_physical_memory();
        let max_pages = total_ram / info.size;

        // Invariant: Reserve one extra page as a guard page (never committed).
        // This provides a hardware-level trap for out-of-bounds access.
        let reserved_pages = max_pages
            .checked_add(1)
            .ok_or(VirtualPageStackError::CapacityOverflow)?;

        let capacity = reserved_pages
            .checked_mul(info.size)
            .ok_or(VirtualPageStackError::CapacityOverflow)?;

        let base = unsafe {
            memory::reserve(capacity, info.flags).map_err(|e| {
                VirtualPageStackError::ReservationFailed {
                    size: capacity,
                    source: match e {
                        memory::MemoryError::ReserveFailed { source, .. } => source,
                        _ => std::io::Error::new(std::io::ErrorKind::Other, e.to_string()),
                    },
                }
            })?
        };

        // Invariant: Ensure the OS granted reservation is aligned to our page size.
        // This is critical for contiguous math and huge page backing. Some OS-level
        // calls can return page-aligned but not huge-page-aligned addresses if
        // the alignment hint is ignored or unsupported.
        if base.as_ptr() as usize % info.size != 0 {
            // Explicitly release and error out if the OS didn't respect alignment
            unsafe {
                let _ = memory::release(base.as_ptr(), capacity);
            }
            return Err(VirtualPageStackError::ReservationFailed {
                size: capacity,
                source: std::io::Error::new(
                    std::io::ErrorKind::AddrNotAvailable,
                    format!(
                        "OS failed to respect page alignment hint (requested {} bytes)",
                        info.size
                    ),
                ),
            });
        }

        // Invariant: The guard page is reserved at the end of the range [max_pages, max_pages + 1).
        // It is never touched by commit/decommit as committed_count <= max_pages.
        // We explicitly protect it here to ensure it is in a NOACCESS state,
        // providing a hardware-level trap regardless of platform-specific reserve behavior.
        unsafe {
            let guard_off = max_pages * info.size;
            let guard_addr = base.as_ptr().add(guard_off);
            memory::protect_noaccess(guard_addr, info.size)
                .map_err(VirtualPageStackError::GuardProtectFailed)?;
        }

        Ok(Self {
            base,
            max_pages,
            page_size: info.size,
            reserved_size: capacity,
            flags: info.flags,
            committed_count: 0,
            _marker: PhantomData,
        })
    }

    /// Returns the standard system page size info.
    ///
    /// The result is cached after the first successful discovery.
    pub fn get_default_page_size() -> PageSizeInfo {
        Self::try_default_page_size().expect("No standard page size discovered")
    }

    /// Fallible version of `get_default_page_size`.
    pub fn try_default_page_size() -> Result<PageSizeInfo, VirtualPageStackError> {
        static CACHE: OnceLock<PageSizeInfo> = OnceLock::new();
        // Thread-safe initialization using get_or_init for cleaner semantics.
        Ok(*CACHE.get_or_init(|| {
            Self::supported_page_sizes()
                .iter()
                .filter(|i| !i.flags.huge_pages)
                .min_by_key(|i| i.size)
                .copied()
                .expect("No standard page size discovered")
        }))
    }

    /// Returns the number of currently committed pages (0..N range).
    pub fn committed_pages(&self) -> usize {
        self.committed_count
    }

    /// Adjusts the number of committed physical pages.
    ///
    /// # Semantics
    /// - **Initialization**: Newly committed pages are considered UNINITIALIZED. While most OSs
    ///   provide zeroed memory for security, consumers MUST NOT rely on this behavior.
    /// - **Content Loss**: Decommitting a page results in immediate data loss. Re-committing
    ///   the same address range will return fresh, uninitialized memory.
    pub fn set_committed_pages(&mut self, new_count: usize) -> Result<(), VirtualPageStackError> {
        let current = self.committed_count;
        match new_count.cmp(&current) {
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Greater => self.grow(new_count - current),
            std::cmp::Ordering::Less => self.shrink(current - new_count),
        }
    }

    /// Monotonic growth: Commits `delta` additional pages.
    ///
    /// This is a hot path for contiguous allocation that avoids general branching.
    #[inline]
    pub fn grow(&mut self, delta: usize) -> Result<(), VirtualPageStackError> {
        if delta == 0 {
            return Ok(());
        }

        let current_count = self.committed_count;
        let new_count = current_count
            .checked_add(delta)
            .ok_or(VirtualPageStackError::CapacityOverflow)?;

        if new_count > self.max_pages {
            return self.report_exceeds_capacity(new_count);
        }

        let size = self.bytes_for_pages_unchecked(delta);
        let addr = self.ptr_at_unchecked(current_count);

        unsafe {
            memory::commit(addr, size, self.flags).map_err(VirtualPageStackError::CommitFailed)?;
        }

        self.committed_count = new_count;
        Ok(())
    }

    #[cold]
    fn report_exceeds_capacity(&self, new_count: usize) -> Result<(), VirtualPageStackError> {
        Err(VirtualPageStackError::CommitExceedsCapacity(
            new_count,
            self.max_pages,
        ))
    }

    /// Monotonic shrink: Decommits `delta` pages from the end.
    ///
    /// This is a fast path for contiguous deallocation.
    #[inline]
    pub fn shrink(&mut self, delta: usize) -> Result<(), VirtualPageStackError> {
        if delta == 0 {
            return Ok(());
        }

        let current_count = self.committed_count;
        if delta > current_count {
            return self.report_shrink_underflow(delta, current_count);
        }

        let new_count = current_count - delta;
        let size = self.bytes_for_pages_unchecked(delta);
        let addr = self.ptr_at_unchecked(new_count);

        unsafe {
            memory::decommit(addr, size).map_err(VirtualPageStackError::DecommitFailed)?;
        }

        self.committed_count = new_count;
        Ok(())
    }

    #[cold]
    fn report_shrink_underflow(
        &self,
        delta: usize,
        committed: usize,
    ) -> Result<(), VirtualPageStackError> {
        Err(VirtualPageStackError::ShrinkUnderflow { delta, committed })
    }

    /// Centralized offset calculation without bounds check.
    #[inline(always)]
    fn bytes_for_pages_unchecked(&self, pages: usize) -> usize {
        // Invariant check: The total size must fit within usize.
        // This is guaranteed at construction as (max_pages + 1) * page_size is verified.
        let off = pages * self.page_size;
        debug_assert_eq!(
            off / self.page_size,
            pages,
            "Pointer offset overflowed usize"
        );
        off
    }

    /// Centralized pointer resolution without bounds check.
    #[inline(always)]
    fn ptr_at_unchecked(&self, index: usize) -> *mut u8 {
        let off = self.bytes_for_pages_unchecked(index);
        unsafe { self.base.as_ptr().add(off) }
    }

    /// Centralized bounds check logic.
    #[inline]
    fn check_bounds(&self, index: usize) -> Result<(), VirtualPageStackError> {
        if index >= self.committed_count {
            return Err(VirtualPageStackError::IndexOutOfBounds(
                index,
                self.committed_count,
            ));
        }
        Ok(())
    }

    /// Returns a constant pointer to the start of the page at `index`.
    ///
    /// # Contract & Safety
    /// - **Validity**: The returned pointer remains valid until the next `shrink`, `set_committed_pages`, or `Drop`.
    /// - **Aliasing**: Multiple pointers to the same page are permitted.
    /// - **UB**: Reading from a pointer after the page has been decommitted or the stack dropped is undefined behavior.
    ///
    /// The caller must ensure that the index is within the committed range.
    #[inline]
    pub unsafe fn page_ptr(&self, index: usize) -> *const u8 {
        unsafe { self.page_ptr_unchecked(index) }
    }

    /// Returns a mutable pointer to the start of the page at `index`.
    ///
    /// # Contract & Safety
    /// - **Exclusivity**: Requires `&mut self` to ensure no other references (immutable or mutable)
    ///   exist to the stack during the acquisition of this mutable pointer.
    /// - **Aliasing**: Multiple pointers derived from this mutable reference are permitted,
    ///   but the user must ensure they do not violate Rust's aliasing rules when dereferencing.
    #[inline]
    pub unsafe fn page_ptr_mut(&mut self, index: usize) -> *mut u8 {
        unsafe { self.page_ptr_mut_unchecked(index) }
    }

    /// Checked version of `page_ptr`.
    #[inline]
    pub fn page_ptr_at(&self, index: usize) -> Result<*const u8, VirtualPageStackError> {
        self.check_bounds(index)?;
        Ok(self.ptr_at_unchecked(index) as *const u8)
    }

    /// Checked version of `page_ptr_mut`.
    #[inline]
    pub fn page_ptr_at_mut(&mut self, index: usize) -> Result<*mut u8, VirtualPageStackError> {
        self.check_bounds(index)?;
        Ok(self.ptr_at_unchecked(index))
    }

    /// Optimized version for hot paths; bounds check only in debug builds.
    #[inline(always)]
    pub unsafe fn page_ptr_unchecked(&self, index: usize) -> *const u8 {
        debug_assert!(
            index < self.committed_count,
            "Accessing uncommitted page at index {} (committed: {})",
            index,
            self.committed_count
        );

        self.ptr_at_unchecked(index) as *const u8
    }

    /// Optimized mutable version for hot paths; bounds check only in debug builds.
    #[inline(always)]
    pub unsafe fn page_ptr_mut_unchecked(&mut self, index: usize) -> *mut u8 {
        debug_assert!(
            index < self.committed_count,
            "Accessing uncommitted page at index {} (committed: {})",
            index,
            self.committed_count
        );

        self.ptr_at_unchecked(index)
    }

    /// Returns a immutable slice to the page at `index`.
    pub fn page_slice(&self, index: usize) -> Result<&[u8], VirtualPageStackError> {
        self.check_bounds(index)?;
        unsafe {
            let ptr = self.ptr_at_unchecked(index) as *const u8;
            Ok(std::slice::from_raw_parts(ptr, self.page_size))
        }
    }

    /// Returns a mutable slice to the page at `index`.
    pub fn page_slice_mut(&mut self, index: usize) -> Result<&mut [u8], VirtualPageStackError> {
        self.check_bounds(index)?;
        unsafe {
            let ptr = self.ptr_at_unchecked(index);
            Ok(std::slice::from_raw_parts_mut(ptr, self.page_size))
        }
    }

    /// Returns the active page size in bytes.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the total number of usable reserved pages (excluding guard page).
    pub fn capacity_pages(&self) -> usize {
        self.max_pages
    }

    /// Returns the total number of usable reserved bytes (excluding guard page).
    pub fn capacity_bytes(&self) -> usize {
        self.max_pages * self.page_size
    }

    /// Returns the number of currently committed bytes.
    pub fn committed_bytes(&self) -> usize {
        self.committed_count * self.page_size
    }

    /// Returns a list of supported page sizes on the host system.
    pub fn supported_page_sizes() -> &'static [PageSizeInfo] {
        memory::get_supported_page_sizes()
    }

    /// Finds the largest supported page size that is less than or equal to `max_size`.
    pub fn find_largest_page_size(max_size: usize) -> Result<PageSizeInfo, VirtualPageStackError> {
        Self::supported_page_sizes()
            .iter()
            .rev()
            .find(|i| i.size <= max_size)
            .copied()
            .ok_or(VirtualPageStackError::NoSupportedPageSize(max_size))
    }
}

impl Drop for VirtualPageStack {
    fn drop(&mut self) {
        // Best-effort: Ensure the size passed to release matches the exact size passed to reserve.
        // The reserved_size includes the guard page.
        // Failure here is a critical resource leak, but we avoid panicking in Drop.
        unsafe {
            if let Err(_e) = memory::release(self.base.as_ptr(), self.reserved_size) {
                // In system components, we rely on debug_assert to catch teardown issues
                // without introducing complex logging dependencies in global allocators.
                debug_assert!(
                    false,
                    "Failed to release virtual memory during Drop: {:?}",
                    _e
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_page_stack_basic() {
        let mut stack = VirtualPageStack::new(VirtualPageStack::get_default_page_size()).unwrap();

        assert_eq!(stack.committed_pages(), 0);

        // Commit 5 pages
        stack.set_committed_pages(5).expect("Commit failed");
        assert_eq!(stack.committed_pages(), 5);

        unsafe {
            let ptr0 = stack.page_ptr_mut(0);
            *ptr0 = 42;
            assert_eq!(*ptr0, 42);

            let ptr4 = stack.page_ptr_mut(4);
            *ptr4 = 133;
            assert_eq!(*ptr4, 133);
        }

        // Shrink to 2 pages
        stack.set_committed_pages(2).expect("Shrink failed");
        assert_eq!(stack.committed_pages(), 2);
    }

    #[test]
    fn test_virtual_page_stack_from_discovery() {
        let sizes = VirtualPageStack::supported_page_sizes();
        assert!(!sizes.is_empty());

        let info = sizes[0];
        let mut stack = VirtualPageStack::new(info).unwrap();
        assert_eq!(stack.page_size(), info.size);

        stack.set_committed_pages(1).expect("Commit failed");
        assert_eq!(stack.committed_pages(), 1);
    }

    #[test]
    fn test_find_largest_page_size() {
        let standard_size = memory::get_page_size();
        let info = VirtualPageStack::find_largest_page_size(standard_size).unwrap();
        assert_eq!(info.size, standard_size);
    }

    #[test]
    fn test_page_slices() {
        let mut stack = VirtualPageStack::new(VirtualPageStack::get_default_page_size()).unwrap();
        stack.grow(1).unwrap();

        // Mutable slice access
        {
            let slice = stack.page_slice_mut(0).expect("Slice access failed");
            slice[0] = 77;
            slice[1] = 88;
        }

        // Immutable slice access
        {
            let slice = stack.page_slice(0).expect("Slice access failed");
            assert_eq!(slice[0], 77);
            assert_eq!(slice[1], 88);
            assert_eq!(slice.len(), stack.page_size());
        }

        // Out of bounds check
        assert!(stack.page_slice(1).is_err());
        assert!(stack.page_slice_mut(1).is_err());
    }

    #[test]
    fn test_page_size_fallback() {
        let limit = 1024; // Too small for standard 4KB

        // Demonstrate fallback pattern
        let info = VirtualPageStack::find_largest_page_size(limit)
            .unwrap_or_else(|_| VirtualPageStack::get_default_page_size());

        assert_eq!(info.size, memory::get_page_size());
    }

    #[test]
    fn test_grow_shrink() {
        let mut stack = VirtualPageStack::new(VirtualPageStack::get_default_page_size()).unwrap();

        // Grow by 3 pages
        stack.grow(3).expect("Grow failed");
        assert_eq!(stack.committed_pages(), 3);

        // Grow by 2 more pages
        stack.grow(2).expect("Grow failed");
        assert_eq!(stack.committed_pages(), 5);

        // Shrink by 2 pages
        stack.shrink(2).expect("Shrink failed");
        assert_eq!(stack.committed_pages(), 3);

        // Shrink by everything
        stack.shrink(3).expect("Shrink failed");
        assert_eq!(stack.committed_pages(), 0);
    }

    #[test]
    fn test_numa_initialization() {
        // Enforce the "specified once at construction" invariant.
        let info = VirtualPageStack::get_default_page_size().with_numa_node(0);
        let mut stack = VirtualPageStack::new(info).unwrap();

        // All subsequent commitments now automatically use node 0.
        stack.grow(1).expect("grow failed");
        assert_eq!(stack.committed_pages(), 1);
    }

    #[test]
    #[cfg(unix)]
    fn test_guard_page_trap() {
        use std::os::unix::process::ExitStatusExt;
        use std::process::Command;

        // We run this specific test in a subprocess because it's expected to segfault.
        // If we ran it in the main test runner, it would crash the whole suite.
        let status = Command::new(std::env::current_exe().unwrap())
            .arg("allocators::virtual_page_stack::tests::identity_guard_trap")
            .arg("--ignored")
            .arg("--exact")
            .status()
            .expect("failed to execute subprocess");

        // On Unix, segfault is signal 11.
        assert_eq!(status.signal(), Some(11), "Expected segfault (signal 11)");
    }

    #[test]
    #[ignore] // Helper for test_guard_page_trap
    fn identity_guard_trap() {
        let stack = VirtualPageStack::new(VirtualPageStack::get_default_page_size()).unwrap();
        let max_pages = stack.capacity_pages();

        unsafe {
            // Accessing page at index max_pages (the guard page)
            let guard_ptr = stack.base.as_ptr().add(max_pages * stack.page_size);
            let _val = std::ptr::read_volatile(guard_ptr);
        }
    }
}
