use flask::{FlaskConfig, MinimumRequirement, Registry, Release, ReleaseStatus, SecurityPolicy};
use semver::Version;
use std::{collections::BTreeMap, hint::black_box, time::Instant};

fn main() {
    let manifest = format!(
        "#settings\nmodule: 'app'\nversion: 0.1.0\n#dependencies\n{}",
        (0..100)
            .map(|n| format!("package_{n}: 1.0.0\n"))
            .collect::<String>()
    );
    measure("mod.flask parse (100 dependencies)", 1000, || {
        black_box(FlaskConfig::parse_manifest(black_box(&manifest)).unwrap());
    });
    for count in [10, 100, 1000] {
        let registry = catalog(count, 0);
        selection(&registry, &format!("MVS ({count} reachable releases)"));
    }
    let registry = catalog(100, 10_000);
    selection(&registry, "MVS (100 reachable + 10000 unrelated releases)");
}

fn catalog(count: usize, unrelated: usize) -> Registry {
    let mut registry = Registry::default();
    for index in 0..count + unrelated {
        let name = format!("package_{index}");
        let dependencies = if index + 1 < count {
            BTreeMap::from([(
                "next".into(),
                MinimumRequirement::parse("next", &format!("package_{}@1.0.0", index + 1)).unwrap(),
            )])
        } else {
            BTreeMap::new()
        };
        registry
            .insert(Release {
                name: name.clone(),
                version: Version::new(1, 0, 0),
                archive_sha256: "a".repeat(64),
                interface_sha256: "b".repeat(64),
                published_at: 0,
                dependencies: BTreeMap::new(),
                minimum_dependencies: Some(dependencies),
            })
            .unwrap();
        registry
            .set_status(&name, &Version::new(1, 0, 0), ReleaseStatus::Eligible)
            .unwrap();
    }
    registry
}

fn selection(registry: &Registry, label: &str) {
    let requirements = BTreeMap::from([(
        "package_0".into(),
        MinimumRequirement::parse("package_0", "1.0.0").unwrap(),
    )]);
    let policy = SecurityPolicy {
        now: 300_000,
        metadata_expires_at: 400_000,
        cooldown_seconds: 259_200,
    };
    measure(label, 1000, || {
        black_box(
            registry
                .resolve_minimum(black_box(&requirements), policy)
                .unwrap(),
        );
    });
}

fn measure(label: &str, iterations: u32, mut run: impl FnMut()) {
    for _ in 0..20 {
        run();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        run();
    }
    println!(
        "{label}: {:?}/iteration ({iterations} iterations)",
        start.elapsed() / iterations
    );
}
