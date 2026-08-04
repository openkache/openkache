//! Compile-time-selected storage-worker runtime and direct-I/O facade.

use std::future::Future;
use std::io;
use std::ops::Range;
#[cfg(not(feature = "storage-runtime-simulated"))]
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy)]
pub(crate) struct RuntimeConfig {
    pub(crate) worker_index: usize,
    pub(crate) entries: u32,
    pub(crate) event_interval: usize,
    pub(crate) sqpoll: bool,
    pub(crate) sqpoll_cpu: Option<usize>,
    pub(crate) worker_cpu: usize,
    pub(crate) simulated_io_latency: Duration,
}

#[cfg(feature = "storage-runtime-kimojio")]
mod backend {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    use kimojio::configuration::{Configuration, ExitBehavior};
    use kimojio::operations::{self, OFlags};

    use super::*;

    pub(crate) const NAME: &str = "kimojio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = false;

    pub(crate) const fn effective_ring_entries(_configured: u32) -> u32 {
        128
    }

    #[derive(Clone)]
    pub(crate) struct File(Rc<kimojio::OwnedFd>);

    pub(crate) trait ReadBuffer: 'static {
        fn read_capacity_mut(&mut self) -> &mut [u8];
        fn set_read_len(&mut self, initialized_len: usize);
    }

    pub(crate) trait WriteBuffer: 'static {
        fn initialized(&self) -> &[u8];
    }

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future + 'static,
    {
        let _ = (
            config.entries,
            config.event_interval,
            config.sqpoll_cpu,
            config.simulated_io_latency,
        );
        if config.sqpoll {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Kimojio storage workers do not support OpenKache SQPOLL configuration",
            ));
        }
        let worker_index = config.worker_index.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Kimojio supports at most 256 traced storage workers",
            )
        })?;
        crate::platform::pin_current_thread(config.worker_cpu)?;
        let configuration =
            Configuration::new().set_exit_behavior(ExitBehavior::WhenMainTaskCompletes);
        let mut runtime = kimojio::Runtime::new(worker_index, configuration);
        match runtime.block_on(future) {
            Some(Ok(output)) => Ok(output),
            Some(Err(_)) => Err(io::Error::other("Kimojio storage runtime task panicked")),
            None => Err(io::Error::other(
                "Kimojio storage runtime shut down before its worker completed",
            )),
        }
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        drop(operations::spawn_task(future));
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        operations::timeout_at(std::time::Instant::now() + duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn sleep(duration: Duration) -> io::Result<()> {
        operations::sleep(duration).await.map_err(io::Error::from)
    }

    pub(crate) async fn open_file(
        path: &Path,
        create: bool,
        write: bool,
        flags: i32,
    ) -> io::Result<File> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "file path contains a NUL byte")
        })?;
        let mut open_flags = if write { OFlags::RDWR } else { OFlags::RDONLY };
        if create {
            open_flags |= OFlags::CREATE;
        }
        open_flags |= OFlags::from_bits_retain(flags as _);
        let file = operations::open(&path, open_flags, 0o600u32.into())
            .await
            .map_err(io::Error::from)?;
        Ok(File(Rc::new(file)))
    }

    impl File {
        pub(crate) fn raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }

        pub(crate) async fn set_len(&self, len: u64) -> io::Result<()> {
            let len = i64::try_from(len).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "file length overflowed")
            })?;
            let result = unsafe { libc::ftruncate(self.raw_fd(), len) };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub(crate) async fn reserve_range(&self, offset: u64, len: u64) -> io::Result<()> {
            operations::fallocate(&*self.0, 0, offset, len)
                .await
                .map_err(io::Error::from)
        }

        pub(crate) async fn read_at<B>(
            &self,
            mut buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: ReadBuffer,
        {
            let range_start = range.start;
            let result =
                operations::pread(&*self.0, &mut buffer.read_capacity_mut()[range], offset)
                    .await
                    .map_err(io::Error::from);
            if let Ok(read) = &result {
                buffer.set_read_len(range_start + *read);
            }
            (result, buffer)
        }

        pub(crate) async fn write_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: WriteBuffer,
        {
            let result = operations::pwrite(&*self.0, &buffer.initialized()[range], offset)
                .await
                .map_err(io::Error::from);
            (result, buffer)
        }
    }
}

