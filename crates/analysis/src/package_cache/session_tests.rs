use super::*;
use typecheck::transform::BodyReuse;

pub(super) const WORD: &str = test_fixtures::preludes::WORD_PRELUDE;

pub(super) fn session_cache(workers: usize) -> PackageCache {
    session_cache_with_reuse(workers, BodyReuse::ExactInputs)
}

pub(super) fn session_cache_with_reuse(workers: usize, reuse: BodyReuse) -> PackageCache {
    let (tx, _rx) = crossbeam_channel::unbounded();
    let session = PackageCheckSession::new(workers.try_into().unwrap(), reuse).unwrap();
    PackageCache::with_check_session(tx, session)
}

pub(super) fn equivalent(actual: &PackageEntry, expected: &PackageEntry) {
    assert_eq!(actual.file_paths, expected.file_paths);
    assert_eq!(actual.public_interface, expected.public_interface);
    assert_eq!(actual.typed_asts, expected.typed_asts);
    assert_eq!(actual.prepare_diags, expected.prepare_diags);
    assert_eq!(actual.typecheck_outputs, expected.typecheck_outputs);
    assert_eq!(actual.import_diags, expected.import_diags);
    for (actual, expected) in actual.typed_asts.iter().zip(&expected.typed_asts) {
        assert_eq!(actual.warnings, expected.warnings);
        assert_eq!(actual.parse_warnings, expected.parse_warnings);
        assert_eq!(actual.import_mod_paths, expected.import_mod_paths);
        assert_eq!(actual.imported_trait_names, expected.imported_trait_names);
        assert_eq!(actual.eval_ast, expected.eval_ast);
        assert_eq!(actual.type_registry, expected.type_registry);
        assert_eq!(actual.target_layout, expected.target_layout);
        assert_eq!(actual.span_table, expected.span_table);
        assert_eq!(actual.places, expected.places);
        assert_eq!(actual.place_versions, expected.place_versions);
        assert_eq!(
            actual.place_version_components,
            expected.place_version_components
        );
        assert_eq!(actual.constant_objects, expected.constant_objects);
    }
}

pub(super) fn clean(paths: &[PathBuf]) -> Arc<PackageEntry> {
    let cache = PackageCache::for_test();
    for path in paths {
        cache.add_file(path.clone()).unwrap();
    }
    cache.get_or_compute_package(paths).unwrap()
}

#[test]
fn edits_and_file_orders_match_clean_editor_analysis() {
    for workers in [1, 2, 4, 8] {
        for reverse in [false, true] {
            let package = test_fixtures::TempPackage::new("editor_session_edits");
            package.write_flask("fixture");
            let types = package.write("000-types.gin", WORD);
            let producer = package.write(
                "100-producer.gin",
                "helper(x ref Word) Word: return 1\nprivate\nlocal(x ref Word) Word: return 1\n",
            );
            let consumer = package.write(
                "200-consumer.gin",
                "consumer(x ref Word) Word: return helper(x)\n",
            );
            let mut paths = vec![types, producer.clone(), consumer.clone()];
            if reverse {
                paths.reverse();
            }
            let cache = session_cache(workers);
            for path in &paths {
                cache.add_file(path.clone()).unwrap();
            }
            equivalent(
                &cache.get_or_compute_package(&paths).unwrap(),
                &clean(&paths),
            );
            for source in [
                "helper(x ref Word) Word: return 1\nprivate\nlocal(x ref Word) Word: return 2\n",
                "helper(x ref Word, y ref Word) Word: return 1\nprivate\nlocal(x ref Word) Word: return 2\n",
                "helper(x ref Word) Word: return 2\nhelper(x Word) Word: return 3\nprivate\nlocal(x ref Word) Word: return 2\n",
                "absent(x ref Word) Word: return 2\n",
                "helper(x ref Word) Word: return 2\n",
            ] {
                std::fs::write(&producer, source).unwrap();
                cache.set_contents(&producer, source.to_owned());
                equivalent(
                    &cache.get_or_compute_package(&paths).unwrap(),
                    &clean(&paths),
                );
                let stats = cache.check_stats(&producer).unwrap();
                assert_eq!(
                    stats.lowered_files + stats.reused_files,
                    stats.declared_files
                );
            }
            cache.invalidate_package_cache_for(&consumer);
            equivalent(
                &cache.get_or_compute_package(&paths).unwrap(),
                &clean(&paths),
            );
            assert!(cache.check_stats(&consumer).unwrap().reused_files > 0);
            for path in &paths {
                assert!(cache.evict_file(path));
            }
            assert!(
                cache
                    .checking
                    .as_ref()
                    .unwrap()
                    .packages
                    .lock()
                    .unwrap()
                    .is_empty()
            );
        }
    }
}

