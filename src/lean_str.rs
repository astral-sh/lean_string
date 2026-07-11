use core::{borrow::Borrow, cmp, fmt, hash::Hash, hash::Hasher, mem, ops::Deref, str::FromStr};

use alloc::{borrow::Cow, boxed::Box, string::String};

#[cfg(feature = "std")]
use std::ffi::OsStr;

use crate::{LeanString, Repr, ReserveError, UnwrapWithMsg};

/// Compact, immutable, UTF-8 encoded owned string type.
///
/// Values constructed directly store short strings inline. Long strings use an exactly-sized,
/// reference-counted heap allocation without a capacity field, making clones cheap without
/// retaining capacity that can never be used. Freezing a heap-allocated [`LeanString`] preserves
/// the heap storage kind even when its current contents fit inline. Use
/// [`LeanStr::into_lean_string`] to convert heap storage to the growable layout used by
/// [`LeanString`].
#[repr(transparent)]
pub struct LeanStr(Repr);

const _: () = {
    assert!(size_of::<LeanStr>() == size_of::<[usize; 2]>());
    assert!(size_of::<Option<LeanStr>>() == size_of::<[usize; 2]>());
    assert!(align_of::<LeanStr>() == align_of::<usize>());
    assert!(align_of::<Option<LeanStr>>() == align_of::<usize>());
};

impl LeanStr {
    /// Creates an empty `LeanStr`.
    #[inline]
    pub const fn new() -> Self {
        Self(Repr::new())
    }

    /// Creates a `LeanStr` backed directly by a static string when it does not fit inline.
    #[inline]
    pub const fn from_static_str(text: &'static str) -> Self {
        match Repr::from_static_str(text) {
            Ok(repr) => Self(repr),
            Err(_) => panic!("text is too long"),
        }
    }

    /// Creates an exactly-sized `LeanStr`.
    #[inline]
    pub fn try_from_str(text: &str) -> Result<Self, ReserveError> {
        Repr::from_exact_str(text).map(Self)
    }

    /// Creates an exactly-sized `LeanStr` by joining string slices with a separator.
    ///
    /// Heap storage is allocated at most once. To handle allocation failure, use
    /// [`LeanStr::try_join`].
    ///
    /// # Panics
    ///
    /// Panics if the joined length overflows or the exact buffer cannot be allocated.
    ///
    /// # Examples
    ///
    /// ```
    /// # use lean_string::LeanStr;
    /// let name = LeanStr::join(&["package", "module", "name"], ".");
    /// assert_eq!(name, "package.module.name");
    /// ```
    #[inline]
    pub fn join<T: AsRef<str>>(slices: &[T], separator: &str) -> Self {
        Self::try_join(slices, separator).unwrap_with_msg()
    }

    /// Fallible version of [`LeanStr::join`].
    ///
    /// Returns a [`ReserveError`] if the joined length overflows, the exact buffer cannot be
    /// allocated, or an [`AsRef`] implementation reports inconsistent slice lengths while the
    /// result is constructed.
    #[inline]
    pub fn try_join<T: AsRef<str>>(slices: &[T], separator: &str) -> Result<Self, ReserveError> {
        Repr::from_exact_joined_slices(slices, separator).map(Self)
    }

    /// Returns the string as a string slice.
    #[inline]
    pub const fn as_str(&self) -> &str {
        self.0.as_str()
    }

    /// Returns the string as a byte slice.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Returns the length in bytes.
    #[inline]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` if the string has a length of zero.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns whether the string uses a reference-counted heap allocation.
    #[inline]
    pub const fn is_heap_allocated(&self) -> bool {
        self.0.is_heap_buffer()
    }

    /// Converts this value into a mutable [`LeanString`].
    ///
    /// Unique exact heap storage is converted to growable storage with `realloc`. Shared heap
    /// storage is copied into a new growable allocation.
    ///
    /// # Panics
    ///
    /// Panics if reallocating or copying heap storage fails. To handle allocation failure, use
    /// [`LeanStr::try_into_lean_string`].
    #[inline]
    pub fn into_lean_string(self) -> LeanString {
        self.try_into_lean_string().unwrap_with_msg()
    }

