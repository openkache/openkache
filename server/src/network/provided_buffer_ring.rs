use std::io;
use std::mem::size_of;
use std::sync::atomic::{AtomicU16, Ordering};

use io_uring::{IoUring, types};

pub(super) struct ProvidedBufferRing {
    entries: *mut types::BufRingEntry,
    buffers: Vec<Box<[u8]>>,
    tail: u16,
    mask: u16,
}

impl ProvidedBufferRing {
    /// # Safety
    ///
    /// The registered io_uring must be destroyed or the buffer group unregistered before this
    /// value is dropped.
    pub(super) unsafe fn new(
        io_uring: &IoUring,
        bgid: u16,
        entry_count: u16,
        buffer_size: usize,
    ) -> io::Result<Self> {
        if entry_count == 0 || !entry_count.is_power_of_two() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provided buffer ring entry count must be a power of two",
            ));
        }

        let buffer_size = u32::try_from(buffer_size).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "provided buffer size exceeds u32::MAX",
            )
        })?;

        if buffer_size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "provided buffer size must be non-zero",
            ));
        }

        let mut buffers = (0..entry_count)
            .map(|_| vec![0; buffer_size as usize].into_boxed_slice())
            .collect::<Vec<_>>();
        let entries_size = usize::from(entry_count) * size_of::<types::BufRingEntry>();
        let mapped = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                entries_size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if mapped == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }

        let entries = mapped.cast::<types::BufRingEntry>();

        if let Err(error) = unsafe {
            io_uring
                .submitter()
                .register_buf_ring_with_flags(entries as u64, entry_count, bgid, 0)
        } {
            unsafe {
                libc::munmap(mapped, entries_size);
            }
            return Err(error);
        }

        for bid in 0..entry_count {
            let buffer = &mut buffers[usize::from(bid)];
            let entry = unsafe { &mut *entries.add(usize::from(bid)) };
            entry.set_addr(buffer.as_mut_ptr() as u64);
            entry.set_len(buffer_size);
            entry.set_bid(bid);
        }

        let shared_tail = unsafe { &*types::BufRingEntry::tail(entries).cast::<AtomicU16>() };
        shared_tail.store(entry_count, Ordering::Release);

        Ok(Self {
            entries,
            buffers,
            tail: entry_count,
            mask: entry_count - 1,
        })
    }

    pub(super) fn received(&self, bid: u16, received: usize) -> &[u8] {
        &self.buffers[usize::from(bid)][..received]
    }

    pub(super) fn recycle(&mut self, bid: u16) {
        let slot = usize::from(self.tail & self.mask);
        let buffer = &mut self.buffers[usize::from(bid)];
        let entry = unsafe { &mut *self.entries.add(slot) };
        entry.set_addr(buffer.as_mut_ptr() as u64);
        entry.set_len(buffer.len() as u32);
        entry.set_bid(bid);

        self.tail = self.tail.wrapping_add(1);
        let shared_tail = unsafe { &*types::BufRingEntry::tail(self.entries).cast::<AtomicU16>() };
        shared_tail.store(self.tail, Ordering::Release);
    }
}

impl Drop for ProvidedBufferRing {
    fn drop(&mut self) {
        let entry_count = usize::from(self.mask) + 1;
        let entries_size = entry_count * size_of::<types::BufRingEntry>();
        let result = unsafe { libc::munmap(self.entries.cast(), entries_size) };
        debug_assert_eq!(result, 0);
    }
}
