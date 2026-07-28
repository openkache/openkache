//! Compile-time-selected channel backend used by the server runtime.

use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub(crate) struct SendError;

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sending on a disconnected channel")
    }
}

impl std::error::Error for SendError {}

#[cfg(feature = "quic-quiche")]
#[derive(Debug)]
pub(crate) enum TrySendError<T> {
    Full(T),
    Disconnected(T),
}

#[cfg(feature = "quic-quiche")]
impl<T> fmt::Display for TrySendError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("sending on a full channel"),
            Self::Disconnected(_) => formatter.write_str("sending on a disconnected channel"),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SendTimeoutError;

impl fmt::Display for SendTimeoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sending on a full or disconnected channel")
    }
}

impl std::error::Error for SendTimeoutError {}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecvError;

impl fmt::Display for RecvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("receiving on an empty and disconnected channel")
    }
}

impl std::error::Error for RecvError {}

#[derive(Clone, Copy, Debug)]
pub(crate) enum TryRecvError {
    Empty,
    Disconnected,
}

impl fmt::Display for TryRecvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("receiving on an empty channel"),
            Self::Disconnected => {
                formatter.write_str("receiving on an empty and disconnected channel")
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RecvTimeoutError;

impl fmt::Display for RecvTimeoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("receiving on an empty or disconnected channel")
    }
}

impl std::error::Error for RecvTimeoutError {}

#[cfg(feature = "channel-crossfire")]
mod backend {
    #[cfg(feature = "quic-quiche")]
    use super::TrySendError;
    use super::{Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError};
    use crossfire::{AsyncRx, MAsyncTx, MTx, Rx, mpsc};

    enum SenderInner<T: 'static> {
        BoundedSync(MTx<mpsc::Array<T>>),
        BoundedAsync {
            blocking: MTx<mpsc::Array<T>>,
            asynchronous: MAsyncTx<mpsc::Array<T>>,
        },
        #[cfg(feature = "quic-quiche")]
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
                #[cfg(feature = "quic-quiche")]
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
        #[cfg(feature = "quic-quiche")]
        Unbounded(AsyncRx<mpsc::List<T>>),
    }

    pub(crate) struct AsyncReceiver<T: 'static> {
        inner: AsyncReceiverInner<T>,
    }

    pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>)
    where
        T: Send + Unpin + 'static,
    {
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

    #[cfg(feature = "quic-quiche")]
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
                #[cfg(feature = "quic-quiche")]
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
                #[cfg(feature = "quic-quiche")]
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
                #[cfg(feature = "quic-quiche")]
                SenderInner::Unbounded(sender) => {
                    return sender.send(value).map_err(|_| SendTimeoutError);
                }
            };
            result.map_err(|_| SendTimeoutError)
        }

        #[cfg(feature = "quic-quiche")]
        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            let result = match &self.inner {
                SenderInner::BoundedSync(sender) => sender.try_send(value),
                SenderInner::BoundedAsync { asynchronous, .. } => asynchronous.try_send(value),
                #[cfg(feature = "quic-quiche")]
                SenderInner::Unbounded(sender) => sender.try_send(value),
            };
            result.map_err(|error| match error {
                crossfire::TrySendError::Full(value) => TrySendError::Full(value),
                crossfire::TrySendError::Disconnected(value) => TrySendError::Disconnected(value),
            })
        }

        #[cfg(feature = "quic-quiche")]
        pub(crate) fn is_full(&self) -> bool {
            match &self.inner {
                SenderInner::BoundedSync(sender) => sender.is_full(),
                SenderInner::BoundedAsync { asynchronous, .. } => asynchronous.is_full(),
                #[cfg(feature = "quic-quiche")]
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
                #[cfg(feature = "quic-quiche")]
                AsyncReceiverInner::Unbounded(receiver) => {
                    receiver.recv().await.map_err(|_| RecvError)
                }
            }
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            let result = match &self.inner {
                AsyncReceiverInner::Bounded(receiver) => receiver.try_recv(),
                #[cfg(feature = "quic-quiche")]
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
    #[cfg(feature = "quic-quiche")]
    use super::TrySendError;
    use super::{Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError};

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
        let (sender, receiver) = flume::bounded(capacity);
        (Sender { inner: sender }, Receiver { inner: receiver })
    }

    pub(crate) fn bounded_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        let (sender, receiver) = flume::bounded(capacity);
        (Sender { inner: sender }, AsyncReceiver { inner: receiver })
    }

    pub(crate) fn bounded_sync_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        bounded_async(capacity)
    }

    #[cfg(feature = "quic-quiche")]
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

        #[cfg(feature = "quic-quiche")]
        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            self.inner.try_send(value).map_err(|error| match error {
                flume::TrySendError::Full(value) => TrySendError::Full(value),
                flume::TrySendError::Disconnected(value) => TrySendError::Disconnected(value),
            })
        }

        #[cfg(feature = "quic-quiche")]
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
    #[cfg(feature = "quic-quiche")]
    use super::TrySendError;
    use super::{Duration, RecvError, RecvTimeoutError, SendError, SendTimeoutError, TryRecvError};

    pub(crate) struct Sender<T> {
        inner: kanal::Sender<T>,
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }

    pub(crate) struct Receiver<T> {
        inner: kanal::Receiver<T>,
    }

    pub(crate) struct AsyncReceiver<T> {
        inner: kanal::Receiver<T>,
    }

    pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
        let (sender, receiver) = kanal::bounded(capacity);
        (Sender { inner: sender }, Receiver { inner: receiver })
    }

    pub(crate) fn bounded_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        let (sender, receiver) = kanal::bounded(capacity);
        (Sender { inner: sender }, AsyncReceiver { inner: receiver })
    }

    pub(crate) fn bounded_sync_async<T>(capacity: usize) -> (Sender<T>, AsyncReceiver<T>) {
        bounded_async(capacity)
    }

    #[cfg(feature = "quic-quiche")]
    pub(crate) fn unbounded_async<T>() -> (Sender<T>, AsyncReceiver<T>) {
        let (sender, receiver) = kanal::unbounded();
        (Sender { inner: sender }, AsyncReceiver { inner: receiver })
    }

    impl<T> Sender<T> {
        pub(crate) fn send(&self, value: T) -> Result<(), SendError> {
            self.inner.send(value).map_err(|_| SendError)
        }

        pub(crate) async fn send_async(&self, value: T) -> Result<(), SendError> {
            self.inner
                .as_async()
                .send(value)
                .await
                .map_err(|_| SendError)
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

        #[cfg(feature = "quic-quiche")]
        pub(crate) fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            let mut value = Some(value);
            match self.inner.try_send_option(&mut value) {
                Ok(true) => Ok(()),
                Ok(false) => Err(TrySendError::Full(value.unwrap())),
                Err(_) => Err(TrySendError::Disconnected(value.unwrap())),
            }
        }

        #[cfg(feature = "quic-quiche")]
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
            self.inner.as_async().recv().await.map_err(|_| RecvError)
        }

        pub(crate) fn try_recv(&self) -> Result<T, TryRecvError> {
            self.inner
                .try_recv()
                .map_err(|_| TryRecvError::Disconnected)?
                .ok_or(TryRecvError::Empty)
        }
    }
}

#[cfg(feature = "quic-quiche")]
pub(crate) use backend::unbounded_async;
pub(crate) use backend::{
    AsyncReceiver, Receiver, Sender, bounded, bounded_async, bounded_sync_async,
};
