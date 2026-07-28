//! Process-wide allocator selection for the OpenKache server binary.

#[cfg(any(
    all(feature = "allocator-system", feature = "allocator-mimalloc"),
    all(feature = "allocator-system", feature = "allocator-jemalloc"),
    all(feature = "allocator-system", feature = "allocator-snmalloc"),
    all(feature = "allocator-mimalloc", feature = "allocator-jemalloc"),
    all(feature = "allocator-mimalloc", feature = "allocator-snmalloc"),
    all(feature = "allocator-jemalloc", feature = "allocator-snmalloc"),
))]
compile_error!("enable at most one allocator-* feature");

#[cfg(feature = "allocator-mimalloc")]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "allocator-jemalloc")]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "allocator-snmalloc")]
#[global_allocator]
static GLOBAL_ALLOCATOR: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

#[cfg(not(any(
    feature = "allocator-mimalloc",
    feature = "allocator-jemalloc",
    feature = "allocator-snmalloc",
)))]
#[global_allocator]
static GLOBAL_ALLOCATOR: std::alloc::System = std::alloc::System;

#[cfg(feature = "allocator-mimalloc")]
pub(crate) const NAME: &str = "mimalloc";

#[cfg(feature = "allocator-jemalloc")]
pub(crate) const NAME: &str = "jemalloc";

#[cfg(feature = "allocator-snmalloc")]
pub(crate) const NAME: &str = "snmalloc";

#[cfg(not(any(
    feature = "allocator-mimalloc",
    feature = "allocator-jemalloc",
    feature = "allocator-snmalloc",
)))]
pub(crate) const NAME: &str = "system";
