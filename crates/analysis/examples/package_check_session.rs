#[cfg(feature = "profile-allocations")]
#[global_allocator]
static ALLOCATOR: compiler_profile::TrackingAllocator<std::alloc::System> =
    compiler_profile::TrackingAllocator(std::alloc::System);

use std::path::PathBuf;
use std::time::Instant;

use analysis::PackageCache;
use resolve::GinPackageExt;
use typecheck::transform::{BodyReuse, PackageCheckSession};

fn replace_once(source: &str, before: &str, after: &str) -> String {
    assert_ne!(before, after);
    assert_eq!(
        source.matches(before).count(),
        1,
        "missing or ambiguous edit: {before}"
    );
    source.replacen(before, after, 1)
}

fn cache(mode: &str) -> PackageCache {
    if mode == "editor" {
        let (tx, _) = crossbeam_channel::unbounded();
        return PackageCache::with_check_session(
            tx,
            resolve::default_check_session().expect("editor session"),
        )
        .with_resolved_result_reuse();
    }
    let (tx, _rx) = crossbeam_channel::unbounded();
    let resolved = mode.starts_with("resolved-");
    let mode = mode.strip_prefix("resolved-").unwrap_or(mode);
    let cache = if mode == "legacy" {
        PackageCache::new(tx)
    } else {
        let (mode, parallel_references) = mode
            .strip_prefix("reference-")
            .map_or((mode, false), |mode| (mode, true));
        let (mode, effect_queries) = mode
            .strip_prefix("query-")
            .map_or((mode, false), |mode| (mode, true));
        let (mode, semantic_reuse) = mode
            .strip_prefix("semantic-")
            .map_or((mode, false), |mode| (mode, true));
        let (mode, function_reuse) = mode
            .strip_prefix("function-")
            .map_or((mode, false), |mode| (mode, true));
        let (mode, limited) = mode
            .strip_prefix("first")
            .map_or((mode, false), |workers| (workers, true));
        let (workers, reuse) = if limited {
            (mode, BodyReuse::ExactInputs)
        } else if let Some(workers) = mode.strip_prefix("reuse") {
            (workers, BodyReuse::ExactInputs)
        } else {
            (mode.strip_prefix("off").unwrap(), BodyReuse::Disabled)
        };
        let workers = workers.parse::<usize>().unwrap();
        let mut session = PackageCheckSession::new(workers.try_into().unwrap(), reuse).unwrap();
        if semantic_reuse {
            session = session.with_semantic_reuse();
        }
        if function_reuse {
            session = session.with_function_reuse();
        }
        if std::env::var_os("GIN_NO_FINISHING_REUSE").is_none() {
            session = session.with_finishing_reuse();
        }
        let session = if limited {
            session.with_retained_pass_limit(1.try_into().unwrap())
        } else {
            session
        };
        let session = if effect_queries {
            session.with_effect_queries()
        } else {
            session
        };
        let session = if parallel_references {
            session.with_parallel_references()
        } else {
            session
        };
        PackageCache::with_check_session(tx, session)
    };
    if resolved {
        cache.with_resolved_result_reuse()
    } else {
        cache
    }
}

fn register(cache: &PackageCache, paths: &[PathBuf]) {
    for path in paths {
        cache.add_file(path.clone()).unwrap();
    }
}

fn equivalent(
    left: &analysis::package_cache::PackageEntry,
    right: &analysis::package_cache::PackageEntry,
) {
    assert_eq!(left.public_interface, right.public_interface);
    assert_eq!(left.file_paths, right.file_paths);
    assert_eq!(left.typed_asts, right.typed_asts);
    assert_eq!(left.typecheck_outputs, right.typecheck_outputs);
    assert_eq!(left.prepare_diags, right.prepare_diags);
    assert_eq!(left.import_diags, right.import_diags);
    for (left, right) in left.typed_asts.iter().zip(&right.typed_asts) {
        assert_eq!(left.warnings, right.warnings);
        assert_eq!(left.parse_warnings, right.parse_warnings);
        assert_eq!(left.import_mod_paths, right.import_mod_paths);
        assert_eq!(left.imported_trait_names, right.imported_trait_names);
        assert_eq!(left.eval_ast, right.eval_ast);
        assert_eq!(left.type_registry, right.type_registry);
        assert_eq!(left.target_layout, right.target_layout);
        assert_eq!(left.span_table, right.span_table);
        assert_eq!(left.places, right.places);
        assert_eq!(left.place_versions, right.place_versions);
        assert_eq!(
            left.place_version_components,
            right.place_version_components
        );
        assert_eq!(left.constant_objects, right.constant_objects);
    }
}

