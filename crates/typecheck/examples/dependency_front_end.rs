#[cfg(feature = "profile-allocations")]
#[global_allocator]
static ALLOCATOR: compiler_profile::TrackingAllocator<std::alloc::System> =
    compiler_profile::TrackingAllocator(std::alloc::System);

use std::collections::HashMap;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ast::ty::PackageInstanceKey;
use flask::CompileTarget;
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, PackageFrontEndOptions, ParsedFile, ParsedModuleCache};
use typecheck::transform::{BodyReuse, PackageCheckSession, PackageTransformOptions};

fn sources(files: usize, functions: usize, constants: bool) -> Vec<(PathBuf, String)> {
    let mut sources = vec![(
        PathBuf::from("000-types.gin"),
        String::from(test_fixtures::preludes::WORD_PRELUDE),
    )];
    for file in 0..files {
        let mut source = String::new();
        for function in 0..functions {
            source.push_str(&format!("work_{file}_{function}(x Word) Word: return x\n"));
        }
        source.push_str(&format!(
            "private\nprivate_{file}(x ref Word) Word: return 1\n"
        ));
        sources.push((PathBuf::from(format!("100-{file:03}.gin")), source));
    }
    sources.push((
        PathBuf::from("200-compute.gin"),
        String::from("compute(x is in Word) is in Word: return 1\n"),
    ));
    sources.push((
        PathBuf::from("300-consumer.gin"),
        String::from("consumer: compute(0)\n"),
    ));
    if constants {
        let mut replacements = 0;
        for (_, source) in &mut sources {
            replacements += source.matches(" Word: return x\n").count();
            *source = source.replace(" Word: return x\n", " Word: return seed\n");
        }
        assert_eq!(replacements, files * functions);
        sources.push((
            PathBuf::from("200-seed.gin"),
            String::from("private\nseed Word: compute(0)\n"),
        ));
    }
    sources
}

fn parse(sources: &[(PathBuf, String)]) -> Vec<ParsedFile> {
    sources
        .iter()
        .map(|(path, source)| {
            let output = source.parse_source_full();
            assert!(
                output
                    .symptoms
                    .iter()
                    .all(|diagnostic| diagnostic.category != diagnostic::Category::Flaw),
                "{:?}",
                output.symptoms
            );
            ParsedFile {
                path: path.clone(),
                source: source.clone(),
                output,
            }
        })
        .collect()
}

fn analyze(
    files: Vec<ParsedFile>,
    imports: bool,
    options: PackageTransformOptions,
    session: Option<&mut PackageCheckSession>,
) -> resolve::PackageFrontEndOutput {
    let dependencies = files
        .first()
        .map(|file| file.path.flask_path_dependencies())
        .unwrap_or_default();
    let options = PackageFrontEndOptions {
        gin_home: None,
        compile_target: CompileTarget::Library,
        transform: options,
        resolve_imports: imports,
        package_fallback: PackageInstanceKey::workspace("benchmark", "0"),
        transform_paths: None,
    };
    match session {
        Some(session) => resolve::run_package_front_end_with_session(
            files,
            &dependencies,
            ParsedModuleCache::default(),
            options,
            session,
        ),
        None => resolve::run_package_front_end(
            files,
            &dependencies,
            ParsedModuleCache::default(),
            options,
        ),
    }
}

fn equivalent(left: &resolve::PackageFrontEndOutput, right: &resolve::PackageFrontEndOutput) {
    assert_eq!(left.public_interface, right.public_interface);
    assert_eq!(left.files.len(), right.files.len());
    for (left, right) in left.files.iter().zip(&right.files) {
        assert_eq!(left.parsed.path, right.parsed.path);
        assert_eq!(left.import_diagnostics, right.import_diagnostics);
        assert_eq!(left.prepare_diagnostics, right.prepare_diagnostics);
        let left = left.typed.as_ref().unwrap();
        let right = right.typed.as_ref().unwrap();
        if left != right {
            macro_rules! differences {
                ($($field:ident),+) => { $(if left.$field != right.$field { eprintln!("different {} in {}", stringify!($field), left.file_id.0); })+ };
            }
            differences!(
                tags,
                defs,
                overload_groups,
                private_tags,
                private_defs,
                exprs,
                resolved_member_calls,
                root_exprs,
                tag_types,
                fn_return_types,
                variant_map,
                variant_annotations,
                semantic_origin,
                module_doc
            );
            panic!("typed results differ in file {}", left.file_id.0);
        }
        assert_eq!(left.type_registry, right.type_registry);
        assert_eq!(left.span_table, right.span_table);
        assert_eq!(left.target_layout, right.target_layout);
        assert_eq!(left.import_mod_paths, right.import_mod_paths);
        assert_eq!(left.imported_trait_names, right.imported_trait_names);
        assert_eq!(left.eval_ast, right.eval_ast);
        assert_eq!(left.warnings, right.warnings);
        assert_eq!(left.declaration_flaws, right.declaration_flaws);
        assert_eq!(left.parse_warnings, right.parse_warnings);
        assert_eq!(left.constant_objects, right.constant_objects);
        assert_eq!(left.places, right.places);
        assert_eq!(left.place_versions, right.place_versions);
        assert_eq!(
            left.place_version_components,
            right.place_version_components
        );
        let diagnostics = |file: &typecheck::TypedFileAst| {
            let mut diagnostics: Vec<_> = file
                .all_flaws()
                .into_iter()
                .map(|(span, flaw)| format!("{span:?}:{flaw:?}"))
                .collect();
            diagnostics.sort();
            diagnostics
        };
        assert_eq!(diagnostics(left), diagnostics(right));
    }
}

