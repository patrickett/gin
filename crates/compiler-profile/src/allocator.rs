#![allow(unsafe_code)]

use super::{CAPTURES, CURRENT};
use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

pub struct TrackingAllocator<A>(pub A);

pub(super) fn memory() -> Option<(u64, u64)> {
    INSTALLED
        .load(Relaxed)
        .then(|| (LIVE.load(Relaxed), PEAK.load(Relaxed)))
}

fn record(allocated: usize, freed: usize, realloc: bool) {
    INSTALLED.store(true, Relaxed);
    if allocated >= freed {
        let live = LIVE.fetch_add((allocated - freed) as u64, Relaxed) + (allocated - freed) as u64;
        PEAK.fetch_max(live, Relaxed);
    } else {
        LIVE.fetch_sub((freed - allocated) as u64, Relaxed);
    }
    if CAPTURES.load(Relaxed) == 0 {
        return;
    }
    let _ = CURRENT.try_with(|slot| {
        let Ok(active) = slot.try_borrow() else {
            return;
        };
        let Some(active) = active.as_ref() else {
            return;
        };
        let counters = &active.context.recorder.phases[active.context.phase as usize];
        if allocated > 0 {
            counters.allocations.fetch_add(1, Relaxed);
            counters
                .allocated_bytes
                .fetch_add(allocated as u64, Relaxed);
        }
        if freed > 0 {
            counters.frees.fetch_add(1, Relaxed);
            counters.freed_bytes.fetch_add(freed as u64, Relaxed);
        }
        counters
            .reallocations
            .fetch_add(u64::from(realloc), Relaxed);
    });
}

// Safety: every operation delegates its pointer/layout contract unchanged to A.
// Counters never dereference allocations and do not allocate themselves.
unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.0.alloc(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, false);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { self.0.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record(layout.size(), 0, false);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(pointer, layout) };
        record(0, layout.size(), false);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { self.0.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            record(size, layout.size(), true);
        }
        pointer
    }
}
