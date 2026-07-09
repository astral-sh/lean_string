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
}

#[test]
fn clone_from_handles_all_storage_kinds() {
    const STATIC: LeanStr =
        LeanStr::from_static_str("a static string longer than the inline limit");

    let exact = LeanStr::from("an exact string longer than the inline limit");
    let mut value = LeanStr::from("another exact string longer than the inline limit");

    value.clone_from(&exact);
    assert!(core::ptr::eq(value.as_ptr(), exact.as_ptr()));

    let inline = LeanStr::from("inline");
    value.clone_from(&inline);
    assert_eq!(value, inline);
    assert!(!value.is_heap_allocated());

    value.clone_from(&STATIC);
    assert_eq!(value, STATIC);
    assert!(!value.is_heap_allocated());
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
