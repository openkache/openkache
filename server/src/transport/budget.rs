//! FIFO byte-budget admission for request and response buffers.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use super::StreamReadError;

/// Byte-weighted memory budget shared by every connection on one network worker.
#[derive(Clone)]
pub(crate) struct RequestBudget {
    inner: Rc<RefCell<RequestBudgetState>>,
}

struct RequestBudgetState {
    capacity: usize,
    used: usize,
    next_waiter_id: u64,
    waiters: VecDeque<RequestBudgetWaiter>,
}

struct RequestBudgetWaiter {
    id: u64,
    bytes: usize,
    waker: Waker,
}

pub(crate) struct RequestBudgetPermit {
    inner: Rc<RefCell<RequestBudgetState>>,
    bytes: usize,
}

struct RequestBudgetAcquire {
    inner: Rc<RefCell<RequestBudgetState>>,
    bytes: usize,
    waiter_id: Option<u64>,
}

impl RequestBudget {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: Rc::new(RefCell::new(RequestBudgetState {
                capacity,
                used: 0,
                next_waiter_id: 0,
                waiters: VecDeque::new(),
            })),
        }
    }

    pub(crate) async fn acquire(
        &self,
        bytes: usize,
        timeout: Duration,
    ) -> Result<RequestBudgetPermit, StreamReadError> {
        if bytes == 0 {
            return Ok(RequestBudgetPermit {
                inner: Rc::clone(&self.inner),
                bytes: 0,
            });
        }
        if bytes > self.inner.borrow().capacity {
            return Err(StreamReadError::TooLarge);
        }
        compio::runtime::time::timeout(
            timeout,
            RequestBudgetAcquire {
                inner: Rc::clone(&self.inner),
                bytes,
                waiter_id: None,
            },
        )
        .await
        .map_err(|_| StreamReadError::Timeout)
    }
}

impl Future for RequestBudgetAcquire {
    type Output = RequestBudgetPermit;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = Rc::clone(&self.inner);
        let bytes = self.bytes;
        let mut state = inner.borrow_mut();
        if state.waiters.is_empty() && state.used <= state.capacity - bytes {
            state.used += bytes;
            return Poll::Ready(RequestBudgetPermit {
                inner: Rc::clone(&inner),
                bytes,
            });
        }

        if let Some(waiter_id) = self.waiter_id {
            let position = state
                .waiters
                .iter()
                .position(|waiter| waiter.id == waiter_id)
                .expect("registered request budget waiter remains queued");
            if position == 0 && state.used <= state.capacity - bytes {
                let waiter = state
                    .waiters
                    .pop_front()
                    .expect("front request budget waiter remains queued");
                debug_assert_eq!(waiter.id, waiter_id);
                state.used += bytes;
                self.waiter_id = None;
                let next = state
                    .waiters
                    .front()
                    .filter(|next| state.used <= state.capacity - next.bytes)
                    .map(|next| next.waker.clone());
                drop(state);
                if let Some(next) = next {
                    next.wake();
                }
                return Poll::Ready(RequestBudgetPermit {
                    inner: Rc::clone(&inner),
                    bytes,
                });
            }
            let waiter = &mut state.waiters[position];
            if !waiter.waker.will_wake(context.waker()) {
                waiter.waker.clone_from(context.waker());
            }
            return Poll::Pending;
        }

        let waiter_id = state.next_waiter_id;
        state.next_waiter_id = state
            .next_waiter_id
            .checked_add(1)
            .expect("request budget waiter identifier overflowed");
        state.waiters.push_back(RequestBudgetWaiter {
            id: waiter_id,
            bytes,
            waker: context.waker().clone(),
        });
        drop(state);
        self.waiter_id = Some(waiter_id);
        Poll::Pending
    }
}

impl Drop for RequestBudgetAcquire {
    fn drop(&mut self) {
        if let Some(waiter_id) = self.waiter_id {
            let wake = {
                let mut state = self.inner.borrow_mut();
                let position = state
                    .waiters
                    .iter()
                    .position(|waiter| waiter.id == waiter_id);
                let was_front = position == Some(0);
                if let Some(position) = position {
                    state.waiters.remove(position);
                }
                was_front
                    .then(|| state.waiters.front())
                    .flatten()
                    .filter(|next| state.used <= state.capacity - next.bytes)
                    .map(|next| next.waker.clone())
            };
            if let Some(wake) = wake {
                wake.wake();
            }
        }
    }
}

impl Drop for RequestBudgetPermit {
    fn drop(&mut self) {
        if self.bytes == 0 {
            return;
        }
        let wake = {
            let mut state = self.inner.borrow_mut();
            state.used = state
                .used
                .checked_sub(self.bytes)
                .expect("released request bytes must be reserved");
            state
                .waiters
                .front()
                .filter(|next| state.used <= state.capacity - next.bytes)
                .map(|next| next.waker.clone())
        };
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
