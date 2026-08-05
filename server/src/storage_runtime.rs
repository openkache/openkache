//! Compile-time-selected storage-worker runtime and direct-I/O facade.

use std::future::Future;
use std::io;
use std::ops::Range;
#[cfg(not(feature = "storage-runtime-simulated"))]
use std::os::fd::{AsRawFd, RawFd};
use std::path::Path;
#[cfg(not(feature = "storage-runtime-simulated"))]
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
    use std::cell::Cell;
    use std::collections::HashSet;

    use compio::driver::{DriverType, ProactorBuilder};
    use compio::runtime::RuntimeBuilder;

    use super::*;

    thread_local! {
        static IO_LATENCY: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    pub(crate) const NAME: &str = "simulated";
    pub(crate) const SUPPORTS_COMBINED_NETWORK_ROLE: bool = true;

    pub(crate) const fn effective_ring_entries(configured: u32) -> u32 {
        configured
    }

    #[derive(Clone)]
    pub(crate) struct File {
        latency: Duration,
    }

    pub(crate) trait ReadBuffer: 'static {
        fn set_read_len(&mut self, initialized_len: usize);
    }

    pub(crate) trait WriteBuffer: 'static {}

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
            latency: IO_LATENCY.get(),
        })
    }

    impl File {
        pub(crate) async fn set_len(&self, _len: u64) -> io::Result<()> {
            Ok(())
        }

        pub(crate) async fn read_at<B>(
            &self,
            mut buffer: B,
            range: Range<usize>,
            _offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: ReadBuffer,
        {
            if !self.latency.is_zero() {
                compio::runtime::time::sleep(self.latency).await;
            }
            let read = range.len();
            buffer.set_read_len(range.end);
            (Ok(read), buffer)
        }

        pub(crate) async fn write_at<B>(
            &self,
            buffer: B,
            range: Range<usize>,
            _offset: u64,
        ) -> (io::Result<usize>, B)
        where
            B: WriteBuffer,
        {
            if !self.latency.is_zero() {
                compio::runtime::time::sleep(self.latency).await;
            }
            (Ok(range.len()), buffer)
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

#[cfg(feature = "storage-runtime-simulated")]
pub(crate) fn file_device_kind(_file: &File) -> crate::platform::StorageDeviceKind {
    crate::platform::StorageDeviceKind::Unknown
}

#[cfg(not(feature = "storage-runtime-simulated"))]
pub(crate) fn file_device_kind(file: &File) -> crate::platform::StorageDeviceKind {
    crate::platform::storage_device_kind_from_fd(file.raw_fd())
}
