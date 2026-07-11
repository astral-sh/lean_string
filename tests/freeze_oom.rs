use std::{
    alloc::{GlobalAlloc, Layout, System},
    ptr,
    sync::atomic::{AtomicBool, Ordering},
};

use lean_string::{LeanStr, LeanString, ReserveError};

struct FailNextAllocation;

static FAIL_NEXT_ALLOCATION: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT_REALLOCATION: AtomicBool = AtomicBool::new(false);

// SAFETY: Allocations are delegated to `System`, except for the single allocation explicitly
// rejected by the test.
unsafe impl GlobalAlloc for FailNextAllocation {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if FAIL_NEXT_ALLOCATION.swap(false, Ordering::SeqCst) {
            ptr::null_mut()
        } else {
            // SAFETY: The caller provides a valid layout.
            unsafe { System.alloc(layout) }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was allocated by `System` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if FAIL_NEXT_REALLOCATION.swap(false, Ordering::SeqCst) {
            ptr::null_mut()
        } else {
            // SAFETY: The caller provides a pointer allocated with `layout`, and `new_size` is the
            // requested replacement allocation size.
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: FailNextAllocation = FailNextAllocation;

#[test]
fn fallible_heap_conversions_report_allocation_failure() {
    const TEXT: &str = "a string longer than the inline limit";

    // A unique growable allocation is converted to exact storage with `realloc`.
    let mut string = LeanString::with_capacity(128);
    string.push_str(TEXT);

    FAIL_NEXT_REALLOCATION.store(true, Ordering::SeqCst);
    let result = string.try_freeze();
    FAIL_NEXT_REALLOCATION.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(ReserveError));

    // A shared growable allocation is copied into a new exact allocation.
    let string = LeanString::from(TEXT);
    let growable = string.clone();

    FAIL_NEXT_ALLOCATION.store(true, Ordering::SeqCst);
    let result = string.try_freeze();
    FAIL_NEXT_ALLOCATION.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(ReserveError));
    assert_eq!(growable, TEXT);

    // A unique exact allocation is converted to growable storage with `realloc`.
    let frozen = LeanStr::from(TEXT);

    FAIL_NEXT_REALLOCATION.store(true, Ordering::SeqCst);
    let result = frozen.try_into_lean_string();
    FAIL_NEXT_REALLOCATION.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(ReserveError));

    // A shared exact allocation is copied into a new growable allocation.
    let frozen = LeanStr::from(TEXT);
    let exact = frozen.clone();

    FAIL_NEXT_ALLOCATION.store(true, Ordering::SeqCst);
    let result = frozen.try_into_lean_string();
    FAIL_NEXT_ALLOCATION.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(ReserveError));
    assert_eq!(exact, TEXT);

    // Joining directly into exact storage reports allocation failure.
    FAIL_NEXT_ALLOCATION.store(true, Ordering::SeqCst);
    let result = LeanStr::try_join(&[TEXT, TEXT], ".");
    FAIL_NEXT_ALLOCATION.store(false, Ordering::SeqCst);

    assert_eq!(result, Err(ReserveError));
}
