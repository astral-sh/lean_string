use core::hint::black_box;
use core::ops::Range;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use lean_string::{LeanString, LeanSubstring};

const CORPUS_LEN: usize = 1024 * 1024;
const TOKEN_LENGTHS: [usize; 8] = [24, 32, 48, 64, 96, 128, 192, 256];

fn corpus_and_ranges() -> (String, Vec<Range<usize>>) {
    let mut corpus = String::with_capacity(CORPUS_LEN);
    let mut ranges = Vec::new();
    let mut index = 0;

    while corpus.len() < CORPUS_LEN {
        let remaining = CORPUS_LEN - corpus.len();
        let desired = TOKEN_LENGTHS[index % TOKEN_LENGTHS.len()];
        let token_len = desired.min(remaining);
        let start = corpus.len();
        let byte = b'a' + (index % 26) as u8;
        corpus.extend(core::iter::repeat_n(char::from(byte), token_len));
        ranges.push(start..corpus.len());
        index += 1;

        if corpus.len() < CORPUS_LEN {
            corpus.push(' ');
        }
    }

    (corpus, ranges)
}

fn materialize(corpus: &str, ranges: &[Range<usize>]) -> Vec<LeanString> {
    ranges.iter().map(|range| LeanString::from(&corpus[range.clone()])).collect()
}

fn make_views(owner: &LeanString, ranges: &[Range<usize>]) -> Vec<LeanSubstring> {
    ranges.iter().map(|range| owner.substring(range.clone()).unwrap()).collect()
}

fn make_views_from_str(corpus: &str, ranges: &[Range<usize>]) -> Vec<LeanSubstring> {
    let owner = LeanString::from(corpus);
    make_views(&owner, ranges)
}

fn scan<T: AsRef<str>>(strings: &[T]) -> usize {
    strings.iter().fold(0, |sum, string| {
        let bytes = string.as_ref().as_bytes();
        sum.wrapping_add(bytes.len())
            .wrapping_add(usize::from(bytes[0]))
            .wrapping_add(usize::from(bytes[bytes.len() - 1]))
    })
}

fn hash<T: Hash>(strings: &[T]) -> u64 {
    let mut hasher = DefaultHasher::new();
    strings.hash(&mut hasher);
    hasher.finish()
}

fn substring_workloads(c: &mut Criterion) {
    let (corpus, ranges) = corpus_and_ranges();
    assert_eq!(corpus.len(), CORPUS_LEN);
    assert!(ranges.len() > 8_000);
    let allocated = materialize(&corpus, &ranges);
    let owner = LeanString::from(corpus.as_str());
    let views = make_views(&owner, &ranges);

    let mut build = c.benchmark_group("substring/build");
    build.throughput(Throughput::Bytes(CORPUS_LEN as u64));
    build.bench_function("allocated_lean_strings", |b| {
        b.iter(|| black_box(materialize(black_box(&corpus), black_box(&ranges))));
    });
    build.bench_function("views_existing_owner", |b| {
        b.iter(|| black_box(make_views(black_box(&owner), black_box(&ranges))));
    });
    build.bench_function("views_from_str", |b| {
        b.iter(|| black_box(make_views_from_str(black_box(&corpus), black_box(&ranges))));
    });
    build.finish();

    let mut access = c.benchmark_group("substring/access");
    access.throughput(Throughput::Elements(ranges.len() as u64));
    access.bench_function("scan_allocated", |b| {
        b.iter(|| black_box(scan(black_box(&allocated))));
    });
    access.bench_function("hash_allocated", |b| {
        b.iter(|| black_box(hash(black_box(&allocated))));
    });
    access.bench_function("scan_views", |b| {
        b.iter(|| black_box(scan(black_box(&views))));
    });
    access.bench_function("hash_views", |b| {
        b.iter(|| black_box(hash(black_box(&views))));
    });
    access.finish();

    let mut ownership = c.benchmark_group("substring/ownership");
    ownership.throughput(Throughput::Elements(ranges.len() as u64));
    ownership.bench_function("clone_drop_allocated", |b| {
        b.iter(|| black_box(allocated.clone()));
    });
    ownership.bench_function("clone_drop_views", |b| {
        b.iter(|| black_box(views.clone()));
    });
    ownership.bench_function("sort_allocated", |b| {
        b.iter_batched(
            || allocated.clone(),
            |mut values| {
                values.sort_unstable();
                black_box(values)
            },
            BatchSize::SmallInput,
        );
    });
    ownership.bench_function("sort_views", |b| {
        b.iter_batched(
            || views.clone(),
            |mut values| {
                values.sort_unstable();
                black_box(values)
            },
            BatchSize::SmallInput,
        );
    });
    ownership.finish();

    let tiny = ranges[ranges.len() / 2].start;
    let tiny = tiny..tiny + 8;
    c.bench_function("substring/retain_tiny/allocated", |b| {
        b.iter(|| black_box(LeanString::from(&corpus[black_box(tiny.clone())])));
    });
    c.bench_function("substring/retain_tiny/view", |b| {
        b.iter(|| black_box(owner.substring(black_box(tiny.clone())).unwrap()));
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500))
        .sample_size(20);
    targets = substring_workloads
}
criterion_main!(benches);