fn edit(base: &[(PathBuf, String)], scenario: &str) -> Vec<(PathBuf, String)> {
    let mut after = base.to_vec();
    let find = |needle: &str| {
        after
            .iter()
            .position(|(_, source)| source.contains(needle))
            .unwrap()
    };
    let work_body = if base
        .iter()
        .any(|(_, source)| source.contains("work_0_0(x Word) Word: return seed"))
    {
        "work_0_0(x Word) Word: return seed"
    } else {
        "work_0_0(x Word) Word: return x"
    };
    let change = match scenario {
        "private_body" => Some((
            "private_0(x ref Word) Word: return 1",
            "private_0(x ref Word) Word: return 2",
        )),
        "public_signature" => Some((work_body, "work_0_0(x ref Word) Word: return 2")),
        "compile_time_body" => Some((
            "compute(x is in Word) is in Word: return 1",
            "compute(x is in Word) is in Word: return 2",
        )),
        _ => None,
    };
    if let Some((old, new)) = change {
        let index = find(old);
        assert_eq!(after[index].1.matches(old).count(), 1);
        after[index].1 = after[index].1.replace(old, new);
        assert_ne!(after, base);
    }
    after
}

fn main() {
    assert!(
        !cfg!(debug_assertions),
        "run this measurement harness with --release"
    );
    let args: Vec<_> = std::env::args().collect();
    let core_path = args.get(1).and_then(|arg| {
        if arg == "core" {
            Some(Path::new("modules/core"))
        } else {
            let path = Path::new(arg);
            path.exists().then_some(path)
        }
    });
    let core = core_path.is_some();
    let files = if core {
        0
    } else {
        args.get(1).map_or(8, |arg| arg.parse().unwrap())
    };
    let functions = args.get(2).map_or(32, |arg| arg.parse().unwrap());
    let samples: usize = args.get(3).map_or(7, |arg| arg.parse().unwrap());
    assert!(samples >= 5, "at least five measured samples are required");
    let requested = args.get(4).map(String::as_str).unwrap_or("production");
    let options = if args.get(5).is_some_and(|arg| arg == "ide") {
        PackageTransformOptions::IDE
    } else {
        PackageTransformOptions::FULL
    };
    let mut base: Vec<_> = if let Some(path) = core_path {
        path.collect_gin_files()
            .into_iter()
            .map(|path| {
                let source = std::fs::read_to_string(&path).unwrap();
                (path, source)
            })
            .collect()
    } else {
        sources(
            files,
            functions,
            args.get(6).is_some_and(|arg| arg == "constants"),
        )
    };
    base.sort_by(|(left, _), (right, _)| left.cmp(right));
    let modes = [
        ("production", 1, BodyReuse::ExactInputs),
        ("legacy", 1, BodyReuse::Disabled),
        ("serial", 1, BodyReuse::Disabled),
        ("parallel2", 2, BodyReuse::Disabled),
        ("parallel4", 4, BodyReuse::Disabled),
        ("parallel8", 8, BodyReuse::Disabled),
        ("reuse4", 4, BodyReuse::ExactInputs),
    ];
    let modes: Vec<_> = modes
        .into_iter()
        .filter(|(mode, _, _)| requested == "all" || requested == *mode)
        .collect();
    assert!(!modes.is_empty(), "unknown mode {requested}");
    let scenarios = if core {
        vec!["cold", "unchanged"]
    } else {
        vec![
            "cold",
            "unchanged",
            "private_body",
            "public_signature",
            "compile_time_body",
        ]
    };
    let mut records = HashMap::<(&str, &str), Vec<serde_json::Value>>::new();
    let scenario_filter = args.get(7).map(String::as_str);
    if let Some(filter) = scenario_filter {
        assert!(
            filter
                .split(',')
                .all(|scenario| scenarios.contains(&scenario))
        );
    }
    for scenario in scenarios.into_iter().filter(|scenario| {
        scenario_filter.is_none_or(|filter| filter.split(',').any(|name| name == *scenario))
    }) {
        let after = edit(&base, scenario);
        let input = parse(&after);
        let before = parse(&base);
        let expected = analyze(input.clone(), core, options.clone(), None);
        let flaws: usize = expected
            .files
            .iter()
            .map(|file| {
                file.typed
                    .as_ref()
                    .map_or(0, |typed| typed.all_flaws().len())
                    + file
                        .parsed
                        .output
                        .symptoms
                        .iter()
                        .chain(&file.import_diagnostics)
                        .chain(&file.prepare_diagnostics)
                        .filter(|diagnostic| diagnostic.category == diagnostic::Category::Flaw)
                        .count()
            })
            .sum();
        let expected_flaws = std::env::var("GIN_BENCH_EXPECT_FLAWS")
            .map(|value| {
                value
                    .parse::<usize>()
                    .expect("expected flaws must be an integer")
            })
            .unwrap_or(0);
        assert_eq!(flaws, expected_flaws, "invalid {scenario} workload");
        for sample in 0..=samples {
            for mode_index in 0..modes.len() {
                let (mode, workers, reuse) = modes[(mode_index + sample) % modes.len()];
                let setup = Instant::now();
                let mut session = match mode {
                    "production" => {
                        Some(resolve::default_check_session().expect("production session"))
                    }
                    "legacy" => None,
                    _ => {
                        Some(PackageCheckSession::new(workers.try_into().unwrap(), reuse).unwrap())
                    }
                };
                let setup_ms = setup.elapsed().as_secs_f64() * 1000.0;
                if scenario != "cold" {
                    black_box(analyze(
                        before.clone(),
                        core,
                        options.clone(),
                        session.as_mut(),
                    ));
                }
                let measure = || {
                    let start = Instant::now();
                    let result = analyze(input.clone(), core, options.clone(), session.as_mut());
                    (result, start.elapsed().as_secs_f64() * 1000.0)
                };
                let ((result, elapsed_ms), profile) = if std::env::var_os("GIN_PROFILE").is_some() {
                    let (result, profile) = compiler_profile::capture(measure);
                    (result, Some(profile))
                } else {
                    let mut measure = measure;
                    (measure(), None)
                };
                equivalent(&expected, &result);
                black_box(&result);
                if sample != 0 {
                    let stats = session.as_ref().map(|session| session.stats());
                    records.entry((scenario, mode)).or_default().push(serde_json::json!({
                        "milliseconds": elapsed_ms,
                        "profile": profile,
                        "cold_with_pool_setup_ms": elapsed_ms + if scenario == "cold" { setup_ms } else { 0.0 },
                        "pool_setup_ms": setup_ms,
                        "passes": stats.map(|s| s.passes),
                        "declared_files": stats.map(|s| s.declared_files), "lowered_files": stats.map(|s| s.lowered_files),
                        "reused_files": stats.map(|s| s.reused_files), "finished_files": stats.map(|s| s.finished_files),
                        "lowered_expressions": stats.map(|s| s.lowered_expressions),
                        "constant_queries": stats.map(|s| s.constant_queries),
                        "constant_environments": stats.map(|s| s.constant_environments),
                        "declarations_ms": stats.map(|s| s.declarations.as_secs_f64() * 1000.0),
                        "type_declarations_ms": stats.map(|s| s.type_declarations.as_secs_f64() * 1000.0),
                        "signatures_ms": stats.map(|s| s.signatures.as_secs_f64() * 1000.0),
                        "lowering_ms": stats.map(|s| s.lowering.as_secs_f64() * 1000.0),
                        "inference_ms": stats.map(|s| s.inference.as_secs_f64() * 1000.0),
                        "finishing_ms": stats.map(|s| s.finishing.as_secs_f64() * 1000.0),
                        "specialization_ms": stats.map(|s| s.specialization.as_secs_f64() * 1000.0),
                    }));
                }
            }
        }
        let source_bytes: Vec<_> = after
            .iter()
            .flat_map(|(path, source)| {
                let relative = core_path
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path);
                format!("{}\0{}\0", relative.display(), source).into_bytes()
            })
            .collect();
        for (mode, workers, reuse) in &modes {
            println!(
                "{}",
                serde_json::json!({
                    "scenario": scenario, "mode": mode, "workers": workers, "reuse": format!("{reuse:?}"),
                    "workload": core_path.map_or("identity-functions".to_owned(), |path| path.display().to_string()),
                    "input_fingerprint": format!("{:?}", interface::Fingerprint::from_bytes(&source_bytes)),
                    "input_files": input.len(), "resolved_files": expected.files.len(), "functions_per_file": functions,
                    "ide": options.ide_package, "target": "library", "profile": "release", "flaws": flaws,
                    "equivalent_to_clean": true, "samples": records.remove(&(scenario, mode)).unwrap(),
                    "schema_version": 1, "excluded_warmups": 1,
                    "scope": "compiler front end from parsed input; includes input clone, preparation, imports and checking; excludes source reads, parsing, session setup, parity validation and result destruction",
                })
            );
        }
    }
}
