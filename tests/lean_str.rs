use lean_string::{LeanStr, LeanString};

const INLINE_LIMIT: usize = size_of::<LeanStr>();

#[test]
fn size() {
    assert_eq!(size_of::<LeanStr>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<Option<LeanStr>>(), size_of::<LeanStr>());
}

#[test]
fn storage_kinds() {
    let inline = LeanStr::from("x".repeat(INLINE_LIMIT));
    let heap = LeanStr::from("x".repeat(INLINE_LIMIT + 1));
    const STATIC: LeanStr =
        LeanStr::from_static_str("a static string longer than the inline limit");

    assert!(!inline.is_heap_allocated());
    assert!(heap.is_heap_allocated());
    assert!(!STATIC.is_heap_allocated());
}

#[test]
fn clone_shares_heap_storage() {
    let one = LeanStr::from("a string longer than the inline limit");
    let two = one.clone();

    assert!(core::ptr::eq(one.as_ptr(), two.as_ptr()));
    assert_eq!(one, two);
    assert_eq!(one.cmp(&two), core::cmp::Ordering::Equal);
}

#[test]
fn comparisons_with_shared_lean_string_storage() {
    let frozen = LeanStr::from("a string longer than the inline limit");
    let thawed = frozen.clone().into_lean_string();

    assert!(core::ptr::eq(frozen.as_ptr(), thawed.as_ptr()));
    assert_eq!(frozen, thawed);
    assert_eq!(thawed, frozen);
}

#[test]
fn comparisons_with_shared_storage_respect_length() {
    let frozen = LeanStr::from_static_str("a static string longer than the inline limit");
    let mut thawed = frozen.clone().into_lean_string();
    thawed.truncate(frozen.len() - 1);

    assert!(core::ptr::eq(frozen.as_ptr(), thawed.as_ptr()));
    assert_ne!(frozen, thawed);
    assert_ne!(thawed, frozen);
}

#[test]
fn comparisons_fall_back_to_content() {
    let one = LeanStr::from("a string longer than the inline limit");
    let equal = LeanStr::from("a string longer than the inline limit");
    let greater = LeanStr::from("b string longer than the inline limit");

    assert!(!core::ptr::eq(one.as_ptr(), equal.as_ptr()));
    assert_eq!(one, equal);
    assert_eq!(one.cmp(&equal), core::cmp::Ordering::Equal);
    assert!(one < greater);
}

#[test]
fn freeze_and_thaw() {
    let mut string = LeanString::with_capacity(128);
    string.push_str("a string longer than the inline limit");

    let frozen = string.freeze();
    let shared = frozen.clone();
    let mut thawed = frozen.into_lean_string();
    thawed.push_str(" with more text");

    assert_eq!(shared, "a string longer than the inline limit");
    assert_eq!(thawed, "a string longer than the inline limit with more text");
}

#[test]
fn try_freeze_compacts_growable_storage() {
    let text = "a string longer than the inline limit";
    let mut string = LeanString::with_capacity(128);
    string.push_str(text);

    let frozen = string.try_freeze().unwrap();
    let thawed = frozen.into_lean_string();

    assert_eq!(thawed, text);
    assert_eq!(thawed.capacity(), thawed.len());
}

#[test]
fn clear_unique_thawed_string_releases_exact_storage() {
    let mut thawed =
        LeanStr::from("a frozen string longer than the inline limit").into_lean_string();

    thawed.clear();

    assert!(thawed.is_empty());
    assert_eq!(thawed.capacity(), INLINE_LIMIT);
    assert!(!thawed.is_heap_allocated());
}

#[test]
fn clear_shared_thawed_string_preserves_frozen_clone() {
    let frozen = LeanStr::from("a frozen string longer than the inline limit");
    let shared = frozen.clone();
    let mut thawed = frozen.into_lean_string();

    thawed.clear();

    assert!(thawed.is_empty());
    assert_eq!(thawed.capacity(), INLINE_LIMIT);
    assert!(!thawed.is_heap_allocated());
    assert_eq!(shared, "a frozen string longer than the inline limit");
    assert!(shared.is_heap_allocated());
}

#[test]
fn collect_freezes_builder() {
    let frozen: LeanStr = "a string longer than the inline limit".chars().collect();

    assert_eq!(frozen, "a string longer than the inline limit");
    assert!(frozen.is_heap_allocated());
}
