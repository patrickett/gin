use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use interface::{
    DeclarationRef, Fingerprint, InterfaceDeclaration, PackageInstanceId, PublicInterface,
};
use std::hint::black_box;

fn subject() -> PackageInstanceId {
    PackageInstanceId {
        package: "benchmark".to_string(),
        version: "1.0.0".to_string(),
        source: "workspace".to_string(),
        instance: "root".to_string(),
    }
}

fn declarations(count: usize) -> Vec<InterfaceDeclaration> {
    (0..count)
        .map(|index| InterfaceDeclaration {
            reference: DeclarationRef::Subject {
                module_path: format!("module.{}", index / 64),
                declaration_path: format!("Declaration{index}"),
            },
            fingerprint: Fingerprint::from_bytes(&index.to_le_bytes()),
        })
        .collect()
}

fn benchmark_interface(c: &mut Criterion) {
    let mut group = c.benchmark_group("interface_encoding_and_fingerprinting");
    for count in [1_000, 10_000] {
        let declarations = declarations(count);
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &declarations,
            |bencher, declarations| {
                bencher.iter(|| {
                    let interface = PublicInterface::from_subject_and_declarations(
                        subject(),
                        declarations.clone(),
                    );
                    black_box(interface.canonical_bytes())
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, benchmark_interface);
criterion_main!(benches);