#[cfg(feature = "storage-runtime-simulated")]
mod backend {
    use std::cell::{Cell, RefCell};
    use std::collections::{BTreeMap, HashSet};

    use compio::driver::{DriverType, ProactorBuilder};
    use compio::runtime::RuntimeBuilder;

    use super::*;

    const PAGE_BYTES: usize = 4 * 1024;

    thread_local! {
        static IO_LATENCY: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    pub(crate) const NAME: &str = "simulated";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) const fn effective_ring_entries(configured: u32) -> u32 {
        configured
    }

    #[derive(Default)]
    struct SimulatedFile {
        latency: Duration,
        len: u64,
        pages: BTreeMap<u64, Box<[u8; PAGE_BYTES]>>,
    }

    #[derive(Clone)]
    pub(crate) struct File {
        state: Rc<RefCell<SimulatedFile>>,
    }

    pub(crate) trait ReadBuffer: 'static {
        fn read_capacity_mut(&mut self) -> &mut [u8];
        fn set_read_len(&mut self, initialized_len: usize);
    }

    pub(crate) trait WriteBuffer: 'static {
        fn initialized(&self) -> &[u8];
    }

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future,
    {
        let _ = (config.worker_index, config.entries, config.sqpoll_cpu);
        if config.sqpoll {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "simulated storage workers do not support SQPOLL",
            ));
        }
        IO_LATENCY.set(config.simulated_io_latency);
        let mut proactor = ProactorBuilder::new();
        proactor.driver_type(DriverType::Poll);
        let runtime = RuntimeBuilder::new()
            .with_proactor(proactor)
            .thread_affinity(HashSet::from([config.worker_cpu]))
            .event_interval(config.event_interval)
            .build()?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        compio::runtime::spawn(future).detach();
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        compio::runtime::time::timeout(duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn open_file(
        _path: &Path,
        _create: bool,
        _write: bool,
        _flags: i32,
    ) -> io::Result<File> {
        Ok(File {
            state: Rc::new(RefCell::new(SimulatedFile {
                latency: IO_LATENCY.get(),
                ..SimulatedFile::default()
            })),
        })
    }

    impl File {
        pub(crate) async fn set_len(&self, len: u64) -> io::Result<()> {
            let mut state = self.state.borrow_mut();
            state.len = len;
            let first_discarded_page = len.div_ceil(PAGE_BYTES as u64);
            state.pages.split_off(&first_discarded_page);
            if let Some(page) = state.pages.get_mut(&(len / PAGE_BYTES as u64)) {
                page[(len as usize % PAGE_BYTES)..].fill(0);
            }
            Ok(())
        }

        pub(crate) async fn read_at<B>(
            &self,
            mut buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: ReadBuffer,
        {
            let latency = self.state.borrow().latency;
            if !latency.is_zero() {
                compio::runtime::time::sleep(latency).await;
            }
            let range_start = range.start;
            let destination = &mut buffer.read_capacity_mut()[range];
            let state = self.state.borrow();
            let available = state.len.saturating_sub(offset);
            let read = destination
                .len()
                .min(usize::try_from(available).unwrap_or(usize::MAX));
            destination[..read].fill(0);
            copy_from_pages(&state.pages, offset, &mut destination[..read]);
            buffer.set_read_len(range_start + read);
            (Ok(read), buffer)
        }

        pub(crate) async fn write_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: WriteBuffer,
        {
            let latency = self.state.borrow().latency;
            if !latency.is_zero() {
                compio::runtime::time::sleep(latency).await;
            }
            let source = &buffer.initialized()[range];
            let Some(end) = offset.checked_add(source.len() as u64) else {
                return (
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "simulated file write offset overflowed",
                    )),
                    buffer,
                );
            };
            let mut state = self.state.borrow_mut();
            copy_to_pages(&mut state.pages, offset, source);
            state.len = state.len.max(end);
            (Ok(source.len()), buffer)
        }
    }

    fn copy_from_pages(
        pages: &BTreeMap<u64, Box<[u8; PAGE_BYTES]>>,
        mut offset: u64,
        mut destination: &mut [u8],
    ) {
        while !destination.is_empty() {
            let page_index = offset / PAGE_BYTES as u64;
            let page_offset = offset as usize % PAGE_BYTES;
            let chunk_len = destination.len().min(PAGE_BYTES - page_offset);
            if let Some(page) = pages.get(&page_index) {
                destination[..chunk_len]
                    .copy_from_slice(&page[page_offset..page_offset + chunk_len]);
            }
            offset += chunk_len as u64;
            destination = &mut destination[chunk_len..];
        }
    }

    fn copy_to_pages(
        pages: &mut BTreeMap<u64, Box<[u8; PAGE_BYTES]>>,
        mut offset: u64,
        mut source: &[u8],
    ) {
        while !source.is_empty() {
            let page_index = offset / PAGE_BYTES as u64;
            let page_offset = offset as usize % PAGE_BYTES;
            let chunk_len = source.len().min(PAGE_BYTES - page_offset);
            let page = pages
                .entry(page_index)
                .or_insert_with(|| Box::new([0; PAGE_BYTES]));
            page[page_offset..page_offset + chunk_len].copy_from_slice(&source[..chunk_len]);
            offset += chunk_len as u64;
            source = &source[chunk_len..];
        }
    }
}

