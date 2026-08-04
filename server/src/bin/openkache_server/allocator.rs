//! Process-wide allocator selection for the OpenKache server binary.

#[cfg(any(
    all(feature = "allocator-system", feature = "allocator-mimalloc"),
    all(feature = "allocator-system", feature = "allocator-jemalloc"),
    all(feature = "allocator-system", feature = "allocator-snmalloc"),
    all(feature = "allocator-mimalloc", feature = "allocator-jemalloc"),
    all(feature = "allocator-mimalloc", feature = "allocator-snmalloc"),
    all(feature = "allocator-jemalloc", feature = "allocator-snmalloc"),
    not(any(
        feature = "allocator-system",
        feature = "allocator-mimalloc",
        feature = "allocator-jemalloc",
        feature = "allocator-snmalloc",
    )),
))]
compile_error!("enable exactly one allocator-* feature");

#[cfg(all(
    feature = "allocator-system",
    not(any(
        feature = "allocator-mimalloc",
        feature = "allocator-jemalloc",
        feature = "allocator-snmalloc",
    )),
))]
#[global_allocator]
static GLOBAL_ALLOCATOR: std::alloc::System = std::alloc::System;

#[cfg(all(
    feature = "allocator-mimalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-jemalloc",
        feature = "allocator-snmalloc",
    )),
))]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(all(
    feature = "allocator-jemalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-mimalloc",
        feature = "allocator-snmalloc",
    )),
))]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(
    feature = "allocator-snmalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-mimalloc",
        feature = "allocator-jemalloc",
    )),
))]
#[global_allocator]
static GLOBAL_ALLOCATOR: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

#[cfg(all(
    feature = "allocator-system",
    not(any(
        feature = "allocator-mimalloc",
        feature = "allocator-jemalloc",
        feature = "allocator-snmalloc",
    )),
))]
pub(crate) const NAME: &str = "system";

#[cfg(all(
    feature = "allocator-mimalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-jemalloc",
        feature = "allocator-snmalloc",
    )),
))]
pub(crate) const NAME: &str = "mimalloc";

#[cfg(all(
    feature = "allocator-jemalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-mimalloc",
        feature = "allocator-snmalloc",
    )),
))]
pub(crate) const NAME: &str = "jemalloc";

#[cfg(all(
    feature = "allocator-snmalloc",
    not(any(
        feature = "allocator-system",
        feature = "allocator-mimalloc",
        feature = "allocator-jemalloc",
    )),
))]
pub(crate) const NAME: &str = "snmalloc";