#[test]
fn package_eviction_releases_checks_while_retaining_registered_sources() {
    let package = test_fixtures::TempPackage::new("editor_session_eviction");
    package.write_flask("fixture");
    let paths = [
        package.write("types.gin", WORD),
        package.write("main.gin", "one Word: 1\n"),
    ];
    let cache = session_cache(4);
    for path in &paths {
        cache.add_file(path.clone()).unwrap();
    }
    equivalent(
        &cache.get_or_compute_package(&paths).unwrap(),
        &clean(&paths),
    );
    cache.invalidate_package_cache_for(&paths[0]);
    equivalent(
        &cache.get_or_compute_package(&paths).unwrap(),
        &clean(&paths),
    );
    assert!(cache.check_stats(&paths[0]).unwrap().reused_files > 0);
    cache.evict_package_cache_for(&paths[0]);
    assert_eq!(cache.cache_counts(), (2, 0, 0, 0));
    assert!(cache.check_stats(&paths[0]).is_none());
    equivalent(
        &cache.get_or_compute_package(&paths).unwrap(),
        &clean(&paths),
    );
    assert_eq!(cache.check_stats(&paths[0]).unwrap().reused_files, 0);
}

#[test]
fn flawed_packages_publish_diagnostics_in_stable_source_order() {
    let package = test_fixtures::TempPackage::new("editor_diagnostic_order");
    package.write_flask("fixture");
    let mut source = "choose(value) True or False: return True\n".to_owned();
    for index in 0..16 {
        source.push_str(&format!("accept_{index}(value True): return value\n"));
    }
    let path = package.write("main.gin", &source);
    let paths = [path.clone()];
    let expected = clean(&paths);
    assert!(expected.typecheck_outputs[0].len() >= 16);
    let spans: Vec<_> = expected.typecheck_outputs[0]
        .iter()
        .map(|diagnostic| (diagnostic.span.start(), diagnostic.span.end()))
        .collect();
    assert!(spans.windows(2).all(|pair| pair[0] <= pair[1]));
    for workers in [1, 4] {
        let cache = session_cache(workers);
        cache.add_file(path.clone()).unwrap();
        for _ in 0..8 {
            cache.invalidate_package_cache_for(&path);
            equivalent(&cache.get_or_compute_package(&paths).unwrap(), &expected);
            equivalent(&clean(&paths), &expected);
        }
    }
}

#[test]
fn dependency_configuration_changes_recompute_cached_imports() {
    let app = test_fixtures::TempPackage::new("editor_dependency_change");
    let first = test_fixtures::TempPackage::new("editor_dependency_first");
    let second = test_fixtures::TempPackage::new("editor_dependency_second");
    for (package, value) in [(&first, 1), (&second, 2)] {
        package.write_flask("dependency");
        package.write(
            "math.gin",
            &format!("{WORD}compute(value is in Word) is in Word: return {value}\n"),
        );
    }
    let path = app.write("main.gin", "use dependency compute\nanswer: compute(0)\n");
    let paths = [path.clone()];
    for cache in [PackageCache::for_test(), session_cache(4)] {
        for dependency in [&first, &second, &first] {
            app.write_flask_with_deps(
                "app",
                &format!(
                    "dependency: '../{}'\n",
                    dependency.path().file_name().unwrap().to_str().unwrap()
                ),
            );
            if !cache.contains(&path) {
                cache.add_file(path.clone()).unwrap();
            }
            equivalent(
                &cache.get_or_compute_package(&paths).unwrap(),
                &clean(&paths),
            );
        }
    }
}

