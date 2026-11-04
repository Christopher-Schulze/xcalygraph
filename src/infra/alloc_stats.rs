//! Time per pipeline stage on stderr in builds with the `stage-stats` feature, plus allocation
//! counts with `alloc-stats` (`scripts/benchmarks/measure-allocations.sh`); both functions compile
//! to nothing otherwise. The counters contend across threads, so only `stage-stats` times are
//! comparable.

#[cfg(not(feature = "stage-stats"))]
#[inline(always)]
pub fn stage(_name: &str) {}

#[cfg(not(feature = "stage-stats"))]
#[inline(always)]
pub fn start() {}

#[cfg(feature = "stage-stats")]
pub use counting::{stage, start};

#[cfg(feature = "stage-stats")]
mod counting {
    #[cfg(feature = "alloc-stats")]
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    use std::time::Instant;

    static COUNT: AtomicU64 = AtomicU64::new(0);
    static BYTES: AtomicU64 = AtomicU64::new(0);
    static LIVE: AtomicU64 = AtomicU64::new(0);
    static PEAK: AtomicU64 = AtomicU64::new(0);
    static STAGE_START: Mutex<Option<Instant>> = Mutex::new(None);

    #[cfg(feature = "alloc-stats")]
    struct Counting;

    #[cfg(feature = "alloc-stats")]
    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    #[cfg(feature = "alloc-stats")]
    fn grow(bytes: u64) {
        BYTES.fetch_add(bytes, Relaxed);
        PEAK.fetch_max(LIVE.fetch_add(bytes, Relaxed) + bytes, Relaxed);
    }

    // SAFETY: every call forwards to the system allocator unchanged; only counters are added.
    #[cfg(feature = "alloc-stats")]
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

    /// Marks the start of the first stage.
    pub fn start() {
        if let Ok(mut start) = STAGE_START.lock() {
            *start = Some(Instant::now());
        }
    }

    /// One line per finished stage: allocations and bytes since the previous stage, live heap
    /// now, its peak during the stage and the stage's milliseconds.
    pub fn stage(name: &str) {
        let elapsed = STAGE_START
            .lock()
            .ok()
            .and_then(|mut start| start.replace(Instant::now()))
            .map_or(0, |start| start.elapsed().as_millis());
        let live = LIVE.load(Relaxed);
        eprintln!(
            "alloc-stats\t{name}\t{}\t{}\t{}\t{}\t{elapsed}",
            COUNT.swap(0, Relaxed),
            BYTES.swap(0, Relaxed),
            live,
            PEAK.swap(live, Relaxed),
        );
    }
}
