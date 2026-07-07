use super::*;
use alloc::alloc::{alloc, dealloc, realloc};
use core::{alloc::Layout, hint, ptr, ptr::NonNull};

#[cfg(all(not(loom), target_pointer_width = "64"))]
use core::sync::atomic::AtomicU32 as AtomicRefCount;
#[cfg(all(not(loom), target_pointer_width = "32"))]
use core::sync::atomic::AtomicUsize as AtomicRefCount;
#[cfg(all(loom, target_pointer_width = "64"))]
use loom::sync::atomic::AtomicU32 as AtomicRefCount;
#[cfg(all(loom, target_pointer_width = "32"))]
use loom::sync::atomic::AtomicUsize as AtomicRefCount;

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
    // 64-bit architecture or 32-bit architecture if `is_len_heap_layout` is false:
    // | Header | Data (array of `u8`) |
    //          ^ ptr
    // 32-bit architecture if `is_len_heap_layout` is true:
    // | Length | Header | Data (array of `u8`) |
    //                   ^ ptr
    ptr: NonNull<u8>,
    len: TextLen,
}

struct Header {
    count: AtomicRefCount,
    capacity: HeaderCapacity,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    Compact,
    #[cfg(target_pointer_width = "64")]
    Wide,
}

#[cfg(target_pointer_width = "64")]
type HeaderCapacity = u32;
#[cfg(target_pointer_width = "32")]
type HeaderCapacity = Capacity;

#[cfg(target_pointer_width = "64")]
const WIDE_CAPACITY_SENTINEL: HeaderCapacity = u32::MAX;

impl HeaderKind {
    #[cfg_attr(target_pointer_width = "32", allow(unused_variables))]
    fn for_capacity(capacity: Capacity) -> Self {
        #[cfg(target_pointer_width = "64")]
        if capacity.as_usize() >= WIDE_CAPACITY_SENTINEL as usize {
            cold_path();
            return HeaderKind::Wide;
        }

        HeaderKind::Compact
    }

    const fn extra_size(self) -> usize {
        match self {
            HeaderKind::Compact => 0,
            #[cfg(target_pointer_width = "64")]
            HeaderKind::Wide => size_of::<usize>(),
        }
    }

    #[cfg_attr(target_pointer_width = "32", allow(unused_variables))]
    fn can_represent(self, capacity: Capacity) -> bool {
        match self {
            HeaderKind::Compact => {
                #[cfg(target_pointer_width = "64")]
                return capacity.as_usize() < WIDE_CAPACITY_SENTINEL as usize;

                #[cfg(target_pointer_width = "32")]
                return true;
            }
            #[cfg(target_pointer_width = "64")]
            HeaderKind::Wide => true,
        }
    }
}

impl Header {
    fn new(capacity: Capacity, kind: HeaderKind) -> Self {
        Self { count: AtomicRefCount::new(1), capacity: capacity.into_header(kind) }
    }

    #[cfg(all(test, target_pointer_width = "64", not(loom)))]
    fn kind(&self) -> HeaderKind {
        Header::kind_from_capacity(self.capacity)
    }

    #[cfg_attr(target_pointer_width = "32", allow(unused_variables))]
    fn kind_from_capacity(capacity: HeaderCapacity) -> HeaderKind {
        #[cfg(target_pointer_width = "64")]
        if capacity == WIDE_CAPACITY_SENTINEL {
            return HeaderKind::Wide;
        }

        HeaderKind::Compact
    }

    /// Reads the capacity while preserving the provenance of the complete allocation.
    ///
    /// # Safety
    /// `header` must point to an initialized header in an allocation created by `HeapBuffer`.
    unsafe fn capacity(header: *const Header) -> Capacity {
        unsafe { Header::metadata(header).1 }
    }

