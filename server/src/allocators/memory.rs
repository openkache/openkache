//! Low-level memory management wrappers for OS memory operations.
//!
//! Provides safe-ish abstractions over `mmap`/`mprotect`/`munmap` (Unix) and
//! `VirtualAlloc`/`VirtualFree` (Windows), including reservation, commitment,
//! decommitment, guard-page protection, and page-size discovery.

use std::ptr::{self, NonNull};
use std::sync::OnceLock;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("Failed to reserve {size} bytes: {source}")]
    ReserveFailed { size: usize, source: std::io::Error },
    #[cfg(unix)]
    #[error("Failed to commit legacy pages at {addr:?}: {source}")]
    CommitFailed {
        addr: *mut u8,
        source: std::io::Error,
    },
    #[cfg(unix)]
    #[error("Failed to decommit legacy pages at {addr:?}: {source}")]
    DecommitFailed {
        addr: *mut u8,
        source: std::io::Error,
    },
    #[error("Failed to release memory at {addr:?}: {source}")]
    ReleaseFailed {
        addr: *mut u8,
        source: std::io::Error,
    },
    #[error("Failed to {op} at {addr:?}: {source}")]
    OperationFailed {
        op: &'static str,
        addr: *mut u8,
        source: std::io::Error,
    },
}

pub type Result<T> = std::result::Result<T, MemoryError>;

#[cfg(unix)]
use libc::{
    MAP_ANON, MAP_FAILED, MAP_PRIVATE, PROT_NONE, PROT_READ, PROT_WRITE, madvise, mmap, mprotect,
    munmap,
};

// Linux-specific flags if not in libc
#[cfg(all(unix, target_os = "linux"))]
mod linux_flags {
    pub const MAP_HUGETLB: libc::c_int = 0x40000;
    pub const MAP_HUGE_SHIFT: libc::c_int = 26;
    pub const MAP_NORESERVE: libc::c_int = 0x4000;
}

#[cfg(windows)]
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_DECOMMIT, MEM_LARGE_PAGES, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS,
    PAGE_READWRITE, VirtualAlloc, VirtualFree,
};

static SUPPORTED_PAGE_SIZES: OnceLock<Vec<PageSizeInfo>> = OnceLock::new();

/// Flags to control memory reservation and commitment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryFlags {
    /// Request large pages (hugepages).
    pub huge_pages: bool,
    /// The log2 of the huge page size.
    /// If 0, the default huge page size is used.
    /// Only applicable if `huge_pages` is true.
    pub huge_page_size_log2: u8,
    /// Do not reserve swap space for this mapping.
    /// On Linux, this corresponds to MAP_NORESERVE.
    /// Useful for defending against "overcommit disabled" environments.
    pub no_reserve: bool,
    /// Preferred NUMA node for the memory.
    /// If Some(node), the memory will be bound to that node during commitment.
    pub numa_node: Option<u16>,
}

impl MemoryFlags {
    /// Sets the preferred NUMA node.
    pub fn with_numa_node(mut self, node: u16) -> Self {
        self.numa_node = Some(node);
        self
    }
}

/// Information about a supported page size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSizeInfo {
    /// The page size in bytes.
    pub size: usize,
    /// The flags required to use this page size.
    pub flags: MemoryFlags,
}

impl PageSizeInfo {
    /// Ergonomic helper to override the NUMA node for this page size.
    pub fn with_numa_node(mut self, node: u16) -> Self {
        self.flags = self.flags.with_numa_node(node);
        self
    }
}

/// Returns the system page size in bytes.
pub fn get_page_size() -> usize {
    #[cfg(unix)]
    unsafe {
        libc::sysconf(libc::_SC_PAGESIZE) as usize
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
        let mut info: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut info);
        info.dwPageSize as usize
    }
}

/// Returns the total physical memory (RAM) in bytes.
pub fn get_total_physical_memory() -> usize {
    #[cfg(unix)]
    unsafe {
        let pages = libc::sysconf(libc::_SC_PHYS_PAGES) as usize;
        let page_size = libc::sysconf(libc::_SC_PAGESIZE) as usize;
        pages * page_size
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Memory::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut status);
        status.ullTotalPhys as usize
    }
}

/// Returns a list of supported page sizes on the host system.
pub fn get_supported_page_sizes() -> &'static [PageSizeInfo] {
    SUPPORTED_PAGE_SIZES
        .get_or_init(discover_supported_page_sizes)
        .as_slice()
}

