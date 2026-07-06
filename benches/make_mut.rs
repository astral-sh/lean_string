use core::hint::black_box;
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lean_string::LeanString;

const INLINE_TEXT: &str = "12345678";
const HEAP_TEXT: &str = "a mutable string longer than inline storage";

fn make_mut(c: &mut Criterion) {
    let mut group = c.benchmark_group("make_mut");

    group.bench_function("inline", |b| {
        b.iter_batched(
            || LeanString::from(INLINE_TEXT),
            |mut value| {
                black_box(value.make_mut().as_ptr());
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("unique_heap", |b| {
        b.iter_batched(
            || LeanString::from(HEAP_TEXT),
            |mut value| {
                black_box(value.make_mut().as_ptr());
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    let retained_owner = LeanString::from(HEAP_TEXT);
    group.bench_function("shared_heap_retained_owner", |b| {
        b.iter_batched(
            || retained_owner.clone(),
            |mut value| {
                black_box(value.make_mut().as_ptr());
                black_box(value)
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function("static", |b| {
        b.iter_batched(
            || LeanString::from_static_str(HEAP_TEXT),
            |mut value| {
                black_box(value.make_mut().as_ptr());
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
    targets = make_mut
}
criterion_main!(benches);
