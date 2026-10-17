use super::session_tests::{WORD, clean, equivalent, session_cache, session_cache_with_reuse};
use super::*;
use typecheck::transform::BodyReuse;

#[test]
fn exact_results_survive_invalidation_and_release_on_eviction() {
    for base in [
        PackageCache::for_test(),
        session_cache(4),
        session_cache_with_reuse(4, BodyReuse::Disabled),
    ] {
        let package = test_fixtures::TempPackage::new("resolved_result_lifetime");
        package.write_flask("fixture");
        let paths = [
            package.write("types.gin", WORD),
            package.write("main.gin", "one Word: 1\n"),
        ];
        let cache = base.with_resolved_result_reuse();
        for path in &paths {
            cache.add_file(path.clone()).unwrap();
        }
        let first = cache.get_or_compute_package(&paths).unwrap();
        assert_eq!(cache.resolved_result_reused(&paths[0]), Some(false));
        cache.invalidate_package_cache_for(&paths[0]);
        let second = cache.get_or_compute_package(&paths).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.resolved_result_reused(&paths[0]), Some(true));
        equivalent(&second, &clean(&paths));
        let edited = "\n\n--| moved definition\none Word: 1\n";
        std::fs::write(&paths[1], edited).unwrap();
        cache.set_contents(&paths[1], edited.to_owned());
        let moved = cache.get_or_compute_package(&paths).unwrap();
        assert!(!Arc::ptr_eq(&first, &moved));
        equivalent(&moved, &clean(&paths));
        assert_eq!(cache.resolved_result_reused(&paths[0]), Some(false));
        cache.evict_package_cache_for(&paths[0]);
        assert_eq!(cache.resolved_result_reused(&paths[0]), None);
        let reopened = cache.get_or_compute_package(&paths).unwrap();
        assert!(!Arc::ptr_eq(&moved, &reopened));
        for path in &paths {
            assert!(cache.evict_file(path));
        }
        assert!(
            cache
                .resolved_results
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn body_signature_overload_and_missing_name_edits_match_clean_results() {
    for reuse in [BodyReuse::Disabled, BodyReuse::ExactInputs] {
        for workers in [1, 2, 4, 8] {
            for reversed in [false, true] {
                let package = test_fixtures::TempPackage::new("resolved_result_edits");
                package.write_flask("fixture");
                let types = package.write("types.gin", WORD);
                let provider = package.write("provider.gin", "helper(x ref Word) Word: return 1\n");
                let caller = package.write(
                    "caller.gin",
                    "consumer(x ref Word) Word: return helper(x)\n",
                );
                let mut paths = vec![types, provider.clone(), caller];
                if reversed {
                    paths.reverse();
                }
                let cache = session_cache_with_reuse(workers, reuse).with_resolved_result_reuse();
                for path in &paths {
                    cache.add_file(path.clone()).unwrap();
                }
                let mut previous = cache.get_or_compute_package(&paths).unwrap();
                for source in [
                    "helper(x ref Word) Word: return 2\n",
                    "helper(x ref Word, y ref Word) Word: return 2\n",
                    "helper(x ref Word, y ref Word) Word: return x\n",
                    "helper(y ref Word, x ref Word) Word: return x\n",
                    "helper(x ref Word) Word: return 2\nhelper(x Word) Word: return 3\n",
                    "absent(x ref Word) Word: return 2\n",
                    "helper(x ref Word) Word: return 1\n",
                ] {
                    std::fs::write(&provider, source).unwrap();
                    cache.set_contents(&provider, source.to_owned());
                    let current = cache.get_or_compute_package(&paths).unwrap();
                    assert!(!Arc::ptr_eq(&previous, &current));
                    equivalent(&current, &clean(&paths));
                    if reuse == BodyReuse::Disabled {
                        let stats = cache.check_stats(&provider).unwrap();
                        assert_eq!(stats.reused_files, 0);
                        assert!(stats.lowered_files > 0);
                    }
                    cache.invalidate_package_cache_for(&provider);
                    let confirmed = cache.get_or_compute_package(&paths).unwrap();
                    assert!(Arc::ptr_eq(&current, &confirmed));
                    assert_eq!(cache.resolved_result_reused(&provider), Some(true));
                    previous = confirmed;
                }
            }
        }
    }
}

#[test]
fn unused_scope_edits_can_reuse_results_but_new_candidates_must_recompute() {
    let package = test_fixtures::TempPackage::new("resolved_result_scope");
    package.write_flask("fixture");
    package.write(
        "math/math.gin",
        &format!("{WORD}compute(value is in Word) is in Word: return 1\n"),
    );
    let path = package.write("main.gin", "use './math' compute\nanswer: compute(0)\n");
    let paths = [path.clone()];
    let cache = PackageCache::for_test_with_file(path.clone()).with_resolved_result_reuse();
    let first = cache.get_or_compute_package(&paths).unwrap();
    let unused = package.write("math/unused.gin", "Unused is Left or Right\n");
    cache.invalidate_package_cache_for(&unused);
    let unchanged = cache.get_or_compute_package(&paths).unwrap();
    equivalent(&unchanged, &clean(&paths));
    assert!(Arc::ptr_eq(&first, &unchanged));
    let missing = "use './math' absent\nanswer: absent(0)\n";
    std::fs::write(&path, missing).unwrap();
    cache.set_contents(&path, missing.to_owned());
    let unresolved = cache.get_or_compute_package(&paths).unwrap();
    assert!(!unresolved.import_diags.is_empty());
    std::fs::write(
        &unused,
        format!("{WORD}absent(value is in Word) is in Word: return 2\n"),
    )
    .unwrap();
    cache.invalidate_package_cache_for(&unused);
    let resolved = cache.get_or_compute_package(&paths).unwrap();
    equivalent(&resolved, &clean(&paths));
    assert!(!Arc::ptr_eq(&unresolved, &resolved));
    assert!(resolved.import_diags.is_empty());
}

#[test]
fn compile_time_implementations_and_target_inputs_are_not_signature_only_keys() {
    for reuse in [BodyReuse::Disabled, BodyReuse::ExactInputs] {
        let package = test_fixtures::TempPackage::new("resolved_result_comptime");
        package.write_flask("fixture");
        let provider = package.write(
            "math/math.gin",
            &format!("{WORD}compute(value is in Word) is in Word: return 1\n"),
        );
        let path = package.write("main.gin", "use './math' compute\nanswer: compute(0)\n");
        let paths = [path.clone()];
        let cache = session_cache_with_reuse(4, reuse).with_resolved_result_reuse();
        cache.add_file(path.clone()).unwrap();
        let first = cache.get_or_compute_package(&paths).unwrap();
        std::fs::write(
            &provider,
            format!("{WORD}compute(value is in Word) is in Word: return 2\n"),
        )
        .unwrap();
        cache.invalidate_package_cache_for(&provider);
        let changed = cache.get_or_compute_package(&paths).unwrap();
        assert!(!Arc::ptr_eq(&first, &changed));
        assert_eq!(
            first.public_interface.resolution_fingerprint,
            changed.public_interface.resolution_fingerprint
        );
        equivalent(&changed, &clean(&paths));
        for target in ["wasm32-unknown-unknown", "x86_64-unknown-linux-gnu"] {
            package.write(
                "mod.flask",
                &format!(
                    r#"#settings
module: "fixture"
version: 0.0.0
target: "{target}"

#dependencies
"#
                ),
            );
            let current = cache.get_or_compute_package(&paths).unwrap();
            assert!(!Arc::ptr_eq(&changed, &current));
            equivalent(&current, &clean(&paths));
            assert_eq!(cache.resolved_result_reused(&path), Some(false));
        }
    }
}

#[test]
fn stale_results_cannot_update_retained_inputs_or_dependency_state() {
    let package = test_fixtures::TempPackage::new("resolved_result_stale");
    package.write_flask("fixture");
    let path = package.write("main.gin", WORD);
    let cache = PackageCache::for_test_with_file(path.clone()).with_resolved_result_reuse();
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    let identity = cache.file_identity(&path).unwrap();
    let revision = cache.revision.load(Ordering::SeqCst);
    let pending = cache
        .compute_package(
            revision,
            &identity,
            std::slice::from_ref(&path),
            CompileTarget::Library,
            &old.configuration,
            &old.registry_resolution,
        )
        .unwrap();
    assert!(pending.reused);
    let edited = format!("{WORD}\nmissing: unknown\n");
    std::fs::write(&path, &edited).unwrap();
    cache.set_contents(&path, edited);
    assert!(cache.publish_package(identity, revision, pending).is_none());
    let fresh = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    assert!(!Arc::ptr_eq(&old, &fresh));
    equivalent(&fresh, &clean(std::slice::from_ref(&path)));
    assert_eq!(cache.resolved_result_reused(&path), Some(false));
}

#[test]
fn changed_inputs_release_cache_ownership_before_publication() {
    let package = test_fixtures::TempPackage::new("resolved_result_release");
    package.write_flask("fixture");
    let path = package.write("main.gin", &format!("{WORD}\none Word: 1\n"));
    let cache = PackageCache::for_test_with_file(path.clone()).with_resolved_result_reuse();
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    let old_lifetime = Arc::downgrade(&old);
    let configuration = old.configuration.clone();
    let resolution = old.registry_resolution.clone();
    let edited = format!("{WORD}\none Word: 2\n");
    std::fs::write(&path, &edited).unwrap();
    cache.set_contents(&path, edited);
    let identity = cache.file_identity(&path).unwrap();
    let revision = cache.revision.load(Ordering::SeqCst);
    let mut pending = cache
        .compute_package(
            revision,
            &identity,
            std::slice::from_ref(&path),
            CompileTarget::Library,
            &configuration,
            &resolution,
        )
        .unwrap();
    assert!(!pending.reused);
    assert_eq!(cache.resolved_result_reused(&path), None);
    assert_eq!(Arc::strong_count(&old), 1);
    assert!(old_lifetime.upgrade().is_some());
    drop(old);
    assert!(old_lifetime.upgrade().is_none());
    let fresh = Arc::get_mut(&mut pending.entry).unwrap();
    fresh.configuration = configuration;
    fresh.registry_resolution = resolution;
    let published = cache.publish_package(identity, revision, pending).unwrap();
    equivalent(&published, &clean(std::slice::from_ref(&path)));
    cache.invalidate_package_cache_for(&path);
    let confirmed = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    assert!(Arc::ptr_eq(&published, &confirmed));
}

#[test]
fn pending_confirmation_owns_the_input_result_pair() {
    let package = test_fixtures::TempPackage::new("resolved_result_confirmation");
    package.write_flask("fixture");
    let path = package.write("main.gin", WORD);
    let cache = PackageCache::for_test_with_file(path.clone()).with_resolved_result_reuse();
    let old = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    cache.invalidate_package_cache_for(&path);
    let identity = cache.file_identity(&path).unwrap();
    let revision = cache.revision.load(Ordering::SeqCst);
    let pending = cache
        .compute_package(
            revision,
            &identity,
            std::slice::from_ref(&path),
            CompileTarget::Library,
            &old.configuration,
            &old.registry_resolution,
        )
        .unwrap();
    assert!(pending.reused);
    cache
        .resolved_results
        .as_ref()
        .unwrap()
        .lock()
        .unwrap()
        .remove(&identity);
    let published = cache.publish_package(identity, revision, pending).unwrap();
    assert!(Arc::ptr_eq(&old, &published));
    assert_eq!(cache.resolved_result_reused(&path), Some(true));
    cache.invalidate_package_cache_for(&path);
    let confirmed = cache
        .get_or_compute_package(std::slice::from_ref(&path))
        .unwrap();
    assert!(Arc::ptr_eq(&published, &confirmed));
    equivalent(&confirmed, &clean(std::slice::from_ref(&path)));
}