#[test]
fn explicit_ad_hoc_packages_keep_library_target_semantics() {
    let package = test_fixtures::TempPackage::new("editor_adhoc_target");
    let path = package.write("main.gin", "bits := #index_bits(@())\n");
    let expected = clean(std::slice::from_ref(&path));
    package.write(
        "mod.flask",
        r#"#settings
module: "target"
version: 0.0.0
target: "wasm32-unknown-unknown"

#dependencies
"#,
    );
    for cache in [PackageCache::for_test(), session_cache(4)] {
        cache
            .add_file_to_package(path.clone(), cache.new_adhoc_package())
            .unwrap();
        let actual = cache
            .get_or_compute_package(std::slice::from_ref(&path))
            .unwrap();
        assert_eq!(
            actual.typed_asts[0].target_layout,
            expected.typed_asts[0].target_layout
        );
        assert_eq!(actual.typecheck_outputs, expected.typecheck_outputs);
    }
}

#[test]
fn target_configuration_changes_recompute_cached_packages() {
    for cache in [PackageCache::for_test(), session_cache(4)] {
        let package = test_fixtures::TempPackage::new("editor_target_change");
        let source = package.write("main.gin", "bits := #index_bits(@())\n");
        for target in [
            "wasm32-unknown-unknown",
            "x86_64-unknown-linux-gnu",
            "wasm32-unknown-unknown",
        ] {
            package.write(
                "mod.flask",
                &format!(
                    r#"#settings
module: "target"
version: 0.0.0
target: "{target}"

#dependencies
"#
                ),
            );
            if !cache.contains(&source) {
                cache.add_file(source.clone()).unwrap();
            }
            equivalent(
                &cache
                    .get_or_compute_package(std::slice::from_ref(&source))
                    .unwrap(),
                &clean(std::slice::from_ref(&source)),
            );
        }
    }
}

#[test]
fn imported_compile_time_edits_and_missing_imports_match_clean_analysis() {
    for workers in [1, 4] {
        let package = test_fixtures::TempPackage::new("editor_import_edits");
        package.write_flask("fixture");
        let implementation = package.write(
            "math/math.gin",
            &format!("{WORD}compute(value is in Word) is in Word: return 1\n"),
        );
        let source = package.write("main.gin", "use './math' compute\nanswer: compute(0)\n");
        let cache = session_cache(workers);
        cache.add_file(source.clone()).unwrap();
        equivalent(
            &cache
                .get_or_compute_package(std::slice::from_ref(&source))
                .unwrap(),
            &clean(std::slice::from_ref(&source)),
        );
        for body in ["2", "3", "1"] {
            std::fs::write(
                &implementation,
                format!(
                    "{WORD}compute(value is in Word) is in Word: {return_prefix}{body}\n",
                    return_prefix = if body.starts_with('\n') {
                        ""
                    } else {
                        "return "
                    }
                ),
            )
            .unwrap();
            cache.invalidate_package_cache_for(&implementation);
            equivalent(
                &cache
                    .get_or_compute_package(std::slice::from_ref(&source))
                    .unwrap(),
                &clean(std::slice::from_ref(&source)),
            );
        }
        let missing = "use './math' absent\nanswer: absent(0)\n";
        std::fs::write(&source, missing).unwrap();
        cache.set_contents(&source, missing.to_owned());
        equivalent(
            &cache
                .get_or_compute_package(std::slice::from_ref(&source))
                .unwrap(),
            &clean(std::slice::from_ref(&source)),
        );
        std::fs::write(
            &implementation,
            format!("{WORD}absent(value is in Word) is in Word: return 1\n"),
        )
        .unwrap();
        cache.invalidate_package_cache_for(&implementation);
        equivalent(
            &cache
                .get_or_compute_package(std::slice::from_ref(&source))
                .unwrap(),
            &clean(std::slice::from_ref(&source)),
        );
    }
}

