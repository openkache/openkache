//! Module declarations for the allocators subsystem.
//!
//! This module contains sub-modules for virtual memory management, low-level
//! OS memory operations, and a compacting slab allocator built on top.

pub mod compacting_slab_allocator;
pub mod memory;
pub mod virtual_page_stack;
