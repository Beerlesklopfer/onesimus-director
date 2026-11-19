//! Runtime of a `no_std` ofd plugin guest.
//!
//! Rust's std pulls WASI imports (clocks, streams, polling) into a
//! `wasm32-wasip2` component, and the ofd host refuses every import. A guest
//! is therefore `no_std` and links this crate for what std would provide:
//! a bump allocator over linear memory (the host gives every job its own
//! instance, so memory is reclaimed when the job ends), the canonical ABI
//! entry `cabi_realloc`, `memcmp`/`bcmp` (wasi-libc is not linked), and a
//! panic handler that traps.
//!
//! Use: `extern crate onesimus_guest_runtime as _;` in the guest.

#![no_std]

pub mod heap {
    //! A size-class allocator over linear memory: every block is a power of
    //! two (at least 16 bytes, 16-byte aligned) and a freed block goes on the
    //! free list of its class for the next allocation of that class. A
    //! streaming plugin thus reuses the same few blocks for every chunk
    //! instead of growing until the memory limit (a plain bump allocator
    //! frees only the most recent block, and chunk buffers are freed out of
    //! order). Blocks are never merged; the host gives every job its own
    //! instance, so all memory is reclaimed when the job ends. Alignments
    //! above 16 come from the bump region and are not reused.
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::Cell;

    const PAGE: usize = 65536;
    const MIN_SHIFT: u32 = 4;
    const CLASSES: usize = 28;

    pub struct Heap {
        next: Cell<usize>,
        end: Cell<usize>,
        free: [Cell<usize>; CLASSES],
    }
    // SAFETY: the component runs single-threaded.
    unsafe impl Sync for Heap {}

    /// The class of a block for `size` bytes, None when too large.
    fn class(size: usize) -> Option<usize> {
        let shift = size.max(1 << MIN_SHIFT).next_power_of_two().trailing_zeros();
        let c = (shift - MIN_SHIFT) as usize;
        (c < CLASSES).then_some(c)
    }

    impl Heap {
        /// `size` bytes at `align` from the end of the used region.
        fn bump(&self, size: usize, align: usize) -> *mut u8 {
            if self.next.get() == 0 {
                let pages = core::arch::wasm32::memory_size(0);
                self.next.set(pages * PAGE);
                self.end.set(pages * PAGE);
            }
            let start = (self.next.get() + align - 1) & !(align - 1);
            let Some(end) = start.checked_add(size) else { return core::ptr::null_mut() };
            if end > self.end.get() {
                let need = (end - self.end.get()).div_ceil(PAGE);
                if core::arch::wasm32::memory_grow(0, need) == usize::MAX {
                    return core::ptr::null_mut();
                }
                self.end.set(self.end.get() + need * PAGE);
            }
            self.next.set(end);
            start as *mut u8
        }
    }

    unsafe impl GlobalAlloc for Heap {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if layout.align() > 16 {
                return self.bump(layout.size(), layout.align());
            }
            let Some(c) = class(layout.size()) else { return core::ptr::null_mut() };
            let head = self.free[c].get();
            if head != 0 {
                // SAFETY: a freed block of this class holds the next free block.
                self.free[c].set(unsafe { *(head as *const usize) });
                return head as *mut u8;
            }
            self.bump(1 << (c as u32 + MIN_SHIFT), 16)
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            if layout.align() > 16 || ptr.is_null() {
                return;
            }
            if let Some(c) = class(layout.size()) {
                // SAFETY: the block is ours, at least 16 bytes and 16-byte aligned.
                unsafe { *(ptr as *mut usize) = self.free[c].get() };
                self.free[c].set(ptr as usize);
            }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            if layout.align() <= 16 && class(layout.size()).is_some() && class(layout.size()) == class(new_size) {
                return ptr;
            }
            // SAFETY: as GlobalAlloc::realloc requires.
            unsafe {
                let new = self.alloc(Layout::from_size_align_unchecked(new_size, layout.align()));
                if !new.is_null() {
                    core::ptr::copy_nonoverlapping(ptr, new, layout.size().min(new_size));
                    self.dealloc(ptr, layout);
                }
                new
            }
        }
    }

    #[global_allocator]
    pub static HEAP: Heap = Heap {
        next: Cell::new(0),
        end: Cell::new(0),
        free: [const { Cell::new(0) }; CLASSES],
    };
}

/// memcmp/bcmp: wasi-libc is not linked into a `no_std` component, and the
/// compiler emits calls to them for slice comparisons. A plain byte loop
/// (comparing slices here would call memcmp again).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn memcmp(a: *const core::ffi::c_void, b: *const core::ffi::c_void, n: usize) -> i32 {
    let (a, b) = (a.cast::<u8>(), b.cast::<u8>());
    let mut i = 0;
    while i < n {
        // SAFETY: the caller passes n readable bytes at a and b.
        let (x, y) = unsafe { (*a.add(i), *b.add(i)) };
        if x != y {
            return i32::from(x) - i32::from(y);
        }
        i += 1;
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bcmp(a: *const core::ffi::c_void, b: *const core::ffi::c_void, n: usize) -> i32 {
    // SAFETY: same contract as memcmp.
    unsafe { memcmp(a, b, n) }
}

/// The canonical ABI allocation entry: the host writes request bytes into
/// memory it gets here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cabi_realloc(old: *mut u8, old_len: usize, align: usize, new_len: usize) -> *mut u8 {
    use core::alloc::{GlobalAlloc, Layout};
    if new_len == 0 {
        return align as *mut u8;
    }
    let Ok(layout) = Layout::from_size_align(new_len, align) else { core::arch::wasm32::unreachable() };
    // SAFETY: a fresh allocation, or a reallocation of one this function made
    // with the same alignment.
    let ptr = unsafe {
        if old_len == 0 {
            heap::HEAP.alloc(layout)
        } else {
            heap::HEAP.realloc(old, Layout::from_size_align_unchecked(old_len, align), new_len)
        }
    };
    if ptr.is_null() {
        core::arch::wasm32::unreachable()
    }
    ptr
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