#[test]
fn new_files_in_transitive_dependency_scopes_invalidate_missing_imports() {
    let app = test_fixtures::TempPackage::new("editor_scope_app");
    let bridge = test_fixtures::TempPackage::new("editor_scope_bridge");
    let leaf = test_fixtures::TempPackage::new("editor_scope_leaf");
    leaf.write_flask("leaf");
    bridge.write_flask_with_deps(
        "bridge",
        &format!(
            "leaf: '../{}'\n",
            leaf.path().file_name().unwrap().to_str().unwrap()
        ),
    );
    app.write_flask_with_deps(
        "app",
        &format!(
            "bridge: '../{}'\n",
            bridge.path().file_name().unwrap().to_str().unwrap()
        ),
    );
    leaf.write("math/types.gin", WORD);
    bridge.write(
        "bridge.gin",
        "use leaf.math Word, absent\nexpose(value is in Word) is in Word: return absent(value)\n",
    );
    let source = app.write("main.gin", "use bridge expose\nanswer: expose(0)\n");
    let cache = session_cache(4);
    cache.add_file(source.clone()).unwrap();
    let missing = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    assert!(!missing.import_diags.is_empty());
    let added = leaf.write(
        "math/functions.gin",
        "use './' Word\nabsent(value is in Word) is in Word: return 1\n",
    );
    cache.invalidate_package_cache_for(&added);
    let current = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    equivalent(&current, &clean(std::slice::from_ref(&source)));
    assert!(
        current.import_diags.is_empty(),
        "{:?}",
        current.import_diags
    );
}

#[test]
fn invalidation_rejects_in_flight_results_and_dependency_caches() {
    let package = test_fixtures::TempPackage::new("editor_stale_publication");
    package.write_flask("fixture");
    let source = package.write("main.gin", WORD);
    let cache = session_cache(4);
    cache.add_file(source.clone()).unwrap();
    let identity = cache.file_identity(&source).unwrap();
    let revision = cache.revision.load(Ordering::SeqCst);
    let result = cache
        .compute_package(
            revision,
            &identity,
            std::slice::from_ref(&source),
            CompileTarget::Library,
            &None,
            &None,
        )
        .unwrap();
    cache.set_contents(&source, format!("{WORD}\nmissing: unknown\n"));
    assert!(cache.publish_package(identity, revision, result).is_none());
    assert_eq!(cache.cache_counts(), (1, 0, 0, 0));
    let current = cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    assert!(
        current
            .typecheck_outputs
            .iter()
            .flatten()
            .any(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
    );
}

#[test]
fn edits_do_not_wait_for_checking_and_stale_checks_retry() {
    let package = test_fixtures::TempPackage::new("editor_concurrent_edit");
    package.write_flask("fixture");
    let source = package.write("main.gin", WORD);
    let cache = Arc::new(session_cache(4));
    cache.add_file(source.clone()).unwrap();
    cache
        .get_or_compute_package(std::slice::from_ref(&source))
        .unwrap();
    let identity = cache.file_identity(&source).unwrap();
    let session = cache.checking.as_ref().unwrap().packages.lock().unwrap()[&identity].clone();
    cache.invalidate_package_cache_for(&source);
    let held = session.lock().unwrap();
    std::thread::scope(|scope| {
        let reading = scope.spawn(|| {
            cache
                .get_or_compute_package(std::slice::from_ref(&source))
                .unwrap()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while Arc::strong_count(&session) < 3 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(Arc::strong_count(&session) >= 3);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let editing_cache = &cache;
        let editing_source = &source;
        scope.spawn(move || {
            editing_cache.set_contents(editing_source, format!("{WORD}\nmissing: unknown\n"));
            tx.send(()).unwrap();
        });
        let update = rx.recv_timeout(Duration::from_secs(2));
        drop(held);
        assert!(update.is_ok(), "editing blocked behind semantic checking");
        let result = reading.join().unwrap();
        assert!(
            result
                .typecheck_outputs
                .iter()
                .flatten()
                .any(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
        );
    });
}
