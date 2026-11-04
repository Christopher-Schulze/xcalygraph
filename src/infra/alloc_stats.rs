//! Allocation counts per pipeline stage on stderr, only in builds with the `alloc-stats` feature
//! (`scripts/benchmarks/measure-allocations.sh`); `stage` compiles to nothing otherwise.

#[cfg(not(feature = "alloc-stats"))]
#[inline(always)]
pub fn stage(_name: &str) {}

#[cfg(feature = "alloc-stats")]
pub use counting::stage;

#[cfg(feature = "alloc-stats")]
mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static COUNT: AtomicU64 = AtomicU64::new(0);
    static BYTES: AtomicU64 = AtomicU64::new(0);
    static LIVE: AtomicU64 = AtomicU64::new(0);
    static PEAK: AtomicU64 = AtomicU64::new(0);

    struct Counting;

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    fn grow(bytes: u64) {
        BYTES.fetch_add(bytes, Relaxed);
        PEAK.fetch_max(LIVE.fetch_add(bytes, Relaxed) + bytes, Relaxed);
    }

    // SAFETY: every call forwards to the system allocator unchanged; only counters are added.
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            COUNT.fetch_add(1, Relaxed);
            grow(layout.size() as u64);
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            LIVE.fetch_sub(layout.size() as u64, Relaxed);
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            COUNT.fetch_add(1, Relaxed);
            if size > layout.size() {
                grow((size - layout.size()) as u64);
            } else {
                LIVE.fetch_sub((layout.size() - size) as u64, Relaxed);
            }
            unsafe { System.realloc(pointer, layout, size) }
        }
    }

    /// One line per finished stage: allocations and bytes since the previous stage, live heap
    /// now and its peak during the stage.
    pub fn stage(name: &str) {
        let live = LIVE.load(Relaxed);
        eprintln!(
            "alloc-stats\t{name}\t{}\t{}\t{}\t{}",
            COUNT.swap(0, Relaxed),
            BYTES.swap(0, Relaxed),
            live,
            PEAK.swap(live, Relaxed),
        );
    }
}
