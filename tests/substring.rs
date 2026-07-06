use std::collections::{HashMap, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};

use lean_string::{LeanString, LeanSubstring};

#[test]
fn companion_layout_does_not_change_lean_string() {
    assert_eq!(size_of::<LeanString>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<LeanSubstring>(), size_of::<LeanString>() + 2 * size_of::<u32>());
}

#[test]
fn validates_range_and_utf8_boundaries() {
    let owner = LeanString::from("hello, 世界");

    assert_eq!(owner.substring(0..5).unwrap(), "hello");
    assert_eq!(owner.substring(7..13).unwrap(), "世界");
    assert!(owner.substring(8..13).is_none());
    assert!(owner.substring(7..12).is_none());
    assert!(owner.substring(owner.len()..5).is_none());
    assert!(owner.substring(0..owner.len() + 1).is_none());
}

#[test]
fn supports_empty_ranges() {
    let owner = LeanString::from("hello");
    let empty = owner.substring(owner.len()..owner.len()).unwrap();

    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.as_str(), "");
    assert_eq!(empty.range(), owner.len()..owner.len());
}

#[test]
fn clone_and_traits_use_selected_text() {
    let owner = LeanString::from("prefix-alpha-suffix-prefix-beta-suffix");
    let alpha = owner.substring(7..12).unwrap();
    let alpha_clone = alpha.clone();
    let beta = owner.substring(27..31).unwrap();

    assert_eq!(alpha, alpha_clone);
    assert!(alpha < beta);
    assert_eq!(alpha.to_string(), "alpha");
    assert_eq!(format!("{alpha:?}"), "\"alpha\"");
    assert_eq!(alpha.as_ref() as &str, "alpha");
    assert_eq!(AsRef::<[u8]>::as_ref(&alpha), b"alpha");

    let mut alpha_hash = DefaultHasher::new();
    alpha.hash(&mut alpha_hash);
    let mut str_hash = DefaultHasher::new();
    "alpha".hash(&mut str_hash);
    assert_eq!(alpha_hash.finish(), str_hash.finish());

    let mut map = HashMap::new();
    map.insert(alpha, 1);
    assert_eq!(map.get("alpha"), Some(&1));
}

#[test]
fn consuming_constructor_avoids_an_extra_owner_clone() {
    let owner = LeanString::from("a complete heap-allocated owner");
    let ptr = owner.as_ptr();
    let len = owner.len();
    let view = owner.into_substring(0..len).unwrap();

    assert!(view.covers_owner());
    let owner = view.into_owned();
    assert_eq!(owner.as_ptr(), ptr);
}

#[test]
fn partial_into_owned_copies_only_the_selection() {
    let owner = LeanString::from("prefix-selected text-suffix");
    let owner_capacity = owner.capacity();
    let view = owner.into_substring(7..20).unwrap();

    assert_eq!(view.retained_capacity(), owner_capacity);
    let selected = view.into_owned();
    assert_eq!(selected, "selected text");
    assert!(selected.capacity() < owner_capacity);
}

#[test]
fn compact_releases_large_owner_reference() {
    let owner = LeanString::from("x".repeat(1024 * 1024));
    let mut view = owner.into_substring(500_000..500_008).unwrap();

    assert_eq!(view, "xxxxxxxx");
    assert_eq!(view.owner_len(), 1024 * 1024);
    assert!(view.retained_capacity() >= 1024 * 1024);

    view.compact();

    assert_eq!(view, "xxxxxxxx");
    assert_eq!(view.range(), 0..8);
    assert_eq!(view.owner_len(), 8);
    assert_eq!(view.retained_capacity(), size_of::<LeanString>());
}

#[test]
fn failed_consuming_range_returns_the_owner() {
    let owner = LeanString::from("a heap-allocated owner for invalid slicing");
    let owner = owner.into_substring(1..usize::MAX).unwrap_err();
    assert_eq!(owner, "a heap-allocated owner for invalid slicing");
}

#[test]
fn is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<LeanSubstring>();
}
