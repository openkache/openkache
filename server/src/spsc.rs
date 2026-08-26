use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(align(64))]
/** 자주 갱신되는 원자 값을 독립 캐시 라인에 배치해 false sharing을 줄이는 래퍼다. */
struct CacheLine<T>(/** 다른 캐시 라인과 분리해 보관할 실제 값이다. */ T);

/** 단일 생산자와 단일 소비자가 공유하는 고정 크기 lock-free 링 버퍼다. */
struct Ring<T, const N: usize> {
    /** 원소가 저장되는 슬롯 배열이다. 논리적인 head와 tail 사이의 슬롯만 초기화되어 있다. */
    slots: [UnsafeCell<MaybeUninit<T>>; N],
    /** 소비자가 다음에 읽을 슬롯의 인덱스다. */
    head: CacheLine<AtomicUsize>,
    /** 생산자가 다음에 쓸 슬롯의 인덱스다. */
    tail: CacheLine<AtomicUsize>,
}

// 안전성: 각 슬롯에는 생산자와 소비자가 정확히 하나씩 있습니다. Release로 tail을
// 공개하고 Acquire로 관찰하면 소유권이 이전됩니다.
unsafe impl<T: Send, const N: usize> Sync for Ring<T, N> {}

/** 링 버퍼의 유일한 쓰기 끝점으로, 값을 게시하고 소비자 진행 상태를 캐시한다. */
pub(crate) struct Producer<T, const N: usize> {
    /** 생산자와 소비자가 공동 소유하는 링 버퍼다. */
    ring: Arc<Ring<T, N>>,
    /** 원자 읽기 횟수를 줄이기 위해 마지막으로 관찰한 소비자 head 값이다. */
    head_cache: usize,
}

/** 링 버퍼의 유일한 읽기 끝점으로, 게시된 값을 회수하고 생산자 진행 상태를 캐시한다. */
pub(crate) struct Consumer<T, const N: usize> {
    /** 생산자와 소비자가 공동 소유하는 링 버퍼다. */
    ring: Arc<Ring<T, N>>,
    /** 원자 읽기 횟수를 줄이기 위해 마지막으로 관찰한 생산자 tail 값이다. */
    tail_cache: usize,
}

/** 최소 두 슬롯의 고정 크기 SPSC 링을 만들고 유일한 생산자와 소비자를 반환한다. */
pub(crate) fn channel<T, const N: usize>() -> (Producer<T, N>, Consumer<T, N>) {
    assert!(N >= 2, "an SPSC ring needs at least two slots");
    let ring = Arc::new(Ring {
        slots: std::array::from_fn(|_| UnsafeCell::new(MaybeUninit::uninit())),
        head: CacheLine(AtomicUsize::new(0)),
        tail: CacheLine(AtomicUsize::new(0)),
    });
    (
        Producer {
            ring: Arc::clone(&ring),
            head_cache: 0,
        },
        Consumer {
            ring,
            tail_cache: 0,
        },
    )
}

impl<T, const N: usize> Producer<T, N> {
    /** 캐시된 head를 필요할 때만 갱신해 새 값을 게시할 빈 슬롯이 있는지 확인한다. */
    pub(crate) fn has_capacity(&mut self) -> bool {
        let tail = self.ring.tail.0.load(Ordering::Relaxed);
        let next = (tail + 1) % N;
        if next != self.head_cache {
            return true;
        }
        self.head_cache = self.ring.head.0.load(Ordering::Acquire);
        next != self.head_cache
    }

    /** 값을 다음 슬롯에 게시하고, 링이 가득 찼으면 소유권과 함께 원래 값을 반환한다. */
    pub(crate) fn push(&mut self, value: T) -> Result<(), T> {
        let tail = self.ring.tail.0.load(Ordering::Relaxed);
        let next = (tail + 1) % N;
        if next == self.head_cache {
            self.head_cache = self.ring.head.0.load(Ordering::Acquire);
            if next == self.head_cache {
                return Err(value);
            }
        }

        // 안전성: `tail` 위치의 슬롯은 이 생산자만 쓰며, 소비자는 아래의 Release
        // 저장이 실행되기 전까지 해당 슬롯을 관찰할 수 없습니다.
        unsafe { (*self.ring.slots[tail].get()).write(value) };
        self.ring.tail.0.store(next, Ordering::Release);
        Ok(())
    }
}

impl<T, const N: usize> Consumer<T, N> {
    /** 다음 게시 값을 회수하고 head를 전진시키며, 링이 비었으면 `None`을 반환한다. */
    pub(crate) fn pop(&mut self) -> Option<T> {
        let head = self.ring.head.0.load(Ordering::Relaxed);
        if head == self.tail_cache {
            self.tail_cache = self.ring.tail.0.load(Ordering::Acquire);
            if head == self.tail_cache {
                return None;
            }
        }

        // 안전성: Acquire 읽기가 생산자가 초기화한 슬롯을 관찰했으며, `head`를 읽고
        // 전진시키는 주체는 이 소비자뿐입니다.
        let value = unsafe { (*self.ring.slots[head].get()).assume_init_read() };
        self.ring.head.0.store((head + 1) % N, Ordering::Release);
        Some(value)
    }
}

impl<T, const N: usize> Drop for Ring<T, N> {
    /** 마지막 공유 소유권이 해제될 때 링에 남은 초기화된 원소를 빠짐없이 drop한다. */
    fn drop(&mut self) {
        let mut head = self.head.0.load(Ordering::Relaxed);
        let tail = self.tail.0.load(Ordering::Relaxed);
        while head != tail {
            // 안전성: 마지막 Arc가 해제된 뒤에는 생산자와 소비자가 존재하지 않으며,
            // [head, tail) 범위의 슬롯은 초기화되어 있습니다.
            unsafe { (*self.slots[head].get()).assume_init_drop() };
            head = (head + 1) % N;
        }
    }
}
