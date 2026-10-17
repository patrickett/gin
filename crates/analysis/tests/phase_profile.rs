use analysis::PackageCache;
use compiler_profile::{Report, capture};
use test_fixtures::TempPackage;
use typecheck::transform::{BodyReuse, PackageCheckSession};

fn measured(report: &Report, name: &str) -> bool {
    report
        .phases
        .iter()
        .any(|phase| phase.name == name && phase.calls > 0)
}

#[test]
fn cold_edit_and_cache_hit_profile_the_shared_frontend() {
    for workers in [1, 2, 4] {
        let package = TempPackage::new("profile_editor");
        package.write_flask("profile_editor");
        let types = package.write(
            "types.gin",
            "Word is in 0 to 255\nWord has finish(self): return\n",
        );
        package.write("math/marker.gin", "marker: 1\n");
        let body = package.write(
            "body.gin",
            "use './math' marker\nidentity(x Word) Word: return x\n",
        );
        let paths = vec![types, body.clone()];
        let session = PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
            .unwrap()
            .with_function_reuse()
            .with_effect_queries()
            .with_parallel_references()
            .with_finishing_reuse()
            .with_retained_pass_limit(1.try_into().unwrap());
        let (tx, _) = crossbeam_channel::unbounded();
        let cache = PackageCache::with_check_session(tx, session).with_resolved_result_reuse();
        let (cold, report) = capture(|| {
            for path in &paths {
                cache.add_file(path.clone()).unwrap();
            }
            cache.get_or_compute_package(&paths).unwrap()
        });
        assert!(measured(&report, "parse"));
        assert!(
            report
                .phases
                .iter()
                .find(|phase| phase.name == "parse")
                .unwrap()
                .calls
                >= 3
        );
        assert!(measured(&report, "prepare"));
        assert!(measured(&report, "front-end"));
        assert!(measured(&report, "flow"));
        assert!(
            cold.typecheck_outputs
                .iter()
                .flatten()
                .all(|d| d.category != diagnostic::Category::Flaw)
        );
        let (_, hit) = capture(|| cache.get_or_compute_package(&paths).unwrap());
        assert!(!measured(&hit, "package-pass"));
        let (edited, edit) = capture(|| {
            cache.set_contents(
                &body,
                "use './math' marker\nidentity(x Word) Word: return 1\n".to_owned(),
            );
            cache.get_or_compute_package(&paths).unwrap()
        });
        assert!(measured(&edit, "references"));
        assert!(measured(&edit, "effects"));
        let fresh = PackageCache::for_test();
        for path in &paths {
            fresh.add_file(path.clone()).unwrap();
        }
        fresh.set_contents(
            &body,
            "use './math' marker\nidentity(x Word) Word: return 1\n".to_owned(),
        );
        let expected = fresh.get_or_compute_package(&paths).unwrap();
        assert_eq!(edited.typed_asts, expected.typed_asts);
        assert_eq!(edited.typecheck_outputs, expected.typecheck_outputs);
        assert_eq!(edited.public_interface, expected.public_interface);
    }
}