fn discover_supported_page_sizes() -> Vec<PageSizeInfo> {
    let mut infos = Vec::new();

    // Standard page size
    infos.push(PageSizeInfo {
        size: get_page_size(),
        flags: MemoryFlags {
            huge_pages: false,
            huge_page_size_log2: 0,
            no_reserve: false,
            numa_node: None,
        },
    });

    #[cfg(all(unix, target_os = "linux"))]
    {
        if let Ok(entries) = std::fs::read_dir("/sys/kernel/mm/hugepages/") {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str()
                    && let Some(rest) = name.strip_prefix("hugepages-")
                    && let Some(kb) = rest.strip_suffix("kB")
                    && let Ok(size_kb) = kb.parse::<usize>()
                {
                    let Some(size_bytes) = size_kb.checked_mul(1024) else {
                        continue;
                    };
                    infos.push(PageSizeInfo {
                        size: size_bytes,
                        flags: MemoryFlags {
                            huge_pages: true,
                            huge_page_size_log2: size_bytes.trailing_zeros() as u8,
                            no_reserve: false,
                            numa_node: None,
                        },
                    });
                }
            }
        }
    }

    #[cfg(windows)]
    {
        unsafe {
            use windows_sys::Win32::System::Memory::GetLargePageMinimum;
            let large_page_min = GetLargePageMinimum();
            if large_page_min > 0 {
                let info = PageSizeInfo {
                    size: large_page_min,
                    flags: MemoryFlags {
                        huge_pages: true,
                        huge_page_size_log2: large_page_min.trailing_zeros() as u8,
                        no_reserve: false,
                        numa_node: None,
                    },
                };
                if !infos.iter().any(|i| i.size == large_page_min) {
                    infos.push(info);
                }
            }
        }
    }

    infos.sort_by_key(|i| i.size);
    infos.dedup_by_key(|i| i.size);
    infos
}

