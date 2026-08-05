//! Compile-time-selected channel backend used by the server runtime.
//
// Transport implementations consume different subsets of this internal API.
// Keep the channel contract transport-agnostic and let unused operations be
// removed with the unselected transport code.
#![allow(dead_code)]

use std::time::Duration;

fn assert_bounded_capacity(capacity: usize) {
    assert!(capacity > 0, "bounded channel capacity must be positive");
}

#[cfg(feature = "storage-runtime-kimojio")]
async fn wait_for_storage_channel_poll() -> Result<(), RecvError> {
    crate::storage_runtime::sleep(Duration::from_micros(10))
        .await
        .map_err(|_| RecvError)
}

#[cfg(feature = "network-runtime-kimojio")]
async fn wait_for_network_channel_poll() {
    crate::network_runtime::sleep(Duration::from_micros(10)).await;
}

#[derive(Debug, thiserror::Error)]
#[error("sending on a disconnected channel")]
pub(crate) struct SendError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum TrySendError<T> {
    #[error("sending on a full channel")]
    Full(T),
    #[error("sending on a disconnected channel")]
    Disconnected(T),
}

#[derive(Debug, thiserror::Error)]
#[error("sending on a full or disconnected channel")]
pub(crate) struct SendTimeoutError;

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("receiving on an empty and disconnected channel")]
pub(crate) struct RecvError;

#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(crate) enum TryRecvError {
    #[error("receiving on an empty channel")]
    Empty,
    #[error("receiving on an empty and disconnected channel")]
    Disconnected,
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("receiving on an empty or disconnected channel")]
pub(crate) struct RecvTimeoutError;

#[cfg(feature = "channel-crossfire")]
mod backend {
    use super::TrySendError;
    use super::{
        Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError,
        assert_bounded_capacity,
    };
    use crossfire::{AsyncRx, MAsyncTx, MTx, Rx, mpsc};

