use super::*;
use alloc::alloc::{alloc, dealloc, realloc};
use core::{alloc::Layout, hint, ptr, ptr::NonNull};

#[cfg(all(not(loom), target_pointer_width = "64"))]
use core::sync::atomic::AtomicU32;
#[cfg(not(loom))]
use core::sync::atomic::AtomicUsize;
#[cfg(all(loom, target_pointer_width = "64"))]
use loom::sync::atomic::AtomicU32;
#[cfg(loom)]
use loom::sync::atomic::AtomicUsize;

use internal::*;

/// [`HeapBuffer`] grows at an amortized rates of 1.5x
#[inline(always)]
pub(crate) fn amortized_growth(cur_len: usize, additional: usize) -> usize {
    let required = cur_len.saturating_add(additional);
    let amortized = cur_len.saturating_mul(3) / 2;
    amortized.max(required)
}

#[repr(C)]
pub(super) struct HeapBuffer {
    // Exact buffers omit the capacity because it is equal to their length:
    // | ExactHeader | Data (array of `u8`) |
    //                 ^ ptr
    //
    // On 64-bit architectures, ExactHeader contains a four-byte reference count. The allocation
    // remains word-aligned, but the byte-aligned string data starts immediately after the header.
    //
    // Growable buffers retain a capacity:
    // | Header | Data (array of `u8`) |
    //          ^ ptr
    //
    // On 32-bit architectures, buffers whose allocation can exceed the inline length limit
    // prepend a `usize` containing the length.
    ptr: NonNull<u8>,
    len: TextLen,
}

struct ExactHeader {
    count: ExactAtomic,
}

#[cfg(target_pointer_width = "64")]
type ExactAtomic = AtomicU32;
#[cfg(target_pointer_width = "32")]
type ExactAtomic = AtomicUsize;

struct Header {
    capacity: Capacity,
    count: AtomicUsize,
}

const _: () = {
    assert!(size_of::<HeapBuffer>() == MAX_INLINE_SIZE);
    assert!(align_of::<HeapBuffer>() == align_of::<usize>());

    #[cfg(all(target_pointer_width = "64", not(loom)))]
    assert!(size_of::<ExactHeader>() == size_of::<u32>());
    #[cfg(all(target_pointer_width = "32", not(loom)))]
    assert!(size_of::<ExactHeader>() == size_of::<usize>());
};

impl HeapBuffer {
    pub(super) fn new(text: &str) -> Result<Self, ReserveError> {
        HeapBuffer::with_exact_capacity(text, text.len())
    }

    pub(super) fn new_exact(text: &str) -> Result<Self, ReserveError> {
        let text_len = text.len();

        let len = TextLen::new_exact(text_len)?;
        let ptr = HeapBuffer::allocate_exact_ptr(text_len)?;

        if len.is_heap() {
            // SAFETY: `allocate_exact_ptr` reserved space for the heap-stored length.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::exact_header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), text_len);
            }
        }

        // SAFETY:
        // - src (`text`) and dst (`ptr`) is valid for `text_len` bytes because `text_len` comes
        //   from `text`, and `ptr` was allocated to be at least that length.
        // - Both src and dst is aligned for u8.
        // - src and dst don't overlap because we allocated dst just now.
        unsafe { ptr::copy_nonoverlapping(text.as_ptr(), ptr.as_ptr(), text_len) };