/// Reserves a range of virtual address space.
/// # Safety
///
/// The caller must ensure that `capacity` is non-zero and that the returned
/// memory range is not accessed before `commit` is called.
pub unsafe fn reserve(capacity: usize, flags: MemoryFlags) -> Result<NonNull<u8>> {
    #[cfg(unix)]
    {
        let mmap_flags = {
            #[cfg(not(target_os = "linux"))]
            {
                let _ = flags;
                MAP_PRIVATE | MAP_ANON
            }
            #[cfg(target_os = "linux")]
            {
                let mut mmap_flags = MAP_PRIVATE | MAP_ANON;
                if flags.huge_pages {
                    mmap_flags |= linux_flags::MAP_HUGETLB;
                    if flags.huge_page_size_log2 > 0 {
                        mmap_flags |= (flags.huge_page_size_log2 as libc::c_int)
                            << linux_flags::MAP_HUGE_SHIFT;
                    }
                }
                if flags.no_reserve {
                    mmap_flags |= linux_flags::MAP_NORESERVE;
                }
                mmap_flags
            }
        };

        let addr = unsafe { mmap(ptr::null_mut(), capacity, PROT_NONE, mmap_flags, -1, 0) };
        if addr == MAP_FAILED {
            return Err(MemoryError::ReserveFailed {
                size: capacity,
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(unsafe { NonNull::new_unchecked(addr as *mut u8) })
    }
    #[cfg(windows)]
    {
        let mut win_flags = MEM_RESERVE;
        if flags.huge_pages {
            win_flags |= MEM_LARGE_PAGES;
        }

        let addr = unsafe { VirtualAlloc(ptr::null(), capacity, win_flags, PAGE_NOACCESS) };
        if addr.is_null() {
            return Err(MemoryError::ReserveFailed {
                size: capacity,
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(unsafe { NonNull::new_unchecked(addr as *mut u8) })
    }
}

/// Commits physical memory to a reserved range of virtual addresses.
///
/// # Atomicity Contract
///
/// This function assumes that the OS commitment operation is all-or-nothing.
/// While some OS implementations (like Windows `MEM_COMMIT`) may technically
/// allow partial success in extremely rare failure modes, this abstraction
/// treats failures as hard errors that require the caller to handle state consistency.
/// # Safety
///
/// The caller must ensure that `addr` and `size` refer to a previously
/// reserved range and that the range is not aliased in an invalid way.
pub unsafe fn commit(addr: *mut u8, size: usize, flags: MemoryFlags) -> Result<()> {
    #[cfg(unix)]
    {
        #[cfg(not(target_os = "linux"))]
        let _ = flags;
        if unsafe { mprotect(addr as *mut _, size, PROT_READ | PROT_WRITE) } != 0 {
            return Err(MemoryError::OperationFailed {
                op: "commit",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }

        #[cfg(target_os = "linux")]
        if let Some(node) = flags.numa_node {
            let Some(nodemask) = 1usize.checked_shl(node as u32) else {
                let err = std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("NUMA node {node} exceeds usize bit width"),
                );
                eprintln!("WARN: NUMA node {} too large for shift: {}", node, err);
                return Ok(());
            };
            let result = unsafe {
                libc::syscall(
                    libc::SYS_mbind,
                    addr as *mut libc::c_void,
                    size as libc::size_t,
                    2, // MPOL_PREFERRED
                    &nodemask as *const usize as *const libc::c_void,
                    (std::mem::size_of::<usize>() * 8 + 1) as libc::c_ulong,
                    0,
                )
            };
            if result != 0 {
                let err = std::io::Error::last_os_error();
                eprintln!(
                    "WARN: mbind NUMA binding for node {} failed ({}). \
                     Memory will not be bound to the preferred NUMA node.",
                    node, err
                );
            }
        }
    }
    #[cfg(windows)]
    {
        let mut win_flags = MEM_COMMIT;
        if flags.huge_pages {
            win_flags |= MEM_LARGE_PAGES;
        }

        let ptr = if let Some(node) = flags.numa_node {
            unsafe {
                windows_sys::Win32::System::Memory::VirtualAllocExNuma(
                    windows_sys::Win32::System::Threading::GetCurrentProcess(),
                    addr as *mut _,
                    size,
                    win_flags,
                    PAGE_READWRITE,
                    node as u32,
                )
            }
        } else {
            unsafe { VirtualAlloc(addr as *mut _, size, win_flags, PAGE_READWRITE) }
        };

        if ptr.is_null() {
            return Err(MemoryError::OperationFailed {
                op: "commit (VirtualAlloc/VirtualAllocExNuma)",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    Ok(())
}

/// Decommits physical memory from a range of virtual addresses.
/// # Safety
///
/// The caller must ensure that `addr` and `size` refer to a previously
/// committed range and that no accesses occur after decommit.
pub unsafe fn decommit(addr: *mut u8, size: usize) -> Result<()> {
    #[cfg(unix)]
    {
        if unsafe { mprotect(addr as *mut _, size, PROT_NONE) } != 0 {
            return Err(MemoryError::OperationFailed {
                op: "decommit",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }

        // MADV_DONTNEED releases physical pages, ensuring that subsequent
        // recommit plus access provides zero-filled pages.
        // Without this, mprotect(PROT_NONE) only changes page permissions
        // and does **not** release physical memory, breaking the
        // zero-fill contract relied on by Slab.
        if unsafe { madvise(addr as *mut libc::c_void, size, libc::MADV_DONTNEED) } != 0 {
            return Err(MemoryError::DecommitFailed {
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    #[cfg(windows)]
    {
        if unsafe { VirtualFree(addr as *mut _, size, MEM_DECOMMIT) } == 0 {
            return Err(MemoryError::OperationFailed {
                op: "decommit",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    Ok(())
}

/// Sets a range of virtual addresses to NOACCESS, ensuring any access triggers a hardware fault.
/// This is used for guard pages and should be independent of reservation/commitment semantics.
/// # Safety
///
/// The caller must ensure that `addr` and `size` are within a valid
/// reserved range and that no code will subsequently access it.
pub unsafe fn protect_noaccess(addr: *mut u8, size: usize) -> Result<()> {
    #[cfg(unix)]
    {
        if unsafe { mprotect(addr as *mut _, size, PROT_NONE) } != 0 {
            return Err(MemoryError::OperationFailed {
                op: "protect_noaccess (mprotect PROT_NONE)",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    #[cfg(windows)]
    {
        let mut old_protect = 0;
        if unsafe {
            windows_sys::Win32::System::Memory::VirtualProtect(
                addr as *mut _,
                size,
                windows_sys::Win32::System::Memory::PAGE_NOACCESS,
                &mut old_protect,
            )
        } == 0
        {
            return Err(MemoryError::OperationFailed {
                op: "protect_noaccess (VirtualProtect PAGE_NOACCESS)",
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    Ok(())
}

/// Releases a reserved range of virtual address space.
/// # Safety
///
/// The caller must ensure that `addr` and `capacity` match exactly the
/// values passed to `reserve`, and that the memory is no longer used.
pub unsafe fn release(addr: *mut u8, capacity: usize) -> Result<()> {
    #[cfg(unix)]
    {
        if unsafe { munmap(addr as *mut _, capacity) } != 0 {
            return Err(MemoryError::ReleaseFailed {
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    #[cfg(windows)]
    {
        if unsafe { VirtualFree(addr as *mut _, 0, MEM_RELEASE) } == 0 {
            return Err(MemoryError::ReleaseFailed {
                addr,
                source: std::io::Error::last_os_error(),
            });
        }
    }
    Ok(())
}