    /// Tries to convert this value into a mutable [`LeanString`].
    ///
    /// Inline and static storage are transferred without reallocating. Unique exact heap storage
    /// is converted with `realloc`, while shared heap storage is copied.
    ///
    /// # Errors
    ///
    /// Returns a [`ReserveError`] if converting heap storage fails. Because this method consumes
    /// `self`, the original [`LeanStr`] is dropped on failure.
    #[inline]
    pub fn try_into_lean_string(mut self) -> Result<LeanString, ReserveError> {
        self.0.make_growable()?;
        Ok(LeanString::from_repr(mem::replace(&mut self.0, Repr::new())))
    }

    pub(crate) fn from_repr(repr: Repr) -> Self {
        debug_assert!(!repr.is_growable_heap_buffer());
        Self(repr)
    }
}

impl Clone for LeanStr {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.make_shallow_clone())
    }

    #[inline]
    fn clone_from(&mut self, source: &Self) {
        self.0.replace_inner(source.0.make_shallow_clone());
    }
}

impl Drop for LeanStr {
    #[inline]
    fn drop(&mut self) {
        self.0.replace_inner(Repr::new());
    }
}

// SAFETY: `LeanStr` is `repr(transparent)` over `Repr`, and heap storage is immutable and
// reference-counted like `Arc`.
unsafe impl Send for LeanStr {}
unsafe impl Sync for LeanStr {}

impl Default for LeanStr {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for LeanStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Debug for LeanStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for LeanStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl AsRef<str> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(feature = "std")]
impl AsRef<OsStr> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.as_str())
    }
}

impl AsRef<[u8]> for LeanStr {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Borrow<str> for LeanStr {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Eq for LeanStr {}

impl PartialEq for LeanStr {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<str> for LeanStr {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<LeanStr> for str {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<&str> for LeanStr {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<LeanStr> for &str {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<String> for LeanStr {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<LeanStr> for String {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<LeanString> for LeanStr {
    #[inline]
    fn eq(&self, other: &LeanString) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<LeanStr> for LeanString {
    #[inline]
    fn eq(&self, other: &LeanStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Ord for LeanStr {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for LeanStr {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for LeanStr {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl From<char> for LeanStr {
    #[inline]
    fn from(value: char) -> Self {
        Self(Repr::from_char(value))
    }
}

impl From<&str> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: &str) -> Self {
        Self::try_from_str(value).unwrap_with_msg()
    }
}

impl From<String> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&String> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<Cow<'_, str>> for LeanStr {
    fn from(value: Cow<'_, str>) -> Self {
        Self::from(value.as_ref())
    }
}

impl From<Box<str>> for LeanStr {
    #[inline]
    #[track_caller]
    fn from(value: Box<str>) -> Self {
        Self::from(value.as_ref())
    }
}

impl From<&LeanStr> for LeanStr {
    #[inline]
    fn from(value: &LeanStr) -> Self {
        value.clone()
    }
}

impl From<LeanStr> for String {
    #[inline]
    fn from(value: LeanStr) -> Self {
        value.as_str().into()
    }
}

impl From<&LeanStr> for String {
    #[inline]
    fn from(value: &LeanStr) -> Self {
        value.as_str().into()
    }
}

impl From<LeanString> for LeanStr {
    #[inline]
    fn from(value: LeanString) -> Self {
        value.freeze()
    }
}

impl From<LeanStr> for LeanString {
    #[inline]
    fn from(value: LeanStr) -> Self {
        value.into_lean_string()
    }
}

impl FromStr for LeanStr {
    type Err = ReserveError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_str(s)
    }
}

impl FromIterator<char> for LeanStr {
    fn from_iter<T: IntoIterator<Item = char>>(iter: T) -> Self {
        LeanString::from_iter(iter).freeze()
    }
}

impl<'a> FromIterator<&'a char> for LeanStr {
    fn from_iter<T: IntoIterator<Item = &'a char>>(iter: T) -> Self {
        iter.into_iter().copied().collect()
    }
}