    /// Reads the header kind and capacity in a single metadata lookup.
    ///
    /// # Safety
    /// `header` must point to an initialized header in an allocation created by `HeapBuffer`.
    unsafe fn metadata(header: *const Header) -> (HeaderKind, Capacity) {
        // SAFETY: The caller guarantees that `header` points to an initialized `Header`.
        let compact_capacity = unsafe { ptr::read(ptr::addr_of!((*header).capacity)) };
        let kind = Header::kind_from_capacity(compact_capacity);

        let capacity = match kind {
            HeaderKind::Compact => Capacity::from_compact_header(compact_capacity),
            #[cfg(target_pointer_width = "64")]
            HeaderKind::Wide => {
                cold_path();
                // SAFETY: A wide header is preceded by an aligned `usize` containing its full
                // capacity. `header` retains the provenance of the complete allocation.
                let capacity =
                    unsafe { ptr::read(header.cast::<u8>().sub(size_of::<usize>()).cast()) };
                Capacity::from_wide_header(capacity)
            }
        };

        (kind, capacity)
    }
}

const _: () = {
    assert!(size_of::<HeapBuffer>() == MAX_INLINE_SIZE);
    assert!(align_of::<HeapBuffer>() == align_of::<usize>());
};

impl HeapBuffer {
    pub(super) fn new(text: &str) -> Result<Self, ReserveError> {
        let text_len = text.len();

        let len = TextLen::new(text_len)?;
        let ptr = HeapBuffer::allocate_ptr(Capacity::new(text_len)?)?;

        if len.is_heap() {
            // SAFETY: Since we passed `text_len` as the capacity and `len` equals to `text_len`,
            // `ptr` is allocated with enough space to store the length.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::header_offset()).sub(size_of::<usize>());
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
        let cap = Capacity::new(capacity)?;
        HeapBuffer::with_capacity_and_kind(cap, HeaderKind::for_capacity(cap))
    }

    fn with_capacity_and_kind(capacity: Capacity, kind: HeaderKind) -> Result<Self, ReserveError> {
        let len = TextLen::new(0)?;
        let ptr = HeapBuffer::allocate_ptr_with_kind(capacity, kind)?;
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

        let len = TextLen::new(text_len)?;
        let ptr = {
            let new_capacity = Capacity::new(amortized_growth(text_len, additional))?;
            HeapBuffer::allocate_ptr(new_capacity)?
        };

        if len.is_heap() {
            // SAFETY: Since the `new_capacity` is greater than or equal to `text_len`, `ptr` is
            // allocated with enough space to store the length.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::header_offset()).sub(size_of::<usize>());
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
        // SAFETY: `self.header_ptr()` points to this buffer's initialized header.
        unsafe { Header::capacity(self.header_ptr()).as_usize() }
    }

    pub(crate) fn ptr(&self) -> NonNull<u8> {
        self.ptr
    }

    pub(super) const fn len(&self) -> usize {
        #[cold]
        const fn len_on_heap(ptr: NonNull<u8>) -> usize {
            // SAFETY: We just checked that `len` is stored on the heap.
            unsafe {
                let len_ptr = ptr.sub(HeapBuffer::header_offset()).sub(size_of::<usize>());
                ptr::read(len_ptr.as_ptr().cast())
            }
        }
        if self.len.is_heap() { len_on_heap(self.ptr) } else { self.len.as_usize() }
    }

    pub(super) fn as_str(&self) -> &str {
        let len = self.len();
        let ptr = self.ptr.as_ptr();
        // SAFETY: HeapBuffer contains valid `len` bytes of UTF-8 string.
        unsafe { core::str::from_utf8_unchecked(slice::from_raw_parts(ptr, len)) }
    }

    /// # Safety
    /// - The buffer must be unique. (HeapBuffer::is_unique() == true)
    /// - `new_capacity` must be greater than or equal to the current string length.
    pub(super) unsafe fn realloc(&mut self, new_capacity: usize) -> Result<(), ReserveError> {
        debug_assert!(self.is_unique());
        debug_assert!(self.len() <= new_capacity);

        let new_capacity = Capacity::new(new_capacity)?;
        let new_kind = HeaderKind::for_capacity(new_capacity);

        unsafe { self.realloc_with_kind(new_capacity, new_kind) }
    }

    /// # Safety
    /// The same requirements as `realloc` apply. `new_kind` must be able to represent
    /// `new_capacity`; production callers should use `HeaderKind::for_capacity`.
    unsafe fn realloc_with_kind(
        &mut self,
        new_capacity: Capacity,
        new_kind: HeaderKind,
    ) -> Result<(), ReserveError> {
        // SAFETY: `self.header_ptr()` points to this buffer's initialized header.
        let (cur_kind, cur_capacity) = unsafe { Header::metadata(self.header_ptr()) };

        debug_assert!(new_kind.can_represent(new_capacity));

        let layout_changes = cur_kind != new_kind
            || is_len_heap_layout(cur_capacity) != is_len_heap_layout(new_capacity);

        if layout_changes {
            let str = self.as_str();
            let mut new_buf = HeapBuffer::with_capacity_and_kind(new_capacity, new_kind)?;
            unsafe {
                ptr::copy_nonoverlapping(str.as_ptr(), new_buf.ptr.as_ptr(), str.len());
                new_buf.set_len(str.len());
                self.dealloc();
            }
            *self = new_buf;
            return Ok(());
        }

        let cur_layout = match HeapBuffer::layout_from_capacity(cur_capacity, cur_kind) {
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

        let new_alloc_size = HeapBuffer::allocation_size(new_capacity, new_kind)?;

        // SAFETY:
        // - `self.allocation()` is already allocated by global allocator.
        // - current allocation is allocated by `cur_layout`.
        // - `new_alloc_size` is a valid non-zero allocation size.
        let allocation =
            unsafe { realloc(self.allocation(cur_kind, cur_capacity), cur_layout, new_alloc_size) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        // SAFETY: The reallocated block is valid for `new_alloc_size`, and the unchanged header
        // kind keeps the initialized data at the same offset.
        self.ptr = unsafe { HeapBuffer::initialize_allocation(allocation, new_capacity, new_kind) };
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
        if self.reference_count().fetch_sub(1, Release) == 1 {
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
        // SAFETY: `self.header_ptr()` points to this buffer's initialized header.
        let (kind, capacity) = unsafe { Header::metadata(self.header_ptr()) };
        let layout = match HeapBuffer::layout_from_capacity(capacity, kind) {
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
            dealloc(self.allocation(kind, capacity), layout);
        }
    }

    pub(super) fn is_unique(&self) -> bool {
        self.header().count.load(Acquire) == 1
    }

    pub(super) fn is_len_on_heap(&self) -> bool {
        self.len.is_heap()
    }

    pub(super) fn reference_count(&self) -> &AtomicRefCount {
        &self.header().count
    }

    /// Attempts to add a reference without permitting the compact counter to wrap.
    ///
    /// A heap allocation can have at most `u32::MAX` live references on 64-bit architectures.
    /// This makes overflow impossible even if many threads concurrently clone the same value.
    #[cfg(target_pointer_width = "64")]
    pub(super) fn try_increment_reference_count(&self) -> bool {
        let count = &self.header().count;

        #[cfg(not(loom))]
        return count.try_update(Relaxed, Relaxed, |count| count.checked_add(1)).is_ok();

        // Loom's atomic model has not yet adopted the standard library's `try_update` name.
        #[cfg(loom)]
        return count.fetch_update(Relaxed, Relaxed, |count| count.checked_add(1)).is_ok();
    }

    /// # Safety
    /// - `len` bytes in the buffer must be valid UTF-8.
    /// - `len` must be less than or equal to the capacity.
    /// - If `len` is stored on the heap, the buffer must be unique.
    pub(super) unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity());

        let new_len = match TextLen::new(len) {
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
                let len_ptr = ptr.sub(HeapBuffer::header_offset()).sub(size_of::<usize>());
                ptr::write(len_ptr.as_ptr().cast(), len);
            }
        }
        if self.len.is_heap() {
            write_len_on_heap(self.ptr, len);
        }
    }

    fn allocate_ptr(capacity: Capacity) -> Result<NonNull<u8>, ReserveError> {
        let kind = HeaderKind::for_capacity(capacity);
        HeapBuffer::allocate_ptr_with_kind(capacity, kind)
    }

    fn allocate_ptr_with_kind(
        capacity: Capacity,
        kind: HeaderKind,
    ) -> Result<NonNull<u8>, ReserveError> {
        debug_assert!(kind.can_represent(capacity));
        let layout = HeapBuffer::layout_from_capacity(capacity, kind)?;

        // SAFETY: layout is non-zero.
        let allocation = unsafe { alloc(layout) };
        if allocation.is_null() {
            return Err(ReserveError);
        }

        // SAFETY: `allocation` is valid for `layout`, which was constructed for this capacity and
        // header kind.
        Ok(unsafe { HeapBuffer::initialize_allocation(allocation, capacity, kind) })
    }

    fn layout_from_capacity(capacity: Capacity, kind: HeaderKind) -> Result<Layout, ReserveError> {
        let alloc_size = HeapBuffer::allocation_size(capacity, kind)?;
        let align = HeapBuffer::align();
        Layout::from_size_align(alloc_size, align).map_err(
            #[cold]
            |_| ReserveError,
        )
    }

    fn allocation_size(capacity: Capacity, kind: HeaderKind) -> Result<usize, ReserveError> {
        #[cfg(target_pointer_width = "64")]
        {
            // `Capacity::new` limits the value to 56 bits, leaving ample room for both headers.
            Ok(size_of::<Header>() + kind.extra_size() + capacity.as_usize())
        }

        #[cfg(target_pointer_width = "32")]
        {
            const ALLOC_LIMIT: usize = (isize::MAX as usize + 1) - HeapBuffer::align();
            let alloc_size = size_of::<Header>()
                .checked_add(kind.extra_size())
                .and_then(|size| size.checked_add(capacity.as_usize()))
                .and_then(|size| {
                    if is_len_heap_layout(capacity) {
                        size.checked_add(size_of::<usize>())
                    } else {
                        Some(size)
                    }
                })
                .ok_or(ReserveError)?;

            if alloc_size > ALLOC_LIMIT {
                cold_path();
                return Err(ReserveError);
            }
            Ok(alloc_size)
        }
    }

    /// Initializes allocation metadata and returns the data pointer.
    ///
    /// # Safety
    /// `allocation` must be aligned and valid for the layout returned by
    /// `layout_from_capacity(capacity, kind)`.
    unsafe fn initialize_allocation(
        mut allocation: *mut u8,
        capacity: Capacity,
        kind: HeaderKind,
    ) -> NonNull<u8> {
        if is_len_heap_layout(capacity) {
            // SAFETY: The allocation reserves a leading `usize` for the heap-stored length.
            unsafe { allocation = allocation.add(size_of::<usize>()) };
        }

        #[cfg(target_pointer_width = "64")]
        if kind == HeaderKind::Wide {
            cold_path();
            // SAFETY: A wide layout reserves an aligned `usize` before the common header.
            unsafe {
                ptr::write(allocation.cast(), capacity.as_usize());
                allocation = allocation.add(size_of::<usize>());
            }
        }

        // SAFETY: The remaining prefix is valid and aligned for the common header, and the data
        // starts immediately after it.
        unsafe {
            ptr::write(allocation.cast(), Header::new(capacity, kind));
            NonNull::new_unchecked(allocation.add(HeapBuffer::header_offset()))
        }
    }

    #[cfg_attr(target_pointer_width = "32", allow(unused_variables))]
    unsafe fn allocation(&self, kind: HeaderKind, capacity: Capacity) -> *mut u8 {
        unsafe {
            let mut allocation = self.header_ptr().cast::<u8>();

            #[cfg(target_pointer_width = "64")]
            if kind == HeaderKind::Wide {
                cold_path();
                allocation = allocation.sub(size_of::<usize>());
            }

            if is_len_heap_layout(capacity) {
                cold_path();
                allocation = allocation.sub(size_of::<usize>());
            }

            allocation
        }
    }

    fn header(&self) -> &Header {
        // SAFETY: `self.header_ptr()` points to this buffer's initialized header.
        unsafe { &*self.header_ptr() }
    }

    fn header_ptr(&self) -> *mut Header {
        // SAFETY: Every data pointer returned by `initialize_allocation` is immediately preceded
        // by an initialized common header.
        unsafe { self.ptr.as_ptr().sub(HeapBuffer::header_offset()).cast() }
    }

    const fn align() -> usize {
        const {
            assert!(align_of::<Header>() <= align_of::<usize>());
            assert!(size_of::<Header>().is_multiple_of(align_of::<usize>()));
            assert!(align_of::<NonNull<u8>>() == align_of::<usize>());
        }
        align_of::<usize>()
    }

    const fn header_offset() -> usize {
        max(size_of::<Header>(), HeapBuffer::align())
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
    /// And the tag is [`LastByte::Heap`].
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
        const TAG: usize = {
            let mut bytes = [0; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        #[cfg(target_pointer_width = "32")]
        const ON_THE_HEAP: usize = {
            let mut bytes = [255; USIZE_SIZE];
            bytes[USIZE_SIZE - 1] = LastByte::HeapMarker as u8;
            usize::from_ne_bytes(bytes)
        };

        pub(super) const fn new(size: usize) -> Result<Self, ReserveError> {
            if size > MAX_LEN {
                #[cfg(target_pointer_width = "64")]
                return Err(ReserveError);
                #[cfg(target_pointer_width = "32")]
                return Ok(TextLen(Self::ON_THE_HEAP));
            }
            Ok(TextLen(size.to_le() | Self::TAG))
        }

        #[inline(always)]
        pub(super) const fn is_heap(&self) -> bool {
            #[cfg(target_pointer_width = "64")]
            return false;
            #[cfg(target_pointer_width = "32")]
            return self.0 == Self::ON_THE_HEAP;
        }

        pub(super) const fn as_usize(self) -> usize {
            let size = self.0 ^ Self::TAG;
            let bytes = size.to_ne_bytes();
            usize::from_le_bytes(bytes)
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

        #[cfg(target_pointer_width = "64")]
        pub(super) fn into_header(self, kind: HeaderKind) -> HeaderCapacity {
            match kind {
                HeaderKind::Compact => {
                    debug_assert!(self.0 < WIDE_CAPACITY_SENTINEL as usize);
                    self.0 as u32
                }
                HeaderKind::Wide => {
                    cold_path();
                    WIDE_CAPACITY_SENTINEL
                }
            }
        }

        #[cfg(target_pointer_width = "32")]
        pub(super) fn into_header(self, kind: HeaderKind) -> HeaderCapacity {
            debug_assert!(kind == HeaderKind::Compact);
            self
        }

        #[cfg(target_pointer_width = "64")]
        pub(super) fn from_compact_header(capacity: HeaderCapacity) -> Self {
            debug_assert_ne!(capacity, WIDE_CAPACITY_SENTINEL);
            Capacity(capacity as usize)
        }

        #[cfg(target_pointer_width = "32")]
        pub(super) fn from_compact_header(capacity: HeaderCapacity) -> Self {
            capacity
        }

        #[cfg(target_pointer_width = "64")]
        pub(super) fn from_wide_header(capacity: usize) -> Self {
            Capacity(capacity)
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
            let len = TextLen::new(MAX_LEN + 1).unwrap();

            assert!(len.is_heap());
            assert_eq!(len.0.to_ne_bytes()[USIZE_SIZE - 1], LastByte::HeapMarker as u8);
        }

        #[test]
        fn allocation_size_respects_layout_limit() {
            let alloc_limit = (isize::MAX as usize + 1) - HeapBuffer::align();
            let metadata_size = size_of::<Header>() + size_of::<usize>();
            let largest_capacity = Capacity::new(alloc_limit - metadata_size).unwrap();
            let oversized_capacity = Capacity::new(alloc_limit - metadata_size + 1).unwrap();

            assert!(
                HeapBuffer::layout_from_capacity(largest_capacity, HeaderKind::Compact).is_ok()
            );
            assert!(
                HeapBuffer::layout_from_capacity(oversized_capacity, HeaderKind::Compact).is_err()
            );
        }
    }
}

#[cfg(all(test, target_pointer_width = "64", not(loom)))]
mod compact_header_tests {
    use super::*;

    #[test]
    fn common_header_is_eight_bytes() {
        assert_eq!(size_of::<Header>(), 8);
        assert_eq!(HeapBuffer::header_offset(), 8);
        assert_eq!(HeapBuffer::align(), align_of::<usize>());
    }

    #[test]
    fn capacity_uses_wide_header_at_the_sentinel() {
        let compact = Capacity::new(u32::MAX as usize - 1).unwrap();
        let wide = Capacity::new(u32::MAX as usize).unwrap();

        assert!(HeaderKind::for_capacity(compact) == HeaderKind::Compact);
        assert!(HeaderKind::for_capacity(wide) == HeaderKind::Wide);
        let max_capacity = (1usize << 56) - 1;
        assert!(Capacity::new(max_capacity).is_ok());
        assert!(Capacity::new(max_capacity + 1).is_err());
    }

    #[test]
    fn reference_count_cannot_wrap() {
        let mut heap = HeapBuffer::new("a string larger than inline").unwrap();
        heap.header().count.store(u32::MAX, Relaxed);
        assert!(!heap.try_increment_reference_count());
        assert_eq!(heap.header().count.load(Relaxed), u32::MAX);

        // Restore the live-reference invariant so the test can release the allocation normally.
        heap.header().count.store(1, Relaxed);
        // SAFETY: This is the only live reference and `heap` is not accessed again.
        unsafe { heap.release() };
    }

    #[test]
    fn forced_wide_header_preserves_data_across_reallocations() {
        const TEXT: &str = "a string larger than inline";

        let mut heap =
            HeapBuffer::with_capacity_and_kind(Capacity::new(64).unwrap(), HeaderKind::Wide)
                .unwrap();
        unsafe {
            ptr::copy_nonoverlapping(TEXT.as_ptr(), heap.ptr.as_ptr(), TEXT.len());
            heap.set_len(TEXT.len());
        }

        assert!(heap.header().kind() == HeaderKind::Wide);
        assert_eq!(heap.capacity(), 64);
        assert_eq!(heap.as_str(), TEXT);

        unsafe {
            heap.realloc_with_kind(Capacity::new(96).unwrap(), HeaderKind::Wide).unwrap();
        }
        assert!(heap.header().kind() == HeaderKind::Wide);
        assert_eq!(heap.capacity(), 96);
        assert_eq!(heap.as_str(), TEXT);

        unsafe {
            heap.realloc(48).unwrap();
        }
        assert!(heap.header().kind() == HeaderKind::Compact);
        assert_eq!(heap.capacity(), 48);
        assert_eq!(heap.as_str(), TEXT);

        unsafe {
            heap.realloc_with_kind(Capacity::new(80).unwrap(), HeaderKind::Wide).unwrap();
        }
        assert!(heap.header().kind() == HeaderKind::Wide);
        assert_eq!(heap.capacity(), 80);
        assert_eq!(heap.as_str(), TEXT);

        assert!(heap.try_increment_reference_count());
        let mut clone = HeapBuffer { ptr: heap.ptr, len: heap.len };
        // SAFETY: `clone` keeps the allocation alive and `heap` is not accessed again.
        unsafe { heap.release() };
        assert_eq!(clone.header().count.load(Relaxed), 1);
        assert_eq!(clone.as_str(), TEXT);
        // SAFETY: This is the final live reference and `clone` is not accessed again.
        unsafe { clone.release() };
    }
}