    enum SenderInner<T: 'static> {
        BoundedSync(MTx<mpsc::Array<T>>),
        BoundedAsync {
            blocking: MTx<mpsc::Array<T>>,
            asynchronous: MAsyncTx<mpsc::Array<T>>,
        },
        Unbounded(MTx<mpsc::List<T>>),
    }

    pub(crate) struct Sender<T: 'static> {
        inner: SenderInner<T>,
    }

    impl<T: 'static> Clone for Sender<T> {
        fn clone(&self) -> Self {
            let inner = match &self.inner {
                SenderInner::BoundedSync(sender) => SenderInner::BoundedSync(sender.clone()),
                SenderInner::BoundedAsync {
                    blocking,
                    asynchronous,
                } => SenderInner::BoundedAsync {
                    blocking: blocking.clone(),
                    asynchronous: asynchronous.clone(),
                },
                SenderInner::Unbounded(sender) => SenderInner::Unbounded(sender.clone()),
            };
            Self { inner }
        }
    }

    pub(crate) struct Receiver<T: 'static> {
        inner: Rx<mpsc::Array<T>>,
    }

    enum AsyncReceiverInner<T: 'static> {
        Bounded(AsyncRx<mpsc::Array<T>>),
        Unbounded(AsyncRx<mpsc::List<T>>),
    }

    pub(crate) struct AsyncReceiver<T: 'static> {
        inner: AsyncReceiverInner<T>,
    }

    pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>)
    where
        T: Send + Unpin + 'static,
    {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = mpsc::bounded_blocking(capacity);
        (
            Sender {
                inner: SenderInner::BoundedSync(sender),
            },
            Receiver { inner: receiver },
        )
    }

    pub(crate) fn bounded_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>)
    where
        T: Send + Unpin + 'static,
    {
        assert_bounded_capacity(capacity);
        let (asynchronous, receiver) = mpsc::bounded_async(capacity);
        let blocking = asynchronous.clone().into_blocking();
        (
            Sender {
                inner: SenderInner::BoundedAsync {
                    blocking,
                    asynchronous,
                },
            },
            AsyncReceiver {
                inner: AsyncReceiverInner::Bounded(receiver),
            },
        )
    }

    pub(crate) fn bounded_sync_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>)
    where
        T: Send + Unpin + 'static,
    {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = mpsc::bounded_blocking_async(capacity);
        (
            Sender {
                inner: SenderInner::BoundedSync(sender),
            },
            AsyncReceiver {
                inner: AsyncReceiverInner::Bounded(receiver),
            },
        )
    }

    pub(crate) fn unbounded_async<T>() -> (Sender<T>, AsyncReceiver<T>)
    where
        T: Send + Unpin + 'static,
    {
        let (sender, receiver) = mpsc::unbounded_async();
        (
            Sender {
                inner: SenderInner::Unbounded(sender),
            },
            AsyncReceiver {
                inner: AsyncReceiverInner::Unbounded(receiver),
            },
        )
    }

    impl<T> Sender<T>
    where
        T: Send + Unpin + 'static,
    {
        pub(crate) fn send(&self, value: T) -> Result<(), SendError> {
            let result = match &self.inner {
                SenderInner::BoundedSync(sender) => sender.send(value),
                SenderInner::BoundedAsync { blocking, .. } => blocking.send(value),
                SenderInner::Unbounded(sender) => sender.send(value),
            };
            result.map_err(|_| SendError)
        }

        pub(crate) async fn send_async(&self, value: T) -> Result<(), SendError> {
            match &self.inner {
                SenderInner::BoundedSync(sender) => sender
                    .clone()
                    .into_async()
                    .send(value)
                    .await
                    .map_err(|_| SendError),
                SenderInner::BoundedAsync { asynchronous, .. } => {
                    asynchronous.send(value).await.map_err(|_| SendError)
                }
                SenderInner::Unbounded(sender) => sender.send(value).map_err(|_| SendError),
            }
        }

        pub(crate) fn send_timeout(
            &self,
            value: T,
            timeout: Duration,
        ) -> Result<(), SendTimeoutError> {
            let result = match &self.inner {
                SenderInner::BoundedSync(sender) => sender.send_timeout(value, timeout),
                SenderInner::BoundedAsync { blocking, .. } => blocking.send_timeout(value, timeout),
                SenderInner::Unbounded(sender) => {
                    return sender.send(value).map_err(|_| SendTimeoutError);
                }
            };
            result.map_err(|_| SendTimeoutError)
        }

        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            let result = match &self.inner {
                SenderInner::BoundedSync(sender) => sender.try_send(value),
                SenderInner::BoundedAsync { asynchronous, .. } => asynchronous.try_send(value),
                SenderInner::Unbounded(sender) => sender.try_send(value),
            };
            result.map_err(|error| match error {
                crossfire::TrySendError::Full(value) => TrySendError::Full(value),
                crossfire::TrySendError::Disconnected(value) => TrySendError::Disconnected(value),
            })
        }

        pub(crate) fn is_full(&self) -> bool {
            match &self.inner {
                SenderInner::BoundedSync(sender) => sender.is_full(),
                SenderInner::BoundedAsync { asynchronous, .. } => asynchronous.is_full(),
                SenderInner::Unbounded(sender) => sender.is_full(),
            }
        }
    }

    impl<T> Receiver<T>
    where
        T: Send + Unpin + 'static,
    {
        pub(crate) fn recv(&self) -> Result<T, RecvError> {
            self.inner.recv().map_err(|_| RecvError)
        }

        pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
            self.inner
                .recv_timeout(timeout)
                .map_err(|_| RecvTimeoutError)
        }
    }

    impl<T> AsyncReceiver<T>
    where
        T: Send + Unpin + 'static,
    {
        pub(crate) async fn recv_async(&self) -> Result<T, RecvError> {
            match &self.inner {
                AsyncReceiverInner::Bounded(receiver) => {
                    receiver.recv().await.map_err(|_| RecvError)
                }
                AsyncReceiverInner::Unbounded(receiver) => {
                    receiver.recv().await.map_err(|_| RecvError)
                }
            }
        }

        pub(crate) async fn recv_async_storage(&self) -> Result<T, RecvError> {
            #[cfg(feature = "storage-runtime-kimojio")]
            loop {
                match self.try_recv() {
                    Ok(value) => return Ok(value),
                    Err(TryRecvError::Disconnected) => return Err(RecvError),
                    Err(TryRecvError::Empty) => super::wait_for_storage_channel_poll().await?,
                }
            }

            #[cfg(not(feature = "storage-runtime-kimojio"))]
            self.recv_async().await
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            let result = match &self.inner {
                AsyncReceiverInner::Bounded(receiver) => receiver.try_recv(),
                AsyncReceiverInner::Unbounded(receiver) => receiver.try_recv(),
            };
            result.map_err(|error| match error {
                crossfire::TryRecvError::Empty => TryRecvError::Empty,
                crossfire::TryRecvError::Disconnected => TryRecvError::Disconnected,
            })
        }
    }
}

#[cfg(feature = "channel-flume")]
mod backend {
    use super::TrySendError;
    use super::{
        Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError,
        assert_bounded_capacity,
    };

    pub(crate) struct Sender<T> {
        inner: flume::Sender<T>,
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    pub(crate) struct Receiver<T> {
        inner: flume::Receiver<T>,
    }

    pub(crate) struct AsyncReceiver<T> {
        inner: flume::Receiver<T>,
    }

    pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = flume::bounded(capacity);
        (Sender { inner: sender }, Receiver { inner: receiver })
    }

    pub(crate) fn bounded_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = flume::bounded(capacity);
        (Sender { inner: sender }, AsyncReceiver { inner: receiver })
    }

    pub(crate) fn bounded_sync_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        bounded_async(capacity)
    }

    pub(crate) fn unbounded_async<T>() -> (Sender<T>, AsyncReceiver<T>) {
        let (sender, receiver) = flume::unbounded();
        (Sender { inner: sender }, AsyncReceiver { inner: receiver })
    }

    impl<T> Sender<T> {
        pub(crate) fn send(&self, value: T) -> Result<(), SendError> {
            self.inner.send(value).map_err(|_| SendError)
        }

        pub(crate) async fn send_async(&self, value: T) -> Result<(), SendError> {
            self.inner.send_async(value).await.map_err(|_| SendError)
        }

        pub(crate) fn send_timeout(
            &self,
            value: T,
            timeout: Duration,
        ) -> Result<(), SendTimeoutError> {
            self.inner
                .send_timeout(value, timeout)
                .map_err(|_| SendTimeoutError)
        }

        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            self.inner.try_send(value).map_err(|error| match error {
                flume::TrySendError::Full(value) => TrySendError::Full(value),
                flume::TrySendError::Disconnected(value) => TrySendError::Disconnected(value),
            })
        }

        pub(crate) fn is_full(&self) -> bool {
            self.inner.is_full()
        }
    }

    impl<T> Receiver<T> {
        pub(crate) fn recv(&self) -> Result<T, RecvError> {
            self.inner.recv().map_err(|_| RecvError)
        }

        pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
            self.inner
                .recv_timeout(timeout)
                .map_err(|_| RecvTimeoutError)
        }
    }

    impl<T> AsyncReceiver<T> {
        pub(crate) async fn recv_async(&self) -> Result<T, RecvError> {
            self.inner.recv_async().await.map_err(|_| RecvError)
        }

        pub(crate) async fn recv_async_storage(&self) -> Result<T, RecvError> {
            #[cfg(feature = "storage-runtime-kimojio")]
            loop {
                match self.try_recv() {
                    Ok(value) => return Ok(value),
                    Err(TryRecvError::Disconnected) => return Err(RecvError),
                    Err(TryRecvError::Empty) => super::wait_for_storage_channel_poll().await?,
                }
            }

            #[cfg(not(feature = "storage-runtime-kimojio"))]
            self.recv_async().await
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            self.inner.try_recv().map_err(|error| match error {
                flume::TryRecvError::Empty => TryRecvError::Empty,
                flume::TryRecvError::Disconnected => TryRecvError::Disconnected,
            })
        }
    }
}

#[cfg(feature = "channel-kanal")]
mod backend {
    use super::TrySendError;
    use super::{
        Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError,
        assert_bounded_capacity,
    };
    use event_listener::Event;
    use std::sync::Arc;

