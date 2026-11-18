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
    //! A bump allocator over linear memory. The host gives every job its own
    //! instance and memory limit, so memory is reclaimed when the job ends;
    //! only the most recent allocation is ever freed in place.
    use core::alloc::{GlobalAlloc, Layout};
    use core::cell::Cell;

    pub struct Bump {
        next: Cell<usize>,
        end: Cell<usize>,
        last: Cell<usize>,
    }
    // SAFETY: the component runs single-threaded.
    unsafe impl Sync for Bump {}

    const PAGE: usize = 65536;

    unsafe impl GlobalAlloc for Bump {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let align = layout.align().max(8);
            if self.next.get() == 0 {
                let pages = core::arch::wasm32::memory_size(0);
                self.next.set(pages * PAGE);
                self.end.set(pages * PAGE);
            }
            let start = (self.next.get() + align - 1) & !(align - 1);
            let Some(end) = start.checked_add(layout.size()) else { return core::ptr::null_mut() };
            if end > self.end.get() {
                let need = (end - self.end.get()).div_ceil(PAGE);
                if core::arch::wasm32::memory_grow(0, need) == usize::MAX {
                    return core::ptr::null_mut();
                }
                self.end.set(self.end.get() + need * PAGE);
            }
            self.last.set(start);
            self.next.set(end);
            start as *mut u8
        }
        unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
            if ptr as usize == self.last.get() {
                self.next.set(self.last.get());
            }
        }
    }

    #[global_allocator]
    pub static HEAP: Bump = Bump { next: Cell::new(0), end: Cell::new(0), last: Cell::new(0) };
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