#[derive(Debug)]
pub(crate) struct Timeout;

#[cfg(feature = "storage-runtime-compio")]
mod backend {
    use std::collections::HashSet;

    use compio::BufResult;
    use compio::buf::{IntoInner, IoBuf, IoBufMut, SetLen};
    use compio::driver::ProactorBuilder;
    use compio::fs::OpenOptions;
    use compio::io::{AsyncReadAt, AsyncWriteAt};
    use compio::runtime::RuntimeBuilder;

    use super::*;

    const SQPOLL_IDLE: Duration = Duration::from_secs(2);

    pub(crate) const NAME: &str = "compio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) const fn effective_ring_entries(configured: u32) -> u32 {
        configured
    }

    #[derive(Clone)]
    pub(crate) struct File(Rc<compio::fs::File>);

    pub(crate) trait ReadBuffer: IoBufMut + SetLen + 'static {}
    impl<T> ReadBuffer for T where T: IoBufMut + SetLen + 'static {}

    pub(crate) trait WriteBuffer: IoBuf + 'static {}
    impl<T> WriteBuffer for T where T: IoBuf + 'static {}

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future,
    {
        let _ = (config.worker_index, config.simulated_io_latency);
        let mut proactor = ProactorBuilder::new();
        proactor.capacity(config.entries);
        if config.sqpoll {
            proactor.sqpoll_idle(SQPOLL_IDLE);
            if let Some(cpu_id) = config.sqpoll_cpu {
                proactor.sqpoll_cpu(
                    cpu_id
                        .try_into()
                        .map_err(|_| io::Error::other("SQPOLL CPU identifier overflowed"))?,
                );
            }
        }
        let runtime = RuntimeBuilder::new()
            .with_proactor(proactor)
            .thread_affinity(HashSet::from([config.worker_cpu]))
            .event_interval(config.event_interval)
            .build()?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        compio::runtime::spawn(future).detach();
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        compio::runtime::time::timeout(duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn spawn_blocking<F, R>(operation: F) -> io::Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        compio::runtime::spawn_blocking(operation)
            .await
            .map_err(io::Error::from)
    }

    pub(crate) async fn open_file(
        path: &Path,
        create: bool,
        write: bool,
        flags: i32,
    ) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(create)
            .truncate(false)
            .read(true)
            .write(write)
            .custom_flags(flags)
            .open(path)
            .await?;
        Ok(File(Rc::new(file)))
    }

    impl File {
        pub(crate) fn raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }

        pub(crate) async fn set_len(&self, len: u64) -> io::Result<()> {
            self.0.set_len(len).await
        }

        pub(crate) async fn read_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: ReadBuffer,
        {
            let BufResult(result, returned) = self.0.read_at(buffer.slice(range), offset).await;
            (result, returned.into_inner())
        }

        pub(crate) async fn write_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: WriteBuffer,
        {
            let mut file = &*self.0;
            let BufResult(result, returned) = file.write_at(buffer.slice(range), offset).await;
            (result, returned.into_inner())
        }
    }
}

