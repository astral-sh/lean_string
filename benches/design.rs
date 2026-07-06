use core::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use lean_string::LeanString;

const MOVE_COLLECTION_COUNTS: [usize; 5] = [0, 1, 2, 8, 64];

fn segment(index: usize, len: usize) -> String {
    let byte = b'a' + (index % 26) as u8;
    String::from_utf8(vec![byte; len]).unwrap()
}

fn lean_segments(count: usize, len: usize) -> Vec<LeanString> {
    (0..count).map(|index| LeanString::from(segment(index, len))).collect()
}

fn reserved_first_segments(count: usize, len: usize) -> Vec<LeanString> {
    if count == 0 {
        return Vec::new();
    }

    let mut segments = Vec::with_capacity(count);
    let mut first = LeanString::with_capacity(count * len);
    first.push_str(&segment(0, len));
    segments.push(first);
    segments.extend((1..count).map(|index| LeanString::from(segment(index, len))));
    segments
}

fn shared_first_segments(count: usize, len: usize) -> (LeanString, Vec<LeanString>) {
    if count == 0 {
        return (LeanString::new(), Vec::new());
    }

    let owner = LeanString::from(segment(0, len));
    let mut segments = Vec::with_capacity(count);
    segments.push(owner.clone());
    segments.extend((1..count).map(|index| LeanString::from(segment(index, len))));
    (owner, segments)
}

fn move_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("move_collection");

    for count in MOVE_COLLECTION_COUNTS {
        group.bench_function(BenchmarkId::new("lean_inline", count), |b| {
            b.iter_batched(
                || lean_segments(count, 8),
                |segments| black_box(segments.into_iter().collect::<LeanString>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("string_inline", count), |b| {
            b.iter_batched(
                || (0..count).map(|index| segment(index, 8)).collect::<Vec<_>>(),
                |segments| black_box(segments.into_iter().collect::<String>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("lean_heap", count), |b| {
            b.iter_batched(
                || lean_segments(count, 32),
                |segments| black_box(segments.into_iter().collect::<LeanString>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("string_heap", count), |b| {
            b.iter_batched(
                || (0..count).map(|index| segment(index, 32)).collect::<Vec<_>>(),
                |segments| black_box(segments.into_iter().collect::<String>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("lean_empty", count), |b| {
            b.iter_batched(
                || vec![LeanString::new(); count],
                |segments| black_box(segments.into_iter().collect::<LeanString>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("lean_reserved_first", count), |b| {
            b.iter_batched(
                || reserved_first_segments(count, 32),
                |segments| black_box(segments.into_iter().collect::<LeanString>()),
                BatchSize::SmallInput,
            );
        });

        group.bench_function(BenchmarkId::new("lean_shared_first", count), |b| {
            b.iter_batched(
                || shared_first_segments(count, 32),
                |(owner, segments)| {
                    let output = segments.into_iter().collect::<LeanString>();
                    black_box(owner);
                    black_box(output)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn reserve_and_push_paths(c: &mut Criterion) {
    let mut group = c.benchmark_group("reserve_and_push_paths");

    group.bench_function("push_str/inline_capacity", |b| {
        b.iter_batched(
            || LeanString::from("inline"),
            |mut value| {
                value.push_str(black_box("x"));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("push_str/unique_heap_capacity", |b| {
        b.iter_batched(
            || {
                let mut value = LeanString::with_capacity(256);
                value.push_str("heap-backed text");
                value
            },
            |mut value| {
                value.push_str(black_box("x"));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("push_str/unique_heap_growth", |b| {
        let text = "a".repeat(64);
        b.iter_batched(
            || LeanString::from(text.as_str()),
            |mut value| {
                value.push_str(black_box("0123456789abcdef"));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("push_str/shared_detach", |b| {
        let text = "a".repeat(64);
        let original = LeanString::from(text.as_str());
        b.iter_batched(
            || (original.clone(), original.clone()),
            |(mut value, shared)| {
                value.push_str(black_box("x"));
                black_box(shared);
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("push_str/static_mutation", |b| {
        b.iter_batched(
            || LeanString::from_static_str("a static string longer than inline storage"),
            |mut value| {
                value.push_str(black_box("x"));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("reserve/inline_capacity", |b| {
        b.iter_batched(
            || LeanString::from("inline"),
            |mut value| {
                value.reserve(black_box(1));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("reserve/unique_heap_capacity", |b| {
        b.iter_batched(
            || {
                let mut value = LeanString::with_capacity(256);
                value.push_str("heap-backed text");
                value
            },
            |mut value| {
                value.reserve(black_box(1));
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(50);
    targets = move_collection, reserve_and_push_paths
}
criterion_main!(benches);
