use core::{
    borrow::Borrow,
    cmp, fmt,
    hash::{Hash, Hasher},
    ops::{Deref, Range},
};

use crate::{LeanString, ReserveError};

/// An owned view into a [`LeanString`].
///
/// `LeanSubstring` shares its owner's storage and stores a byte range rather
/// than allocating and copying the selected text. It occupies 24 bytes on a
/// 64-bit target: a 16-byte [`LeanString`] owner plus 32-bit start and length
/// fields.
///
/// Both the start and byte length must fit in [`u32`]. This keeps the companion
/// type compact, but means it cannot represent a range that starts beyond 4
/// GiB or a single substring longer than 4 GiB, even when [`LeanString`] can
/// represent the owner.
///
/// A small view keeps the owner's complete allocation alive. Call
/// [`LeanSubstring::compact()`] to copy just the selected text into a new
/// [`LeanString`] and release the view's reference to the larger owner.
#[derive(Clone)]
pub struct LeanSubstring {
    owner: LeanString,
    start: u32,
    len: u32,
}

const _: () = {
    assert!(size_of::<LeanSubstring>() == size_of::<LeanString>() + 2 * size_of::<u32>());
};

impl LeanSubstring {
    /// Creates an owned view by consuming `owner`.
    ///
    /// Returns the original owner if `range` is out of bounds, reversed, not on UTF-8
    /// character boundaries, or cannot be represented by the 32-bit range
    /// fields.
    #[inline]
    pub fn from_range(owner: LeanString, range: Range<usize>) -> Result<Self, LeanString> {
        let Some((start, len)) = Self::validate_range(owner.as_str(), range) else {
            return Err(owner);
        };
        Ok(Self { owner, start, len })
    }

    /// Creates an owned view by shallow-cloning `owner`.
    ///
    /// Returns `None` under the same conditions as
    /// creating a consumed view. Validation happens before cloning, so
    /// an invalid range does not touch the owner's reference count.
    #[inline]
    pub fn get(owner: &LeanString, range: Range<usize>) -> Option<Self> {
        let (start, len) = Self::validate_range(owner.as_str(), range)?;
        Some(Self { owner: owner.clone(), start, len })
    }

    #[inline]
    fn validate_range(owner: &str, range: Range<usize>) -> Option<(u32, u32)> {
        let selected = owner.get(range.clone())?;
        let start = u32::try_from(range.start).ok()?;
        let len = u32::try_from(selected.len()).ok()?;
        Some((start, len))
    }

    /// Returns the selected string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.owner.as_str()[self.range()]
    }

    /// Returns the selected bytes.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        self.as_str().as_bytes()
    }

    /// Returns the selected byte length.
    #[inline]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Returns whether the selected range is empty.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the selected byte range within the owner.
    #[inline]
    pub fn range(&self) -> Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }

    /// Returns the owner's complete byte length.
    #[inline]
    pub const fn owner_len(&self) -> usize {
        self.owner.len()
    }

    /// Returns the capacity retained by the owner.
    ///
    /// This is useful for detecting a tiny view that keeps a much larger heap
    /// allocation alive. Static and inline owners do not own a corresponding
    /// heap allocation even though they also report a capacity.
    #[inline]
    pub fn retained_capacity(&self) -> usize {
        self.owner.capacity()
    }

    /// Returns whether this view covers its complete owner.
    #[inline]
    pub fn covers_owner(&self) -> bool {
        self.start == 0 && self.len() == self.owner.len()
    }

    /// Converts the selected text into an owned [`LeanString`].
    ///
    /// If this view covers the complete owner, the owner is moved out without
    /// allocating. Otherwise this allocates and copies the selected text.
    #[inline]
    #[track_caller]
    pub fn into_owned(self) -> LeanString {
        match self.try_into_owned() {
            Ok(owned) => owned,
            Err((error, _view)) => panic!("{error}"),
        }
    }

    /// Fallible version of [`LeanSubstring::into_owned()`].
    ///
    /// On allocation failure, returns the error together with the original
    /// view so the caller can retain it or retry. A view covering its complete
    /// owner still moves that owner out without allocating.
    #[inline]
    pub fn try_into_owned(self) -> Result<LeanString, (ReserveError, LeanSubstring)> {
        self.try_into_owned_with(|selected| selected.parse())
    }

    #[inline]
    fn try_into_owned_with(
        self,
        make_owned: impl FnOnce(&str) -> Result<LeanString, ReserveError>,
    ) -> Result<LeanString, (ReserveError, LeanSubstring)> {
        if self.covers_owner() {
            Ok(self.owner)
        } else {
            match make_owned(self.as_str()) {
                Ok(owned) => Ok(owned),
                Err(error) => Err((error, self)),
            }
        }
    }

    /// Replaces this view's owner with an independent copy of the selected
    /// text and resets its range to start at zero.
    ///
    /// This explicitly releases the view's reference to a potentially much
    /// larger backing allocation. It may allocate even when the view currently
    /// covers its owner.
    #[inline]
    #[track_caller]
    pub fn compact(&mut self) {
        if let Err(error) = self.try_compact() {
            panic!("{error}");
        }
    }

    /// Fallible version of [`LeanSubstring::compact()`].
    ///
    /// If allocation fails, the view and its original owner remain unchanged.
    pub fn try_compact(&mut self) -> Result<(), ReserveError> {
        let compact: LeanString = self.as_str().parse()?;
        self.owner = compact;
        self.start = 0;
        // `self.len` already represents the copied text's length.
        Ok(())
    }
}

impl Deref for LeanSubstring {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for LeanSubstring {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for LeanSubstring {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl Borrow<str> for LeanSubstring {
    #[inline]
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for LeanSubstring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for LeanSubstring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), f)
    }
}

impl Eq for LeanSubstring {}

impl PartialEq for LeanSubstring {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<str> for LeanSubstring {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for LeanSubstring {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<LeanSubstring> for str {
    #[inline]
    fn eq(&self, other: &LeanSubstring) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<LeanSubstring> for &str {
    #[inline]
    fn eq(&self, other: &LeanSubstring) -> bool {
        *self == other.as_str()
    }
}

impl Ord for LeanSubstring {
    #[inline]
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for LeanSubstring {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Hash for LeanSubstring {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_into_owned_preserves_original_view() {
        let owner = LeanString::from("prefix-selected text-suffix");
        let retained_capacity = owner.capacity();
        let view = owner.into_substring(7..20).unwrap();

        let (error, view) = view.try_into_owned_with(|_| Err(ReserveError)).unwrap_err();

        assert_eq!(error, ReserveError);
        assert_eq!(view, "selected text");
        assert_eq!(view.range(), 7..20);
        assert_eq!(view.retained_capacity(), retained_capacity);
    }

    #[test]
    fn complete_owner_path_does_not_invoke_allocator_callback() {
        let owner = LeanString::from("a complete heap-allocated owner");
        let ptr = owner.as_ptr();
        let len = owner.len();
        let view = owner.into_substring(0..len).unwrap();

        let owner =
            view.try_into_owned_with(|_| panic!("complete owner must not be copied")).unwrap();

        assert_eq!(owner.as_ptr(), ptr);
    }
}