#[cfg(feature = "storage-runtime-monoio")]
mod backend {
    use std::os::unix::fs::OpenOptionsExt;

    use monoio::buf::{IoBuf, IoBufMut};
    use monoio::fs::OpenOptions;
    use monoio::{IoUringDriver, RuntimeBuilder};

    use super::*;

    pub(crate) const NAME: &str = "monoio";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = false;

    pub(crate) const fn effective_ring_entries(configured: u32) -> u32 {
        configured
    }

    #[derive(Clone)]
    pub(crate) struct File(Rc<monoio::fs::File>);

    pub(crate) trait ReadBuffer: IoBuf + IoBufMut + 'static {}
    impl<T> ReadBuffer for T where T: IoBuf + IoBufMut + 'static {}

    pub(crate) trait WriteBuffer: IoBuf + 'static {}
    impl<T> WriteBuffer for T where T: IoBuf + 'static {}

    pub(crate) fn run<F>(config: RuntimeConfig, future: F) -> io::Result<F::Output>
    where
        F: Future,
    {
        let _ = (
            config.worker_index,
            config.event_interval,
            config.sqpoll_cpu,
            config.simulated_io_latency,
        );
        if config.sqpoll {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Monoio storage workers do not yet support OpenKache SQPOLL configuration",
            ));
        }
        crate::platform::pin_current_thread(config.worker_cpu)?;
        let mut runtime = RuntimeBuilder::<IoUringDriver>::new()
            .with_entries(config.entries)
            .enable_timer()
            .attach_thread_pool(Box::new(monoio::blocking::DefaultThreadPool::new(1)))
            .build()?;
        Ok(runtime.block_on(future))
    }

    pub(crate) fn spawn_detached<F>(future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        drop(monoio::spawn(future));
    }

    pub(crate) async fn timeout<F>(duration: Duration, future: F) -> Result<F::Output, Timeout>
    where
        F: Future,
    {
        monoio::time::timeout(duration, future)
            .await
            .map_err(|_| Timeout)
    }

    pub(crate) async fn spawn_blocking<F, R>(operation: F) -> io::Result<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        monoio::spawn_blocking(operation)
            .await
            .map_err(|error| io::Error::other(format!("Monoio blocking task failed: {error:?}")))
    }

    pub(crate) async fn open_file(
        path: &Path,
        create: bool,
        write: bool,
        flags: i32,
    ) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(create)
            .truncate(false)
            .read(true)
            .write(write)
            .custom_flags(flags)
            .open(path)
            .await?;
        Ok(File(Rc::new(file)))
    }

    impl File {
        pub(crate) fn raw_fd(&self) -> RawFd {
            self.0.as_raw_fd()
        }

        pub(crate) async fn set_len(&self, len: u64) -> io::Result<()> {
            let len = i64::try_from(len).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "file length overflowed")
            })?;
            let result = unsafe { libc::ftruncate(self.raw_fd(), len) };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }

        pub(crate) async fn read_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: ReadBuffer,
        {
            let (result, returned) = self.0.read_at(buffer.slice_mut(range), offset).await;
            (result, returned.into_inner())
        }

        pub(crate) async fn write_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: WriteBuffer,
        {
            let (result, returned) = self.0.write_at(buffer.slice(range), offset).await;
            (result, returned.into_inner())
        }
    }
}

pub(crate) use backend::*;