        Ok(HeapBuffer { ptr, len })
    }

    pub(crate) fn with_capacity(capacity: usize) -> Result<Self, ReserveError> {
        let len = TextLen::new_growable(0)?;
        let cap = Capacity::new(capacity)?;
        let ptr = HeapBuffer::allocate_growable_ptr(cap)?;
        Ok(HeapBuffer { ptr, len })
    }

    pub(super) fn with_exact_capacity(text: &str, capacity: usize) -> Result<Self, ReserveError> {
        if text.len() > capacity {
            return Err(ReserveError);
        }

        let mut buffer = HeapBuffer::with_capacity(capacity)?;

        // SAFETY:
        // - `buffer` is uniquely owned and has enough capacity for `text`.
        // - `text` contains valid UTF-8 and does not overlap the new allocation.
        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), buffer.ptr.as_ptr(), text.len());
            buffer.set_len(text.len());
        }

        Ok(buffer)
    }

    pub(super) fn with_additional(text: &str, additional: usize) -> Result<Self, ReserveError> {
        let text_len = text.len();

        let len = TextLen::new_growable(text_len)?;
        let ptr = {
            let new_capacity = Capacity::new(amortized_growth(text_len, additional))?;
            HeapBuffer::allocate_growable_ptr(new_capacity)?
        };

        if len.is_heap() {
            // SAFETY: Since the `new_capacity` is greater than or equal to `text_len`, `ptr` is
            // allocated with enough space to store the length.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::growable_header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), text_len);
            }
        }

        // SAFETY:
        // - src (`text`) and dst (`ptr`) is valid for `text_len` bytes because `text_len` comes
        //   from `text`, and `ptr` was allocated to be at least `new_capacity` bytes, which is
        //   greater than `text_len`.
        // - Both src and dst is aligned for u8.
        // - src and dst don't overlap because we allocated dst just now.
        unsafe { ptr::copy_nonoverlapping(text.as_ptr(), ptr.as_ptr(), text_len) };

        Ok(HeapBuffer { ptr, len })
    }

    pub(super) fn capacity(&self) -> usize {
        if self.is_exact() { self.len() } else { self.header().capacity.as_usize() }
    }

    pub(super) const fn is_exact(&self) -> bool {
        self.len.is_exact()
    }

    pub(crate) fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    pub(super) const fn len(&self) -> usize {
        if self.len.is_heap() {
            // SAFETY: The allocation includes a heap-stored length immediately before its header.
            unsafe {
                let len_ptr = self.ptr.sub(self.header_offset()).sub(size_of::<usize>());
                ptr::read(len_ptr.as_ptr().cast())
            }
        } else {
            self.len.as_usize()
        }
    }

    pub(super) fn as_str(&self) -> &str {
        let len = self.len();
        let ptr = self.ptr.as_ptr();
        // SAFETY: HeapBuffer contains valid `len` bytes of UTF-8 string.
        unsafe { core::str::from_utf8_unchecked(slice::from_raw_parts(ptr, len)) }
    }

    /// # Safety
    /// - The buffer must be unique. (HeapBuffer::is_unique() == true)
    /// - The buffer must be growable. (`HeapBuffer::is_exact() == false`)
    /// - `new_capacity` must be greater than or equal to the current string length.
    pub(super) unsafe fn realloc(&mut self, new_capacity: usize) -> Result<(), ReserveError> {
        debug_assert!(!self.is_exact());
        debug_assert!(self.is_unique());
        debug_assert!(self.len() <= new_capacity);

        let new_capacity = Capacity::new(new_capacity)?;
        let cur_capacity = self.header().capacity;

        let cur_layout = match HeapBuffer::layout_from_capacity(cur_capacity) {
            Ok(layout) => layout,
            Err(_) => {
                if cfg!(debug_assertions) {
                    panic!("invalid layout, unexpected `capacity` modification may have occurred");
                }
                // SAFETY:
                // `layout_from_capacity` should not return `Err` because this layout should not
                // have been changed since it was used in the previous allocation.
                unsafe { hint::unreachable_unchecked() }
            }
        };

        let len_heap = match (is_len_heap_layout(cur_capacity), is_len_heap_layout(new_capacity)) {
            (false, false) => false,
            (true, true) => true,
            (true, false) | (false, true) => {
                let str = self.as_str();
                let mut new_buf = HeapBuffer::with_capacity(new_capacity.as_usize())?;
                unsafe {
                    ptr::copy_nonoverlapping(str.as_ptr(), new_buf.ptr.as_ptr(), str.len());
                    new_buf.set_len(str.len());
                    self.dealloc();
                }
                *self = new_buf;
                return Ok(());
            }
        };

        let new_alloc_size = {
            #[cfg(target_pointer_width = "64")]
            {
                // Since The maximum size of `capacity` is limited to 2^56 - 1, we no longer need
                // to check for overflow when rounding up to the nearest multiple of alignment.
                size_of::<Header>().wrapping_add(new_capacity.as_usize())
            }
            #[cfg(target_pointer_width = "32")]
            {
                const ALLOC_LIMIT: usize = (isize::MAX as usize + 1) - HeapBuffer::align();
                let mut alloc_size = size_of::<Header>().saturating_add(new_capacity.as_usize());
                if len_heap {
                    alloc_size = alloc_size.saturating_add(size_of::<usize>());
                }
                if alloc_size > ALLOC_LIMIT {
                    return Err(ReserveError);
                }
                alloc_size
            }
        };

        // SAFETY:
        // - `self.allocation()` is already allocated by global allocator.
        // - current allocation is allocated by `cur_layout`.
        // - `new_alloc_size` is greater than zero.
        // - `new_alloc_size` is ensured not to overflow when rounded up to the nearest multiple of
        //    alignment.
        let mut allocation = unsafe { realloc(self.allocation(), cur_layout, new_alloc_size) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        if len_heap {
            // SAFETY: `allocation` is non-null.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        // SAFETY:
        // - `allocation` is non-null.
        // - the allocation size is larger than or equal to the size of Header.
        unsafe {
            ptr::write(
                allocation.cast(),
                Header {
                    capacity: new_capacity,
                    count: AtomicUsize::new(1), // is_unique() is true.
                },
            );
            let ptr = allocation.add(HeapBuffer::growable_header_offset());
            self.ptr = NonNull::new_unchecked(ptr);
        }
        Ok(())
    }

    /// Decrements the reference count. If this was the last reference, deallocates the buffer.
    ///
    /// # Safety
    ///
    /// - `self` must represent a live, counted reference to the allocation, so the reference count
    ///   must be nonzero.
    /// - After calling this method, `self` must not be accessed. The caller is responsible for
    ///   overwriting `self` or ensuring no further use occurs.
    pub(super) unsafe fn release(&mut self) {
        // Same as `Arc::drop`: `fetch_sub(1, Release)` ensures all prior accesses from other
        // threads are visible before we might deallocate.
        let is_last = if self.is_exact() {
            self.exact_header().count.fetch_sub(1, Release) == 1
        } else {
            self.header().count.fetch_sub(1, Release) == 1
        };
        if is_last {
            // And the `Acquire` fence ensures we see all writes before freeing the memory.
            fence(Acquire);

            // SAFETY: The old value of `fetch_sub` was `1`, so now it is `0`. no other references exist.
            unsafe { self.dealloc() };
        }
    }

    /// # Safety
    ///
    /// - No other references to the allocation may exist.
    /// - After deallocation, neither the fields of `self` nor any pointers or references derived
    ///   from them may be read or otherwise accessed. The `HeapBuffer` value itself may only be
    ///   immediately overwritten or forgotten.
    unsafe fn dealloc(&mut self) {
        let layout = match if self.is_exact() {
            HeapBuffer::layout_from_len(self.len())
        } else {
            HeapBuffer::layout_from_capacity(self.header().capacity)
        } {
            Ok(layout) => layout,
            Err(_) => {
                if cfg!(debug_assertions) {
                    panic!("invalid layout, unexpected `capacity` modification may have occurred");
                }
                // SAFETY:
                // `layout_from_capacity` should not return `Err` because this layout should not
                // have been changed since it was used in the previous allocation.
                unsafe { hint::unreachable_unchecked() }
            }
        };
        unsafe {
            dealloc(self.allocation(), layout);
        }
    }

    pub(super) fn is_unique(&self) -> bool {
        if self.is_exact() {
            self.exact_header().count.load(Acquire) == 1
        } else {
            self.header().count.load(Acquire) == 1
        }
    }

    pub(super) fn is_len_on_heap(&self) -> bool {
        self.len.is_heap()
    }

    /// Increments the reference count, returning whether the conservative overflow threshold was
    /// exceeded.
    pub(super) fn increment_reference_count(&self) -> bool {
        if self.is_exact() {
            let previous = self.exact_header().count.fetch_add(1, Relaxed);

            #[cfg(target_pointer_width = "64")]
            {
                previous > i32::MAX as u32
            }
            #[cfg(target_pointer_width = "32")]
            {
                previous > isize::MAX as usize
            }
        } else {
            self.header().count.fetch_add(1, Relaxed) > isize::MAX as usize
        }
    }

    /// # Safety
    /// - `len` bytes in the buffer must be valid UTF-8.
    /// - `len` must be less than or equal to the capacity.
    /// - The buffer must be growable. (`HeapBuffer::is_exact() == false`)
    /// - If `len` is stored on the heap, the buffer must be unique.
    pub(super) unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(!self.is_exact());
        debug_assert!(len <= self.capacity());

        let new_len = match TextLen::new_growable(len) {
            Ok(len) => len,
            Err(_) => {
                if cfg!(debug_assertions) {
                    panic!("Invalid `set_len` call");
                }
                // SAFETY: `TextSize::new` should not return `Err` because `len` bytes are allocated
                // as a valid UTF-8 string buffer.
                unsafe { hint::unreachable_unchecked() }
            }
        };
        debug_assert!(if new_len.is_heap() { self.is_unique() } else { true });
        self.len = new_len;

        #[cold]
        fn write_len_on_heap(ptr: NonNull<u8>, len: usize) {
            // SAFETY: We just checked that `len` is stored on the heap.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::growable_header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), len);
            }
        }
        if self.len.is_heap() {
            write_len_on_heap(self.ptr, len);
        }
    }

    fn allocate_exact_ptr(len: usize) -> Result<NonNull<u8>, ReserveError> {
        let layout = HeapBuffer::layout_from_len(len)?;

        // SAFETY: layout is non-zero.
        let mut allocation = unsafe { alloc(layout) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        if is_len_heap_layout(Capacity::new(len)?) {
            // SAFETY: The layout reserved a leading word for the length.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        // SAFETY: The allocation includes an `ExactHeader` followed by `len` data bytes.
        unsafe {
            ptr::write(allocation.cast(), ExactHeader { count: ExactAtomic::new(1) });
            let ptr = allocation.add(HeapBuffer::exact_header_offset());
            Ok(NonNull::new_unchecked(ptr))
        }
    }

    fn allocate_growable_ptr(capacity: Capacity) -> Result<NonNull<u8>, ReserveError> {
        let layout = HeapBuffer::layout_from_capacity(capacity)?;

        // SAFETY: layout is non-zero.
        let mut allocation = unsafe { alloc(layout) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        if is_len_heap_layout(capacity) {
            // SAFETY:
            // - `allocation` is non-null.
            // - Since `layout` is created with the `capacity` and `is_len_heap_layout` is true for
            // same `capacity`, we know that we reserved space for the length on the heap.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        // SAFETY:
        // - allocation is non-null.
        // - allocation size is larger than or equal to the size of Header.
        unsafe {
            ptr::write(allocation.cast(), Header { capacity, count: AtomicUsize::new(1) });
            let ptr = allocation.add(HeapBuffer::growable_header_offset());
            Ok(NonNull::new_unchecked(ptr))
        }
    }

    fn layout_from_len(len: usize) -> Result<Layout, ReserveError> {
        let capacity = Capacity::new(len)?;
        HeapBuffer::layout(HeapBuffer::exact_header_offset(), capacity)
    }

    fn layout_from_capacity(capacity: Capacity) -> Result<Layout, ReserveError> {
        HeapBuffer::layout(HeapBuffer::growable_header_offset(), capacity)
    }

    fn layout(header_size: usize, capacity: Capacity) -> Result<Layout, ReserveError> {
        let alloc_size = header_size
            .checked_add(capacity.as_usize())
            .and_then(|size| {
                if is_len_heap_layout(capacity) {
                    size.checked_add(size_of::<usize>())
                } else {
                    Some(size)
                }
            })
            .ok_or(ReserveError)?;
        let align = HeapBuffer::align();
        Layout::from_size_align(alloc_size, align).map_err(
            #[cold]
            |_| ReserveError,
        )
    }

    unsafe fn allocation(&self) -> *mut u8 {
        unsafe {
            if self.has_heap_len_layout() {
                cold_path();
                self.ptr.as_ptr().cast::<u8>().sub(self.header_offset()).sub(size_of::<usize>())
            } else {
                self.ptr.as_ptr().cast::<u8>().sub(self.header_offset())
            }
        }
    }

    fn header(&self) -> &Header {
        debug_assert!(!self.is_exact());
        unsafe { &*self.ptr.as_ptr().sub(HeapBuffer::growable_header_offset()).cast() }
    }

    fn exact_header(&self) -> &ExactHeader {
        debug_assert!(self.is_exact());
        unsafe { &*self.ptr.as_ptr().sub(HeapBuffer::exact_header_offset()).cast() }
    }

    fn has_heap_len_layout(&self) -> bool {
        if self.is_exact() {
            self.len.is_heap()
        } else {
            is_len_heap_layout(self.header().capacity)
        }
    }

    const fn align() -> usize {
        const {
            assert!(align_of::<Header>() == align_of::<usize>());
            assert!(align_of::<NonNull<u8>>() == align_of::<usize>());
        }
        align_of::<usize>()
    }

    const fn exact_header_offset() -> usize {
        size_of::<ExactHeader>()
    }

    const fn growable_header_offset() -> usize {
        max(size_of::<Header>(), HeapBuffer::align())
    }

    const fn header_offset(&self) -> usize {
        if self.is_exact() {
            HeapBuffer::exact_header_offset()
        } else {
            HeapBuffer::growable_header_offset()
        }
    }
}

/// const version of `std::cmp::max::<usize>(x, y)`.
const fn max(x: usize, y: usize) -> usize {
    if x > y { x } else { y }
}

mod internal {
    use super::*;

    /// The length of a [`HeapBuffer`].
    ///
    /// An unsinged integer that uses `size_of::<usize>() - 1` bytes, and the rest 1 byte is used
    /// as a tag.
    ///
    /// Internally, the integer is stored in little-endian order, so the memory layout is like:
    ///
    /// +--------------------------------+--------+
    /// |        unsinged integer        |   tag  |
    /// | (size_of::<usize>() - 1) bytes | 1 byte |
    /// +--------------------------------+--------+
    ///
    /// The tag distinguishes exact buffers from growable buffers.
    ///
    /// In this representation, the max value is limited to:
    ///
    /// - (on 64-bit architecture) 2^56 - 1 = 72057594037927935 = 64 PiB
    /// - (on 32-bit architecture) 2^24 - 2 = 16777214          ≈ 16 MiB
    ///
    /// Practically speaking, on 64-bit architecture, this max value is enough for the
    /// length/capacity of a HeapBuffer. However, it is not enough for 32-bit architectures, and if
    /// more than 3 bytes are needed, the length/capacity must be switched to be stored using the
    /// heap. Therefore, on 32-bit architecture, we use 2^24 - 2 as the maximum value, and 2^24 - 1
    /// as the tag that indicates the length/capacity is stored in the heap.
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct TextLen(usize);

    const USIZE_SIZE: usize = size_of::<usize>();

    const MAX_LEN: usize = {
        let mut bytes = [255; USIZE_SIZE];
        bytes[USIZE_SIZE - 1] = 0;
        usize::from_le_bytes(bytes) - if cfg!(target_pointer_width = "32") { 1 } else { 0 }
    };

    impl TextLen {
        const EXACT_TAG: usize = {
            let mut bytes = [0; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::ExactHeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        const GROWABLE_TAG: usize = {
            let mut bytes = [0; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        #[cfg(target_pointer_width = "32")]
        const EXACT_ON_THE_HEAP: usize = {
            let mut bytes = [255; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::ExactHeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        #[cfg(target_pointer_width = "32")]
        const GROWABLE_ON_THE_HEAP: usize = {
            let mut bytes = [255; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        pub(super) const fn new_exact(size: usize) -> Result<Self, ReserveError> {
            Self::new(size, Self::EXACT_TAG)
        }

        pub(super) const fn new_growable(size: usize) -> Result<Self, ReserveError> {
            Self::new(size, Self::GROWABLE_TAG)
        }

        const fn new(size: usize, tag: usize) -> Result<Self, ReserveError> {
            if size > MAX_LEN {
                #[cfg(target_pointer_width = "64")]
                return Err(ReserveError);
                #[cfg(target_pointer_width = "32")]
                return Ok(TextLen(if tag == Self::EXACT_TAG {
                    Self::EXACT_ON_THE_HEAP
                } else {
                    Self::GROWABLE_ON_THE_HEAP
                }));
            }
            Ok(TextLen(size.to_le() | tag))
        }

        pub(super) const fn is_exact(&self) -> bool {
            self.tag() == Self::EXACT_TAG
        }

        #[inline(always)]
        pub(super) const fn is_heap(&self) -> bool {
            #[cfg(target_pointer_width = "64")]
            return false;
            #[cfg(target_pointer_width = "32")]
            return self.0
                == if self.is_exact() {
                    Self::EXACT_ON_THE_HEAP
                } else {
                    Self::GROWABLE_ON_THE_HEAP
                };
        }

        pub(super) const fn as_usize(self) -> usize {
            let size = self.0 ^ self.tag();
            let bytes = size.to_ne_bytes();
            usize::from_le_bytes(bytes)
        }

        const fn tag(&self) -> usize {
            let mut bytes = [0; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = self.0.to_ne_bytes()[USIZE_SIZE - 1];
            usize::from_ne_bytes(bytes)
        }
    }

    #[cfg_attr(target_pointer_width = "64", allow(unused_variables))]
    #[inline(always)]
    pub(super) fn is_len_heap_layout(capacity: Capacity) -> bool {
        #[cfg(target_pointer_width = "64")]
        return false;
        #[cfg(target_pointer_width = "32")]
        return capacity.as_usize() > MAX_LEN;
    }

    /// The capacity of a [`HeapBuffer`].
    ///
    /// Maximum capacity is limited to:
    ///
    /// - (on 64-bit architecture) 2^56 - 1
    /// - (on 32-bit architecture) 2^32 - 1
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    pub(super) struct Capacity(usize);

    impl Capacity {
        pub(crate) fn new(capacity: usize) -> Result<Self, ReserveError> {
            #[cfg(target_pointer_width = "64")]
            if capacity > MAX_LEN {
                cold_path();
                return Err(ReserveError);
            }
            Ok(Capacity(capacity))
        }

        pub(crate) fn as_usize(&self) -> usize {
            self.0
        }
    }

    // TODO: Replace with hint::cold_path when it becomes stable.
    // Related issues:
    // - https://github.com/rust-lang/rust/issues/26179
    // - https://github.com/rust-lang/rust/pull/120370
    // - https://github.com/rust-lang/libs-team/issues/510
    #[cold]
    pub(super) fn cold_path() {}

    #[cfg(all(test, target_pointer_width = "32"))]
    mod tests {
        use super::*;

        #[test]
        fn heap_stored_length_preserves_repr_tag() {
            let exact = TextLen::new_exact(MAX_LEN + 1).unwrap();
            let growable = TextLen::new_growable(MAX_LEN + 1).unwrap();

            assert!(exact.is_heap());
            assert!(exact.is_exact());
            assert_eq!(exact.0.to_ne_bytes()[USIZE_SIZE - 1], LastByte::ExactHeapMarker as u8);
            assert!(growable.is_heap());
            assert!(!growable.is_exact());
            assert_eq!(growable.0.to_ne_bytes()[USIZE_SIZE - 1], LastByte::HeapMarker as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_layout_omits_capacity() {
        assert_eq!(HeapBuffer::exact_header_offset(), size_of::<ExactAtomic>());
        assert_eq!(HeapBuffer::growable_header_offset(), 2 * size_of::<usize>());

        let len = MAX_INLINE_SIZE + 1;
        assert_eq!(
            HeapBuffer::layout_from_len(len).unwrap().size(),
            size_of::<ExactAtomic>() + len
        );
        assert_eq!(
            HeapBuffer::layout_from_capacity(Capacity::new(len).unwrap()).unwrap().size(),
            2 * size_of::<usize>() + len
        );
        assert_eq!(HeapBuffer::layout_from_len(len).unwrap().align(), align_of::<usize>());
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn exact_data_immediately_follows_compact_header() {
        let mut exact = HeapBuffer::new_exact("a string longer than the inline limit").unwrap();
        let header = exact.exact_header() as *const ExactHeader as usize;

        assert_eq!(exact.ptr().as_ptr() as usize - header, size_of::<AtomicU32>());

        // SAFETY: This is the only live reference and it is not accessed afterward.
        unsafe { exact.release() };
    }

    #[test]
    fn constructors_select_expected_layout() {
        let text = "a string longer than the inline limit";
        let mut exact = HeapBuffer::new_exact(text).unwrap();
        let mut growable = HeapBuffer::new(text).unwrap();

        assert!(exact.is_exact());
        assert!(!growable.is_exact());
        assert_eq!(exact.capacity(), text.len());
        assert_eq!(growable.capacity(), text.len());

        // SAFETY: These are the only live references to their respective allocations, and neither
        // buffer is accessed afterward.
        unsafe {
            exact.release();
            growable.release();
        }
    }
}
