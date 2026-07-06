use core::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use lean_string::LeanString;

const STRING_LENGTHS: [usize; 10] = [11, 12, 13, 15, 16, 17, 24, 32, 48, 64];
const VECTOR_LEN: usize = 16_384;

fn construction(c: &mut Criterion) {
    let mut construction = c.benchmark_group("compact_header/construction");

    for len in STRING_LENGTHS {
        let text = "a".repeat(len);

        construction.throughput(Throughput::Elements(VECTOR_LEN as u64));
        construction.bench_function(BenchmarkId::from_parameter(len), |b| {
            b.iter(|| {
                black_box(
                    (0..VECTOR_LEN)
                        .map(|_| LeanString::from(black_box(text.as_str())))
                        .collect::<Vec<_>>(),
                )
            });
        });
    }

    construction.finish();
}

fn clone_drop(c: &mut Criterion) {
    let mut clone_drop = c.benchmark_group("compact_header/clone_drop");

    for len in STRING_LENGTHS {
        let text = "a".repeat(len);
        let values = (0..VECTOR_LEN).map(|_| LeanString::from(text.as_str())).collect::<Vec<_>>();

        clone_drop.throughput(Throughput::Elements(VECTOR_LEN as u64));
        clone_drop.bench_function(BenchmarkId::from_parameter(len), |b| {
            b.iter(|| black_box(black_box(&values).clone()));
        });
    }

    clone_drop.finish();
}

fn traversal(c: &mut Criterion) {
    let mut traversal = c.benchmark_group("compact_header/traversal");

    for len in STRING_LENGTHS {
        let text = "a".repeat(len);
        let values = (0..VECTOR_LEN).map(|_| LeanString::from(text.as_str())).collect::<Vec<_>>();

        traversal.throughput(Throughput::Bytes((VECTOR_LEN * len) as u64));
        traversal.bench_function(BenchmarkId::from_parameter(len), |b| {
            b.iter(|| {
                black_box(&values)
                    .iter()
                    .map(|value| usize::from(value.as_bytes()[0]) + value.len())
                    .sum::<usize>()
            });
        });
    }

    traversal.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(50);
    targets = construction, clone_drop, traversal
}
criterion_main!(benches);