fn main() {
    assert!(!cfg!(debug_assertions), "run this harness with --release");
    let args: Vec<_> = std::env::args().collect();
    let case = args.get(1).map(String::as_str).unwrap_or("synthetic");
    assert!(["synthetic", "calls", "core", "core-edit"].contains(&case));
    let files = args
        .get(2)
        .map(|s| s.parse::<usize>().expect("files must be an integer"))
        .unwrap_or(32);
    let functions = args
        .get(3)
        .map(|s| s.parse::<usize>().expect("functions must be an integer"))
        .unwrap_or(32);
    let samples = args
        .get(4)
        .map(|s| s.parse::<usize>().expect("samples must be an integer"))
        .unwrap_or(5);
    let mode = args.get(5).map(String::as_str).unwrap_or("editor");
    assert!((5..=100).contains(&samples));
    assert!((1..=1024).contains(&files));
    assert!((1..=1024).contains(&functions));
    let package = test_fixtures::TempPackage::new("editor_benchmark");
    let paths = if case == "core" || case == "core-edit" {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
        let mut paths = root.collect_gin_files();
        paths.sort();
        if case == "core-edit" {
            std::fs::copy(root.join("mod.flask"), package.join("mod.flask")).unwrap();
            paths
                .into_iter()
                .map(|path| {
                    let relative = path.strip_prefix(&root).unwrap().to_str().unwrap();
                    package.write(relative, &std::fs::read_to_string(&path).unwrap())
                })
                .collect()
        } else {
            paths
        }
    } else {
        package.write_flask("benchmark");
        package.write("math/math.gin", "marker: 1\n");
        let mut paths = vec![package.write("000-types.gin", test_fixtures::preludes::WORD_PRELUDE)];
        let body = if case == "calls" {
            paths.push(package.write("050-helper.gin", "stable_helper(x Word) Word: return x\n"));
            (0..16).fold("x".to_owned(), |body, _| format!("stable_helper({body})"))
        } else {
            "x".to_owned()
        };
        for file in 0..files {
            let source = (0..functions)
                .map(|function| {
                    format!(
                        "work_{file}_{function}(x Word) Word: {return_prefix}{body}\n",
                        return_prefix = if body.starts_with('\n') {
                            ""
                        } else {
                            "return "
                        }
                    )
                })
                .collect::<String>();
            paths.push(package.write(&format!("100-{file:03}.gin"), &source));
        }
        paths.push(package.write(
            "200-private.gin",
            "private\nprivate_helper(x ref Word) Word: return 1\n",
        ));
        paths.push(package.write(
            "300-compute.gin",
            "compute(x is in Word) is in Word: return 1\n",
        ));
        paths.push(package.write(
            "400-consumer.gin",
            "use './math' marker\nconsumer: compute(0)\nmarker_consumer: marker\n",
        ));
        paths
    };
    let unrelated = package.join("math/unrelated.gin");
    let helper_index = paths
        .iter()
        .position(|path| path.ends_with("050-helper.gin"));
    let private_index = paths
        .iter()
        .position(|path| path.ends_with("200-private.gin"));
    let compute_index = paths
        .iter()
        .position(|path| path.ends_with("300-compute.gin"));
    let core_body_index = paths.iter().position(|path| path.ends_with("io/io.gin"));
    let original: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    let scenarios: &[&str] = if case == "core" {
        &["cold", "cache_hit", "recheck"]
    } else if case == "core-edit" {
        &["cold", "cache_hit", "recheck", "core_body"]
    } else if case == "calls" {
        &[
            "cold",
            "cache_hit",
            "recheck",
            "same_file_body",
            "callee_body",
            "public_signature",
            "compile_time_body",
        ]
    } else {
        &[
            "cold",
            "cache_hit",
            "recheck",
            "unrelated_scope",
            "private_body",
            "same_file_body",
            "public_signature",
            "compile_time_body",
        ]
    };
    let scenario_filter = args.get(6).map(String::as_str);
    if let Some(filter) = scenario_filter {
        assert!(
            filter
                .split(',')
                .all(|scenario| scenarios.contains(&scenario))
        );
    }
    for scenario in scenarios.iter().filter(|scenario| {
        scenario_filter.is_none_or(|filter| filter.split(',').any(|name| name == **scenario))
    }) {
        let mut observations = Vec::new();
        for sample in 0..=samples {
            if case != "core" {
                let _ = std::fs::remove_file(&unrelated);
                for (path, source) in paths.iter().zip(&original) {
                    std::fs::write(path, source).unwrap();
                }
            }
            let mut active = if *scenario == "cold" {
                None
            } else {
                let active = cache(mode);
                register(&active, &paths);
                active.get_or_compute_package(&paths).unwrap();
                Some(active)
            };
            let edit = match *scenario {
                "callee_body" => {
                    let index = helper_index.unwrap();
                    Some((
                        index,
                        replace_once(&original[index], ": return x", ": return 1"),
                    ))
                }
                "private_body" => {
                    let index = private_index.unwrap();
                    Some((
                        index,
                        replace_once(&original[index], ": return 1", ": return 2"),
                    ))
                }
                "same_file_body" => {
                    let index = paths
                        .iter()
                        .position(|path| path.ends_with("100-000.gin"))
                        .unwrap();
                    let end = original[index].find('\n').unwrap();
                    let source =
                        format!("work_0_0(x Word) Word: return 1{}", &original[index][end..]);
                    Some((index, source))
                }
                "public_signature" => Some((0, replace_once(&original[0], "255", "511"))),
                "compile_time_body" => {
                    let index = compute_index.unwrap();
                    Some((
                        index,
                        replace_once(&original[index], ": return 1", ": return 2"),
                    ))
                }
                "core_body" => {
                    let index = core_body_index.unwrap();
                    let before = "byte_value(value SignedBigInt) Byte: return when value is in Byte then value as Byte else 0";
                    let after = "byte_value(value SignedBigInt) Byte: return when value is in Byte then value as Byte else 1";
                    Some((index, replace_once(&original[index], before, after)))
                }
                _ => None,
            };
            if let Some((index, source)) = &edit {
                assert_ne!(source, &original[*index]);
                std::fs::write(&paths[*index], source).unwrap();
            }
            if *scenario == "unrelated_scope" {
                std::fs::write(&unrelated, "Unused is Left or Right\n").unwrap();
            }
            eprintln!("MEASURE_BEGIN case={case} scenario={scenario} sample={sample} mode={mode}");
            let measure = || {
                let start = Instant::now();
                if *scenario == "cold" {
                    let next = cache(mode);
                    register(&next, &paths);
                    active = Some(next);
                }
                let active = active.unwrap();
                if *scenario == "recheck" {
                    active.invalidate_package_cache_for(&paths[0]);
                }
                if *scenario == "unrelated_scope" {
                    active.invalidate_package_cache_for(&unrelated);
                }
                let edit_start = Instant::now();
                if let Some((index, source)) = edit {
                    active.set_contents(&paths[index], source);
                }
                let edit_ms = edit_start.elapsed().as_secs_f64() * 1000.0;
                let query_start = Instant::now();
                let result = active.get_or_compute_package(&paths).unwrap();
                let query_ms = query_start.elapsed().as_secs_f64() * 1000.0;
                let milliseconds = start.elapsed().as_secs_f64() * 1000.0;
                (active, result, milliseconds, edit_ms, query_ms)
            };
            let ((active, result, milliseconds, edit_ms, query_ms), profile) =
                if std::env::var_os("GIN_PROFILE").is_some() {
                    let (result, profile) = compiler_profile::capture(measure);
                    (result, Some(profile))
                } else {
                    (measure(), None)
                };
            eprintln!("MEASURE_END");
            let expected = cache("legacy");
            register(&expected, &paths);
            let expected_result = expected.get_or_compute_package(&paths).unwrap();
            equivalent(&result, &expected_result);
            let flaws = result
                .typecheck_outputs
                .iter()
                .flatten()
                .chain(result.prepare_diags.iter().flatten())
                .chain(result.import_diags.values().flatten())
                .filter(|d| d.category == diagnostic::Category::Flaw)
                .count();
            let expected_flaws = std::env::var("GIN_BENCH_EXPECT_FLAWS")
                .map(|value| {
                    value
                        .parse::<usize>()
                        .expect("expected flaws must be an integer")
                })
                .unwrap_or(0);
            assert_eq!(
                flaws, expected_flaws,
                "{case} / {scenario}: {:?}",
                result.typecheck_outputs
            );
            let resolved_reuse = active.resolved_result_reused(&paths[0]);
            let package_result_reused = *scenario == "cache_hit" || resolved_reuse == Some(true);
            let stats = if package_result_reused {
                None
            } else {
                active.check_stats(&paths[0])
            };
            if sample > 0 {
                observations.push(serde_json::json!({
                    "profile": profile,
                    "milliseconds": milliseconds,
                    "edit_ms": edit_ms,
                    "query_ms": query_ms,
                    "resolved_result_reused": resolved_reuse,
                    "package_result_reused": package_result_reused,
                    "resolved_files": result.file_paths.len(),
                    "flaws": flaws,
                    "lowered_files": stats.as_ref().map(|s| s.lowered_files),
                    "reused_files": stats.as_ref().map(|s| s.reused_files),
                    "lowered_expressions": stats.as_ref().map(|s| s.lowered_expressions),
                    "declared_files": stats.as_ref().map(|s| s.declared_files),
                    "type_declaration_checks": stats.as_ref().map(|s| s.type_declaration_checks),
                    "type_declaration_reused": stats.as_ref().map(|s| s.type_declaration_reused),
                    "signature_checks": stats.as_ref().map(|s| s.signature_checks),
                    "signature_reused": stats.as_ref().map(|s| s.signature_reused),
                    "reference_checks_reused": stats.as_ref().map(|s| s.reference_checks_reused),
                    "retained_function_answers": stats.as_ref().map(|s| s.retained_function_answers),
                    "retained_function_expressions": stats.as_ref().map(|s| s.retained_function_expressions),
                    "retained_semantic_dependencies": stats.as_ref().map(|s| s.retained_semantic_dependencies),
                    "effect_checks_reused": stats.as_ref().map(|s| s.effect_checks_reused),
                    "staging_checks": stats.as_ref().map(|s| s.staging_checks),
                    "staging_reused": stats.as_ref().map(|s| s.staging_reused),
                    "specialization_checks": stats.as_ref().map(|s| s.specialization_checks),
                    "specialization_reused": stats.as_ref().map(|s| s.specialization_reused),
                    "reference_checks": stats.as_ref().map(|s| s.reference_checks),
                    "reference_rounds": stats.as_ref().map(|s| s.reference_rounds),
                    "effect_checks": stats.as_ref().map(|s| s.effect_checks),
                    "effect_checks_skipped": stats.as_ref().map(|s| s.effect_checks_skipped),
                    "passes": stats.as_ref().map(|s| s.passes),
                    "retained_passes": stats.as_ref().map(|s| s.retained_passes),
                    "declarations_ms": stats.as_ref().map(|s| s.declarations.as_secs_f64() * 1000.0),
                    "type_declarations_ms": stats.as_ref().map(|s| s.type_declarations.as_secs_f64() * 1000.0),
                    "signatures_ms": stats.as_ref().map(|s| s.signatures.as_secs_f64() * 1000.0),
                    "specialization_ms": stats.as_ref().map(|s| s.specialization.as_secs_f64() * 1000.0),
                    "constant_queries": stats.as_ref().map(|s| s.constant_queries),
                    "constant_environments": stats.as_ref().map(|s| s.constant_environments),
                    "lowering_ms": stats.as_ref().map(|s| s.lowering.as_secs_f64() * 1000.0),
                    "inference_ms": stats.as_ref().map(|s| s.inference.as_secs_f64() * 1000.0),
                    "finishing_ms": stats.as_ref().map(|s| s.finishing.as_secs_f64() * 1000.0),
                    "finishing_reused": stats.as_ref().map(|s| s.finishing_reused),
                }));
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "schema_version": 1, "case":case,"files":paths.len(),"functions":functions,
                "scenario":scenario,"mode":mode,"samples":observations,
                "equivalent_to_clean": true, "excluded_warmups": 1,
                "scope": "editor package diagnostics; cold includes session creation, registration, reads, parsing, imports and checking; edits include set_contents and query; excludes fixture writes, parity validation and result destruction",
            })
        );
    }
}
