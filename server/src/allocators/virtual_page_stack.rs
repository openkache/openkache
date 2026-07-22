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
/// This implementation is strictly single-threaded.
///
/// # Examples
///
/// ```
/// use openkache::allocators::virtual_page_stack::VirtualPageStack;
///
/// let mut stack = VirtualPageStack::new(
///     VirtualPageStack::get_default_page_size()
/// ).unwrap();
///
/// // Commit 3 pages of physical memory.
/// stack.set_committed_pages(3).unwrap();
/// assert_eq!(stack.committed_pages(), 3);
///
/// // Write to the first and third pages.
/// unsafe {
///     let p0 = stack.page_ptr_mut_unchecked(0);
///     let p2 = stack.page_ptr_mut_unchecked(2);
///     *p0 = 42;
///     *p2 = 99;
/// }
///
/// // Shrink back to 1 page.
/// stack.set_committed_pages(1).unwrap();
/// assert_eq!(stack.committed_pages(), 1);
/// ```
pub struct VirtualPageStack {
    base: NonNull<u8>,
    max_pages: usize,
    page_size: usize,
    reserved_size: usize,
    flags: MemoryFlags,
    committed_count: usize,
    _marker: PhantomData<*const u8>,
}

impl VirtualPageStack {
    /// Creates a new VirtualPageStack by reserving virtual address space.
    ///
    /// Capacity is automatically sized to the host's physical RAM.
    /// A guard page is placed at the end of the range as a hardware trap
    /// for out-of-bounds access. No physical memory is committed yet.
    ///
    /// # Arguments
    ///
    /// * `info` - Page size and associated flags (huge pages, NUMA, etc.).
    ///
    /// # Returns
    ///
    /// `Ok(Self)` on success.
    ///
    /// # Panics
    ///
    /// Panics if `info.size` is zero.
    ///
    /// # Errors
    ///
    /// Returns `CapacityOverflow` if `total_ram / info.size + 1` overflows `usize`.
    /// Returns `ReservationFailed` if the OS cannot reserve the virtual address range
    /// or if it fails to align to the requested page size.
    /// Returns `GuardProtectFailed` if the guard page cannot be set to NOACCESS.
    ///
    /// # Examples
    ///
    /// ```
    /// use openkache::allocators::virtual_page_stack::VirtualPageStack;
    ///
    /// let stack = VirtualPageStack::new(
    ///     VirtualPageStack::get_default_page_size()
    /// ).unwrap();
    /// assert_eq!(stack.committed_pages(), 0);
    /// ```
    pub fn new(info: PageSizeInfo) -> Result<Self, VirtualPageStackError> {
        assert!(info.size > 0, "Page size must be positive");

        let total_ram = memory::get_total_physical_memory();
        let max_pages = total_ram / info.size;

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
                        _ => std::io::Error::other(e.to_string()),
                    },
                }
            })?
        };

        if !(base.as_ptr() as usize).is_multiple_of(info.size) {
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

    /// Returns the standard (non-huge) system page size info.
    ///
    /// The result is cached after the first successful discovery.
    ///
    /// # Returns
    ///
    /// The smallest non-hugepage `PageSizeInfo` available on the host.
    ///
    /// # Panics
    ///
    /// Panics if no standard page size is discovered (unlikely on real hardware).
    ///
    /// # Examples
    ///
    /// ```
    /// use openkache::allocators::virtual_page_stack::VirtualPageStack;
    ///
    /// let info = VirtualPageStack::get_default_page_size();
    /// assert!(info.size > 0);
    /// assert!(!info.flags.huge_pages);
    /// ```
    pub fn get_default_page_size() -> PageSizeInfo {
        Self::try_default_page_size().expect("No standard page size discovered")
    }

    /// Fallible version of `get_default_page_size`.
    ///
    /// # Returns
    ///
    /// `Ok(PageSizeInfo)` on success.
    ///
    /// # Errors
    ///
    /// Returns `NoSupportedPageSize` if no standard (non-huge) page size is found.
    pub fn try_default_page_size() -> Result<PageSizeInfo, VirtualPageStackError> {
        static CACHE: OnceLock<PageSizeInfo> = OnceLock::new();
        Ok(*CACHE.get_or_init(|| {
            Self::supported_page_sizes()
                .iter()
                .filter(|i| !i.flags.huge_pages)
                .min_by_key(|i| i.size)
                .copied()
                .expect("No standard page size discovered")
        }))
    }

    /// Returns the number of currently committed (physically backed) pages.
    pub fn committed_pages(&self) -> usize {
        self.committed_count
    }

    /// Adjusts the number of committed physical pages.
    ///
    /// Newly committed pages are **guaranteed zero-filled**:
    /// - On first commit (freshly reserved pages): OS always provides zero-fill
    ///   (`mmap MAP_ANONYMOUS` / `VirtualAlloc`).
    /// - After decommit+recommit: `madvise(MADV_DONTNEED)` on Linux /
    ///   `MEM_DECOMMIT` on Windows ensures physical pages are released and
    ///   re-faulted pages are zero-filled.
    ///
    /// Decommitting a page causes immediate data loss; re-committing returns fresh,
    /// zero-filled memory.
    ///
    /// # Arguments
    ///
    /// * `new_count` - Target number of committed pages. Growing commits pages at the
    ///   end of the current range; shrinking decommits from the end.
    ///
    /// # Errors
    ///
    /// Returns `CommitExceedsCapacity` if `new_count` exceeds the stack's capacity.
    /// Returns `CommitFailed` if the OS fails to back the pages with physical memory.
    /// Returns `DecommitFailed` if the OS fails to release physical memory.
    /// Returns `CapacityOverflow` if internal arithmetic overflows `usize` (should not
    /// happen with realistic `new_count` values).
    ///
    /// # Examples
    ///
    /// ```
    /// use openkache::allocators::virtual_page_stack::VirtualPageStack;
    ///
    /// let mut stack = VirtualPageStack::new(
    ///     VirtualPageStack::get_default_page_size()
    /// ).unwrap();
    ///
    /// // Grow from 0 to 10 committed pages.
    /// stack.set_committed_pages(10).unwrap();
    /// assert_eq!(stack.committed_pages(), 10);
    ///
    /// // Shrink from 10 to 3 committed pages.
    /// stack.set_committed_pages(3).unwrap();
    /// assert_eq!(stack.committed_pages(), 3);
    ///
    /// // Setting to the same value is a no-op.
    /// stack.set_committed_pages(3).unwrap();
    /// assert_eq!(stack.committed_pages(), 3);
    /// ```
    pub fn set_committed_pages(&mut self, new_count: usize) -> Result<(), VirtualPageStackError> {
        let current = self.committed_count;
        match new_count.cmp(&current) {
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Greater => self.grow(new_count - current),
            std::cmp::Ordering::Less => self.shrink(current - new_count),
        }
    }

    #[inline]
    fn grow(&mut self, delta: usize) -> Result<(), VirtualPageStackError> {
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

    #[inline]
    fn shrink(&mut self, delta: usize) -> Result<(), VirtualPageStackError> {
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

    #[inline(always)]
    fn bytes_for_pages_unchecked(&self, pages: usize) -> usize {
        let off = pages * self.page_size;
        debug_assert_eq!(
            off / self.page_size,
            pages,
            "Pointer offset overflowed usize"
        );
        off
    }

    #[inline(always)]
    fn ptr_at_unchecked(&self, index: usize) -> *mut u8 {
        let off = self.bytes_for_pages_unchecked(index);
        unsafe { self.base.as_ptr().add(off) }
    }

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

    /// Returns a pointer to the start of the page at `index`.
    /// Bounds check is performed only in debug builds.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index. Must be `< committed_pages()` in release builds.
    ///
    /// # Returns
    ///
    /// A raw `*const u8` pointer to the page. Adjacent pages are contiguous in
    /// virtual address space (gap equals `page_size()`).
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index < committed_pages()` in release builds
    /// and that the returned pointer is not used after the page is decommitted.
    ///
    /// # Examples
    ///
    /// ```
    /// use openkache::allocators::virtual_page_stack::VirtualPageStack;
    ///
    /// let mut stack = VirtualPageStack::new(
    ///     VirtualPageStack::get_default_page_size()
    /// ).unwrap();
    /// stack.set_committed_pages(2).unwrap();
    ///
    /// unsafe {
    ///     let p0 = stack.page_ptr_unchecked(0);
    ///     let p1 = stack.page_ptr_unchecked(1);
    ///     // Pages are initially uninitialised, but the pointers are valid.
    ///     assert_ne!(p0, p1);
    ///     assert_eq!((p1 as usize) - (p0 as usize), stack.page_size());
    /// }
    /// ```
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

    /// Returns a mutable pointer to the start of the page at `index`.
    /// Bounds check is performed only in debug builds.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index. Must be `< committed_pages()` in release builds.
    ///
    /// # Returns
    ///
    /// A raw `*mut u8` pointer to the page.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index < committed_pages()` in release builds,
    /// that the returned pointer does not alias any live reference,
    /// and that it is not used after the page is decommitted.
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

    /// Checked version — returns a pointer to the page at `index` or an error.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index.
    ///
    /// # Returns
    ///
    /// `Ok(*const u8)` on success.
    ///
    /// # Errors
    ///
    /// Returns `IndexOutOfBounds` if `index >= committed_pages()`.
    #[inline]
    pub fn page_ptr_at(&self, index: usize) -> Result<*const u8, VirtualPageStackError> {
        self.check_bounds(index)?;
        Ok(self.ptr_at_unchecked(index) as *const u8)
    }

    /// Checked version — returns a mutable pointer to the page at `index` or an error.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index.
    ///
    /// # Returns
    ///
    /// `Ok(*mut u8)` on success.
    ///
    /// # Errors
    ///
    /// Returns `IndexOutOfBounds` if `index >= committed_pages()`.
    #[inline]
    pub fn page_ptr_at_mut(&mut self, index: usize) -> Result<*mut u8, VirtualPageStackError> {
        self.check_bounds(index)?;
        Ok(self.ptr_at_unchecked(index))
    }

    /// Returns an immutable byte slice covering the page at `index`.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index.
    ///
    /// # Returns
    ///
    /// `Ok(&[u8])` of length `page_size()`.
    ///
    /// # Errors
    ///
    /// Returns `IndexOutOfBounds` if `index >= committed_pages()`.
    pub fn page_slice(&self, index: usize) -> Result<&[u8], VirtualPageStackError> {
        self.check_bounds(index)?;
        unsafe {
            let ptr = self.ptr_at_unchecked(index) as *const u8;
            Ok(std::slice::from_raw_parts(ptr, self.page_size))
        }
    }

    /// Returns a mutable byte slice covering the page at `index`.
    ///
    /// # Arguments
    ///
    /// * `index` - Page index.
    ///
    /// # Returns
    ///
    /// `Ok(&mut [u8])` of length `page_size()`.
    ///
    /// # Errors
    ///
    /// Returns `IndexOutOfBounds` if `index >= committed_pages()`.
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

    /// Returns the list of page sizes the host OS supports (cached after first call).
    ///
    /// Includes both standard pages and any available hugepage sizes.
    /// Sorted by increasing size, deduplicated.
    pub fn supported_page_sizes() -> &'static [PageSizeInfo] {
        memory::get_supported_page_sizes()
    }

    /// Finds the largest supported page size ≤ `max_size`.
    ///
    /// # Arguments
    ///
    /// * `max_size` - Upper bound in bytes.
    ///
    /// # Returns
    ///
    /// `Ok(PageSizeInfo)` for the largest supported page size not exceeding `max_size`.
    ///
    /// # Errors
    ///
    /// Returns `NoSupportedPageSize` if every supported page size is larger than `max_size`.
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
        unsafe {
            if let Err(_e) = memory::release(self.base.as_ptr(), self.reserved_size) {
                debug_assert!(
                    false,
                    "Failed to release virtual memory during Drop: {:?}",
                    _e
                );
            }
        }
    }
}
