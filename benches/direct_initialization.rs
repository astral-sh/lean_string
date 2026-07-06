use core::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use lean_string::LeanString;

const LENGTHS: [usize; 4] = [32, 256, 4096, 65_536];
const HEX: &[u8; 16] = b"0123456789abcdef";

#[inline]
fn produced_byte(index: usize, seed: usize) -> u8 {
    HEX[index.wrapping_mul(17).wrapping_add(seed) & 0xf]
}

fn direct_initialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("direct_initialization");

    for len in LENGTHS {
        group.throughput(Throughput::Bytes(len as u64));

        group.bench_function(BenchmarkId::new("reserved_push", len), |b| {
            b.iter(|| {
                let seed = black_box(3);
                let mut output = LeanString::with_capacity(len);
                for index in 0..len {
                    output.push(produced_byte(index, seed) as char);
                }
                black_box(output)
            });
        });

        group.bench_function(BenchmarkId::new("temporary_string", len), |b| {
            b.iter(|| {
                let seed = black_box(3);
                let mut temporary = String::with_capacity(len);
                for index in 0..len {
                    temporary.push(produced_byte(index, seed) as char);
                }
                black_box(LeanString::from(temporary))
            });
        });

        group.bench_function(BenchmarkId::new("temporary_vec", len), |b| {
            b.iter(|| {
                let seed = black_box(3);
                let mut temporary = Vec::with_capacity(len);
                for index in 0..len {
                    temporary.push(produced_byte(index, seed));
                }
                black_box(LeanString::from_utf8(&temporary).unwrap())
            });
        });

        group.bench_function(BenchmarkId::new("direct_final_storage", len), |b| {
            b.iter(|| {
                let seed = black_box(3);
                // SAFETY: Every slot is initialized, and every produced byte is ASCII.
                let output = unsafe {
                    LeanString::try_from_utf8_unchecked_with(len, |buffer| {
                        for (index, slot) in buffer.iter_mut().enumerate() {
                            slot.write(produced_byte(index, seed));
                        }
                    })
                }
                .unwrap();
                black_box(output)
            });
        });
    }

    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(2))
        .sample_size(50);
    targets = direct_initialization
}
criterion_main!(benches);