    pub(crate) struct Sender<T> {
        inner: kanal::Sender<T>,
        receive_event: Option<Arc<Event>>,
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
                receive_event: self.receive_event.clone(),
            }
        }
    }

    impl<T> Drop for Sender<T> {
        fn drop(&mut self) {
            if let Some(event) = &self.receive_event {
                event.notify(usize::MAX);
            }
        }
    }

    pub(crate) struct Receiver<T> {
        inner: kanal::Receiver<T>,
    }

    // Kanal receive futures can lose a delivered message when select or timeout
    // cancels them. Event-driven try_recv keeps cancellation atomic.
    pub(crate) struct AsyncReceiver<T> {
        inner: kanal::Receiver<T>,
        receive_event: Arc<Event>,
    }

    pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = kanal::bounded(capacity);
        (
            Sender {
                inner: sender,
                receive_event: None,
            },
            Receiver { inner: receiver },
        )
    }

    pub(crate) fn bounded_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        assert_bounded_capacity(capacity);
        let (sender, receiver) = kanal::bounded(capacity);
        let receive_event = Arc::new(Event::new());
        (
            Sender {
                inner: sender,
                receive_event: Some(receive_event.clone()),
            },
            AsyncReceiver {
                inner: receiver,
                receive_event,
            },
        )
    }

    pub(crate) fn bounded_sync_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        bounded_async(capacity)
    }

    pub(crate) fn unbounded_async<T>() -> (Sender<T>, AsyncReceiver<T>) {
        let (sender, receiver) = kanal::unbounded();
        let receive_event = Arc::new(Event::new());
        (
            Sender {
                inner: sender,
                receive_event: Some(receive_event.clone()),
            },
            AsyncReceiver {
                inner: receiver,
                receive_event,
            },
        )
    }

    impl<T> Sender<T> {
        fn notify_receiver(&self) {
            if let Some(event) = &self.receive_event {
                event.notify(1);
            }
        }

        pub(crate) fn send(&self, value: T) -> Result<(), SendError> {
            self.inner.send(value).map_err(|_| SendError)?;
            self.notify_receiver();
            Ok(())
        }

        pub(crate) async fn send_async(&self, value: T) -> Result<(), SendError> {
            self.inner
                .as_async()
                .send(value)
                .await
                .map_err(|_| SendError)?;
            self.notify_receiver();
            Ok(())
        }

        pub(crate) fn send_timeout(
            &self,
            value: T,
            timeout: Duration,
        ) -> Result<(), SendTimeoutError> {
            self.inner
                .send_timeout(value, timeout)
                .map_err(|_| SendTimeoutError)?;
            self.notify_receiver();
            Ok(())
        }

        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            let mut value = Some(value);
            let result = match self.inner.try_send_option(&mut value) {
                Ok(true) => Ok(()),
                Ok(false) => Err(TrySendError::Full(value.unwrap())),
                Err(_) => Err(TrySendError::Disconnected(value.unwrap())),
            };
            if result.is_ok() {
                self.notify_receiver();
            }
            result
        }

        pub(crate) fn is_full(&self) -> bool {
            self.inner.is_full()
        }
    }

    impl<T> Receiver<T> {
        pub(crate) fn recv(&self) -> Result<T, RecvError> {
            self.inner.recv().map_err(|_| RecvError)
        }

        pub(crate) fn recv_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
            self.inner
                .recv_timeout(timeout)
                .map_err(|_| RecvTimeoutError)
        }
    }

    impl<T> AsyncReceiver<T> {
        pub(crate) async fn recv_async(&self) -> Result<T, RecvError> {
            loop {
                let listener = self.receive_event.listen();
                match self.try_recv() {
                    Ok(value) => return Ok(value),
                    Err(TryRecvError::Disconnected) => return Err(RecvError),
                    Err(TryRecvError::Empty) => listener.await,
                }
            }
        }

        pub(crate) async fn recv_async_storage(&self) -> Result<T, RecvError> {
            #[cfg(feature = "storage-runtime-kimojio")]
            loop {
                match self.try_recv() {
                    Ok(value) => return Ok(value),
                    Err(TryRecvError::Disconnected) => return Err(RecvError),
                    Err(TryRecvError::Empty) => super::wait_for_storage_channel_poll().await?,
                }
            }

            #[cfg(not(feature = "storage-runtime-kimojio"))]
            self.recv_async().await
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            self.inner
                .try_recv()
                .map_err(|_| TryRecvError::Disconnected)?
                .ok_or(TryRecvError::Empty)
        }
    }
}

pub(crate) use backend::{
    AsyncReceiver, Receiver, Sender, bounded, bounded_async, bounded_sync_async,
};

impl<T> Sender<T>
where
    T: Send + Unpin + 'static,
{
    /// Sends a value without registering a cross-thread waker on Kimojio.
    pub(crate) async fn send_async_network(&self, value: T) -> Result<(), SendError> {
        #[cfg(not(feature = "network-runtime-kimojio"))]
        {
            return self.send_async(value).await;
        }

        #[cfg(feature = "network-runtime-kimojio")]
        {
            let mut value = value;
            loop {
                match self.try_send(value) {
                    Ok(()) => return Ok(()),
                    Err(TrySendError::Disconnected(_)) => return Err(SendError),
                    Err(TrySendError::Full(returned)) => {
                        value = returned;
                        wait_for_network_channel_poll().await;
                    }
                }
            }
        }
    }
}

impl<T> AsyncReceiver<T>
where
    T: Send + Unpin + 'static,
{
    /// Receives a value without registering a cross-thread waker on Kimojio.
    ///
    /// Kimojio requires every task wake-up to originate on its owning worker.
    /// Polling with the selected runtime's timer preserves that invariant when
    /// a channel sender belongs to another worker or runtime.
    pub(crate) async fn recv_async_network(&self) -> Result<T, RecvError> {
        #[cfg(not(feature = "network-runtime-kimojio"))]
        return self.recv_async().await;

        #[cfg(feature = "network-runtime-kimojio")]
        loop {
            match self.try_recv() {
                Ok(value) => return Ok(value),
                Err(TryRecvError::Disconnected) => return Err(RecvError),
                Err(TryRecvError::Empty) => wait_for_network_channel_poll().await,
            }
        }
    }
}

pub(crate) fn unbounded_async<T>() -> (Sender<T>, AsyncReceiver<T>)
where
    T: Send + Unpin + 'static,
{
    backend::unbounded_async()
}
