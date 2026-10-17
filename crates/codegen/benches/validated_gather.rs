#[path = "gather_plan/support.rs"]
mod support;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use support::{CAPACITY, Kernels};

fn gather(c: &mut Criterion) {
    let kernels = Kernels::new();
    let datasets: [[u8; CAPACITY]; 64] = std::array::from_fn(|row| {
        std::array::from_fn(|column| ((column * 31 + row * 17) % 256) as u8)
    });
    let indices = [13, 257, 511, 1023];
    let plan = kernels.prepare(CAPACITY as u16, indices).unwrap();
    assert_eq!(usize::from(plan.length()), CAPACITY);
    assert_eq!(plan.indices(), indices);
    for values in &datasets {
        let expected = indices
            .iter()
            .map(|index| u64::from(values[usize::from(*index)]))
            .sum::<u64>();
        assert_eq!(kernels.sum_prepared(&plan, values), expected);
        assert_eq!(kernels.sum_checked(&plan, values), expected);
    }

    let mut group = c.benchmark_group("gather_plan");
    group.sample_size(30);
    for repeats in [1usize, 64, 4096] {
        group.throughput(Throughput::Elements((repeats * 4) as u64));
        let order = if std::env::var_os("GIN_GATHER_PREPARED_FIRST").is_some() {
            [true, false]
        } else {
            [false, true]
        };
        let modes = order
            .map(|prepared| {
                (
                    if prepared { "prepared" } else { "checked" },
                    prepared,
                    false,
                )
            })
            .into_iter()
            .chain([("prepare_and_reuse", true, true)]);
        for (name, prepared, include_preparation) in modes {
            let sum = kernels.selected_sum(prepared);
            group.bench_with_input(BenchmarkId::new(name, repeats), &repeats, |b, &repeats| {
                b.iter(|| {
                    let fresh_plan = include_preparation.then(|| {
                        kernels
                            .prepare(black_box(CAPACITY as u16), black_box(indices))
                            .unwrap()
                    });
                    let selected = fresh_plan.as_ref().unwrap_or(&plan);
                    let mut total = 0;
                    for row in 0..repeats {
                        total += sum(
                            black_box(selected),
                            black_box(&datasets[row % datasets.len()]),
                        );
                    }
                    black_box(total)
                });
            });
        }
    }
    group.finish();
}

criterion_group!(benches, gather);
criterion_main!(benches);
