#[path = "runtime_gather/support.rs"]
mod support;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;
use support::{Input, Kernels, Mode, VALUE_CAPACITY};

fn gather(c: &mut Criterion) {
    let kernels = Kernels::new();
    let datasets = (0..100)
        .map(|row| {
            (0..VALUE_CAPACITY)
                .map(|column| ((column * 31 + row * 17) % 256) as u8)
                .collect::<Vec<_>>()
                .into_boxed_slice()
                .try_into()
                .unwrap()
        })
        .collect::<Vec<Box<[u8; VALUE_CAPACITY]>>>();
    let mut group = c.benchmark_group("runtime_gather");
    group.sample_size(30);
    for count in [64, 10_000] {
        let mut state = 1_337u32;
        let indices = (0..count)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) % VALUE_CAPACITY as u32) as u16
            })
            .collect::<Vec<_>>();
        let input = Input::new(&indices);
        let plan = kernels.prepare(&input, VALUE_CAPACITY as u16).unwrap();
        assert_eq!(plan.indices(), indices);
        assert_eq!(plan.extents(), vec![VALUE_CAPACITY as u16; count]);
        for values in &datasets {
            let expected = input
                .indices()
                .iter()
                .map(|index| u64::from(values[usize::from(*index)]))
                .sum::<u64>();
            for mode in [Mode::CheckedCompact, Mode::CheckedWitness, Mode::Prepared] {
                assert_eq!(kernels.selected_sum(mode)(&plan, &input, values), expected);
            }
        }
        for steps in [1, 100] {
            group.throughput(Throughput::Elements((count * steps) as u64));
            let order = if std::env::var_os("GIN_RUNTIME_GATHER_PREPARED_FIRST").is_some() {
                [
                    ("prepared", Mode::Prepared),
                    ("checked_witness", Mode::CheckedWitness),
                    ("checked_compact", Mode::CheckedCompact),
                ]
            } else {
                [
                    ("checked_compact", Mode::CheckedCompact),
                    ("checked_witness", Mode::CheckedWitness),
                    ("prepared", Mode::Prepared),
                ]
            };
            let modes = order
                .into_iter()
                .map(|(name, mode)| (name, mode, false))
                .chain([("prepare_and_reuse", Mode::Prepared, true)]);
            for (name, mode, include_preparation) in modes {
                let sum = kernels.selected_sum(mode);
                group.bench_function(BenchmarkId::new(name, format!("{count}x{steps}")), |b| {
                    b.iter(|| {
                        let fresh = include_preparation.then(|| {
                            kernels
                                .prepare(black_box(&input), black_box(VALUE_CAPACITY as u16))
                                .unwrap()
                        });
                        let selected = fresh.as_ref().unwrap_or(&plan);
                        let mut total = 0;
                        for dataset in datasets.iter().take(steps) {
                            total +=
                                sum(black_box(selected), black_box(&input), black_box(dataset));
                        }
                        black_box(total)
                    });
                });
            }
        }
    }
    group.finish();
}

criterion_group!(benches, gather);
criterion_main!(benches);
