use super::*;
use crate::transform::{
    PackageTransformArtifacts, transform_package,
    transform_package_with_shared_context_and_package,
    transform_package_with_shared_context_and_session,
};
use diagnostic::code::TypeCode;
use parser::query::SourceParseExt;

const WORD: &str = test_fixtures::preludes::WORD_PRELUDE;

#[test]
fn deferred_finishing_matches_eager_resolved_core() {
    use resolve::GinPackageExt;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../modules/core");
    let parsed = root
        .collect_gin_files()
        .into_iter()
        .map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            resolve::ParsedFile {
                path,
                output: source.parse_source_full(),
                source,
            }
        })
        .collect();
    let mut options = resolve::PackageFrontEndOptions {
        gin_home: None,
        compile_target: flask::CompileTarget::Library,
        transform: Default::default(),
        resolve_imports: true,
        package_fallback: package(),
        transform_paths: None,
    };
    options.transform.ide_package = true;
    let output = resolve::run_package_front_end(
        parsed,
        &HashMap::new(),
        resolve::ParsedModuleCache::default(),
        options,
    );
    let mut sources: Vec<_> = output
        .files
        .into_iter()
        .map(|file| file.parsed.output.ast)
        .collect();
    let origin = sources
        .iter()
        .find_map(|file| file.semantic_origin.as_ref())
        .unwrap()
        .package
        .clone();
    let mut merged = FileAst::default();
    for source in &sources {
        merged.merge_from(source.clone());
    }
    let mut constants = crate::prepare_target::const_env_from_prepared_ast(
        &merged,
        Some(&flask::CompileTarget::Library),
    );
    constants.retain(|name, value| name.as_str().contains('.') && value.is_some());
    for source in &mut sources {
        crate::prepare_target::materialize_constant_predicates(source, &constants);
    }
    crate::prepare_target::materialize_constant_predicates(&mut merged, &constants);
    let context = TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(merged))
        .with_package_instance(origin);
    let sources: Vec<_> = sources
        .into_iter()
        .enumerate()
        .map(|(index, source)| (source, FileId(index as u32)))
        .collect();
    let expected = super::super::transform_package_eager(
        &sources,
        &context,
        PackageTransformOptions::IDE,
        None,
    );
    let mut checker = PackageCheckSession::new(std::num::NonZeroUsize::MIN, BodyReuse::ExactInputs)
        .unwrap()
        .with_function_reuse()
        .with_finishing_reuse()
        .with_retained_pass_limit(std::num::NonZeroUsize::MIN);
    let actual = checker.transform(&sources, &context, PackageTransformOptions::IDE);
    assert_eq!(checker.stats.passes, 7);
    assert_eq!(checker.stats.finished_files, sources.len());
    for (expected, actual) in expected.iter().zip(&actual) {
        assert!(
            exact_file_eq(expected, actual),
            "different core file {}",
            expected.file_id.0
        );
    }
}

#[test]
fn deferred_finishing_matches_eager_generic_chains_across_edits() {
    let before = inputs(&[
        WORD,
        "leaf(value): return value\n",
        "middle(value): return leaf(value)\n",
        "outer(value): return middle(value)\n",
        "caller(x Word) Word: return outer(x)\n",
        "independent(x Word) Word: return x\n",
    ]);
    for workers in [1, 2, 4] {
        for reuse in [BodyReuse::Disabled, BodyReuse::ExactInputs] {
            for semantic in [false, true] {
                let mut checker = PackageCheckSession::new(workers.try_into().unwrap(), reuse)
                    .unwrap()
                    .with_function_reuse()
                    .with_finishing_reuse()
                    .with_retained_pass_limit(std::num::NonZeroUsize::MIN);
                if semantic {
                    checker = checker.with_semantic_reuse();
                }
                for leaf in ["value", "missing(value)", "1", "value"] {
                    let mut source = before.clone();
                    source[1] = inputs(&[&format!("leaf(value): return {leaf}\n")]).remove(0);
                    assert_eager_finishing_parity(&mut checker, &source);
                    assert!(checker.stats.passes >= 4, "{:?}", checker.stats);
                    assert_eq!(checker.stats.finished_files, source.len());
                }
            }
        }
    }
}

#[test]
fn deferred_finishing_preserves_generic_diagnostics_proofs_and_rematerialization() {
    let fixtures = [
        format!("{WORD}generic(value): return value\ncaller(x Word) Word: return generic(x, x)\n"),
        format!("{WORD}identity(element, value element) element: return value\ncaller(x Word) Word: return identity(Word, x)\n"),
        format!("{WORD}identity(element, value element) element: return value\nOther is in 0 to 255\ncaller(x Other) Word: return identity(Word, x)\n"),
        format!("{WORD}Holder(element) has\n    finish(self):\n        return\n    value element\n    new(element, value element) Self: return Self(value: value)\n    copy_value(self ref) element: return self.value\nrun: Holder.copy_value(Holder.new(Word, 7 as Word))\n"),
        "Index is in 0 to 100\ngeneric(value): return value\ncaller(x Index) Index: return generic(x + 1)\n".to_owned(),
    ];
    for source in fixtures {
        let mut checker = session(1, BodyReuse::ExactInputs);
        for options in [
            PackageTransformOptions::IDE,
            PackageTransformOptions::FULL,
            PackageTransformOptions::IDE,
        ] {
            let source = inputs(&[&source]);
            let mut merged = FileAst::default();
            for file in &source {
                merged.merge_from(file.clone());
            }
            let context = TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(merged))
                .with_package_instance(package());
            let files: Vec<_> = source
                .into_iter()
                .enumerate()
                .map(|(index, file)| (file, FileId(index as u32)))
                .collect();
            let expected =
                super::super::transform_package_eager(&files, &context, options.clone(), None);
            for _ in 0..2 {
                let actual = checker.transform(&files, &context, options.clone());
                for (expected, actual) in expected.iter().zip(&actual) {
                    assert!(exact_file_eq(expected, actual), "{options:?}");
                }
            }
        }
    }
}

fn assert_eager_finishing_parity(checker: &mut PackageCheckSession, source: &[FileAst]) {
    let mut merged = FileAst::default();
    for file in source {
        merged.merge_from(file.clone());
    }
    let context = TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(merged))
        .with_package_instance(package());
    let files: Vec<_> = source
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, file)| (file, FileId(index as u32)))
        .collect();
    let expected =
        super::super::transform_package_eager(&files, &context, PackageTransformOptions::IDE, None);
    let actual = checker.transform(&files, &context, PackageTransformOptions::IDE);
    assert_eq!(expected.len(), actual.len());
    for (expected, actual) in expected.iter().zip(&actual) {
        assert!(
            exact_file_eq(expected, actual),
            "different file {}",
            expected.file_id.0
        );
    }
}

#[test]
fn body_size_changes_reuse_unchanged_files_with_fresh_check_parity() {
    let body = (0..16).fold("x".to_owned(), |body, _| format!("stable_helper({body})"));
    let source = format!("edited(x Word) Word: return {body}\nsibling(x Word) Word: return x\n");
    let before = inputs(&[
        WORD,
        "stable_helper(x Word) Word: return x\n",
        &source,
        "independent(x Word) Word: return stable_helper(x)\n",
    ]);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        for workers in [1, 2, 4] {
            let mut checker =
                PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                    .unwrap()
                    .with_function_reuse()
                    .with_finishing_reuse()
                    .with_retained_pass_limit(std::num::NonZeroUsize::MIN);
            check(&mut checker, &before, options.clone());
            for replacement in [
                "1",
                "stable_helper(stable_helper(x))",
                "missing(x)",
                "x",
                &body,
            ] {
                let mut after = before.clone();
                after[2] =
                    inputs(&[&source
                        .replace(&format!("return {body}"), &format!("return {replacement}"))])
                    .remove(0);
                let actual = check(&mut checker, &after, options.clone());
                equivalent(&clean(&after, options.clone()), &actual);
                assert_eq!(checker.stats().lowered_files, 1, "{:?}", checker.stats());
                assert_eq!(checker.stats().reused_files, 3);
                assert!(
                    checker.stats().finishing_reused >= 2,
                    "{:?}",
                    checker.stats()
                );
                assert_eq!(
                    actual.typed_asts[2].all_flaws().is_empty(),
                    replacement != "missing(x)"
                );
            }
        }
    }
}

#[test]
fn function_reuse_preserves_siblings_after_body_growth_and_arena_relocation() {
    let mut source = String::from(WORD);
    for index in 0..64 {
        source.push_str(&format!("leaf_{index}(x ref Word) Word: return 1\n"));
    }
    source.push_str("caller(x ref Word) Word: return leaf_0(x)\n");
    let before = inputs(&[&source]);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        for workers in [1, 2, 4] {
            let mut checker =
                PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                    .unwrap()
                    .with_function_reuse()
                    .with_finishing_reuse()
                    .with_retained_pass_limit(std::num::NonZeroUsize::MIN);
            check(&mut checker, &before, options.clone());
            for replacement in ["2", "\n    unused: 0\n    return 2", "250", "1"] {
                let after = inputs(&[&source.replace(
                    "leaf_0(x ref Word) Word: return 1",
                    &format!(
                        "leaf_0(x ref Word) Word: {}{replacement}",
                        if replacement.starts_with('\n') {
                            ""
                        } else {
                            "return "
                        }
                    ),
                )]);
                let actual = check(&mut checker, &after, options.clone());
                let expected = clean(&after, options.clone());
                equivalent(&expected, &actual);
                let stats = checker.stats();
                assert!(stats.reference_checks <= 4, "{stats:?}");
                assert!(stats.effect_checks <= 6, "{stats:?}");
                assert!(stats.reference_checks_reused >= 64, "{stats:?}");
                assert!(stats.effect_checks_reused >= 64, "{stats:?}");
                assert_eq!(stats.type_declaration_reused, 0);
                assert_eq!(stats.signature_reused, 0);
                assert_eq!(stats.staging_reused, 0);
            }
            let mut fork = checker.fork_empty();
            assert!(fork.function_reuse);
            check(&mut fork, &before, options.clone());
            assert!(fork.stats.reference_checks >= 64);
            checker.clear();
            check(&mut checker, &before, options.clone());
            assert!(checker.stats.reference_checks >= 64);
        }
    }
}

#[test]
fn function_dependency_changes_recheck_callers_inside_the_same_file() {
    let mut source = String::from(WORD);
    source.push_str("select(x ref Word, y ref Word) ref Word: return x\ncaller(x ref Word, y ref Word) ref Word: return select(x, y)\n");
    for index in 0..32 {
        source.push_str(&format!("independent_{index}(x ref Word) Word: return 1\n"));
    }
    let before = inputs(&[&source]);
    let after = inputs(&[&source.replace("ref Word: return x\n", "ref Word: return y\n")]);
    for workers in [1, 2, 4] {
        let mut checker =
            PackageCheckSession::new(workers.try_into().unwrap(), BodyReuse::ExactInputs)
                .unwrap()
                .with_effect_queries()
                .with_function_reuse();
        let previous = check(&mut checker, &before, PackageTransformOptions::IDE);
        let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        let caller = crate::DefId::named(ast::prelude::Intern::from_ref("caller"));
        assert_ne!(
            previous.typed_asts[0].defs[&caller].references,
            actual.typed_asts[0].defs[&caller].references
        );
        assert!(checker.stats.reference_checks <= 6, "{:?}", checker.stats);
        assert!(
            checker.stats.reference_checks_reused >= 32,
            "{:?}",
            checker.stats
        );
        assert!(
            checker.stats.effect_checks_reused >= 32,
            "{:?}",
            checker.stats
        );
    }
}

#[test]
fn semantic_reuse_requires_explicit_opt_in() {
    let before = inputs(&[
        WORD,
        "generic(value): return value\n",
        "caller(x Word) Word: return generic(x)\n",
        "independent(x ref Word) Word: return 1\n",
    ]);
    let mut after = before.clone();
    after[3] = inputs(&["independent(x ref Word) Word: return 2\n"]).remove(0);
    for enabled in [false, true] {
        let mut checker =
            PackageCheckSession::new(std::num::NonZeroUsize::MIN, BodyReuse::ExactInputs)
                .unwrap()
                .with_retained_pass_limit(std::num::NonZeroUsize::MIN)
                .with_finishing_reuse()
                .with_effect_queries();
        assert!(!checker.semantic_reuse);
        if enabled {
            checker = checker.with_semantic_reuse();
        }
        assert_eq!(checker.fork_empty().semantic_reuse, enabled);
        check(&mut checker, &before, PackageTransformOptions::IDE);
        let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        assert_eq!(checker.stats.retained_passes, 1);
        if enabled {
            assert!(checker.stats.reference_checks_reused > 0);
            assert!(checker.stats.type_declaration_reused > 0);
        } else {
            assert_eq!(
                checker.stats.type_declaration_checks,
                checker.stats.declared_files
            );
            assert_eq!(checker.stats.signature_checks, checker.stats.declared_files);
            assert_eq!(checker.stats.type_declaration_reused, 0);
            assert_eq!(checker.stats.signature_reused, 0);
            assert_eq!(checker.stats.reference_checks_reused, 0);
            assert_eq!(checker.stats.effect_checks_reused, 0);
            assert_eq!(checker.stats.specialization_reused, 0);
            assert_eq!(checker.stats.staging_reused, 0);
            assert!(
                checker
                    .declaration_environment(&TransformCtx::new())
                    .is_none()
            );
            assert!(checker.stats.reused_files > 0);
        }
        checker.clear();
        assert_eq!(checker.semantic_reuse, enabled);
    }
}

#[test]
fn body_edits_reuse_semantic_answers_and_preserve_clean_results() {
    let before = inputs(&[
        WORD,
        "leaf(x ref Word) Word: return 1\n",
        "caller(x ref Word) Word: return leaf(x)\n",
        "independent(x ref Word) Word: return 3\n",
    ]);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        for workers in [1, 2, 4] {
            let mut checker = session(workers, BodyReuse::ExactInputs)
                .with_retained_pass_limit(1.try_into().unwrap());
            check(&mut checker, &before, options.clone());
            for body in ["2", "250", "1"] {
                let mut after = before.clone();
                after[1] = inputs(&[&format!(
                    "leaf(x ref Word) Word: {return_prefix}{body}\n",
                    return_prefix = if body.starts_with('\n') {
                        ""
                    } else {
                        "return "
                    }
                )])
                .remove(0);
                let actual = check(&mut checker, &after, options.clone());
                equivalent(&clean(&after, options.clone()), &actual);
                let stats = checker.stats();
                assert_eq!(stats.type_declaration_checks, 1, "{stats:?}");
                assert_eq!(stats.signature_checks, 1, "{stats:?}");
                assert_eq!(stats.type_declaration_reused, 3);
                assert_eq!(stats.signature_reused, 3);
                assert!(
                    stats.reference_checks_reused > stats.reference_checks,
                    "{stats:?}"
                );
                assert!(
                    stats.effect_checks_reused > stats.effect_checks,
                    "{stats:?}"
                );
            }
            checker.clear();
            check(&mut checker, &before, options.clone());
            assert_eq!(checker.stats.type_declaration_reused, 0);
            assert_eq!(checker.stats.signature_reused, 0);
            assert_eq!(checker.stats.reference_checks_reused, 0);
        }
    }
}

#[test]
fn changed_reference_answers_invalidate_callers_but_reuse_independent_checks() {
    let before = inputs(&[
        WORD,
        "select(x ref Word, y ref Word) ref Word: return x\n",
        "caller(x ref Word, y ref Word) ref Word: return select(x, y)\n",
        "independent(x Word) Word: return x\n",
    ]);
    let mut after = before.clone();
    after[1] = inputs(&["select(x ref Word, y ref Word) ref Word: return y\n"]).remove(0);
    for workers in [1, 2, 4] {
        let mut checker = session(workers, BodyReuse::ExactInputs);
        let previous = check(&mut checker, &before, PackageTransformOptions::IDE);
        let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        let caller = crate::DefId::named(ast::prelude::Intern::from_ref("caller"));
        assert_ne!(
            previous.typed_asts[2].defs[&caller].references,
            actual.typed_asts[2].defs[&caller].references
        );
        assert!(checker.stats.reference_checks >= 2);
        assert!(checker.stats.reference_checks_reused > 0);
        assert!(checker.stats.effect_checks_reused > 0);
    }
}

#[test]
fn specialization_plans_survive_unrelated_body_edits_and_refresh_template_edits() {
    let before = inputs(&[
        WORD,
        "generic(value): return value\n",
        "caller(x Word) Word: return generic(x)\n",
        "independent(x ref Word) Word: return 1\n",
    ]);
    for workers in [1, 2, 4] {
        let mut checker = session(workers, BodyReuse::ExactInputs);
        check(&mut checker, &before, PackageTransformOptions::IDE);
        let mut after = before.clone();
        after[3] = inputs(&["independent(x ref Word) Word: return 2\n"]).remove(0);
        let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        assert!(checker.stats.passes > 1);
        assert_eq!(
            checker.stats.lowered_files, checker.stats.passes,
            "{:?}",
            checker.stats
        );
        assert!(
            checker.stats.finishing_reused >= before.len() - 1,
            "{:?}",
            checker.stats
        );
        assert!(
            checker.stats.specialization_reused > 0,
            "{:?}",
            checker.stats
        );
        after[1] = inputs(&["generic(value):\n    unused: 1\n    return value\n"]).remove(0);
        let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        checker.clear();
        check(&mut checker, &after, PackageTransformOptions::IDE);
        assert_eq!(checker.stats.specialization_reused, 0);
    }
}

#[test]
fn session_boundaries_reject_unpaired_files() {
    for workers in [1, 2] {
        for (typed_count, source_count) in [(1, 0), (0, 1)] {
            for lowering in [true, false] {
                let mut checker = session(workers, BodyReuse::ExactInputs);
                checker.stats.passes = 1;
                let mut typed: Vec<_> = (0..typed_count)
                    .map(|id| TypedFileAst::new(FileId(id), ast::SpanTable::new()))
                    .collect();
                let source = FileAst::default();
                let sources = vec![&source; source_count];
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if lowering {
                        checker.lower(&mut typed, &sources, &TransformCtx::new());
                    } else {
                        checker.finish_with(
                            &mut typed,
                            &sources,
                            &TransformCtx::new(),
                            super::super::FinishStage::Lowered,
                        );
                    }
                }));
                let error = result.expect_err("unpaired files must be rejected");
                let message = error.downcast_ref::<String>().unwrap();
                assert!(message.contains("declared files and sources must have equal lengths"));
                assert!(checker.generations.is_empty());
                assert_eq!(checker.stats.lowered_files, 0);
            }
        }
    }
}

#[test]
#[should_panic(expected = "lowering requires an active package pass")]
fn session_boundaries_require_an_active_lowering_pass() {
    session(1, BodyReuse::ExactInputs).lower(&mut [], &[], &TransformCtx::new());
}

#[test]
fn session_boundaries_accept_an_empty_active_pass() {
    for workers in [1, 2] {
        let mut checker = session(workers, BodyReuse::ExactInputs);
        checker.stats.passes = 1;
        checker.lower(&mut [], &[], &TransformCtx::new());
        checker.finish_with(
            &mut [],
            &[],
            &TransformCtx::new(),
            super::super::FinishStage::Lowered,
        );
        assert_eq!(checker.generations.len(), 1);
        assert!(checker.generations[0].files.is_empty());
        assert_eq!(checker.stats.lowered_files, 0);
    }
}

#[test]
fn operator_candidate_edits_match_clean_checks() {
    let plain = "select(lhs ref Word, rhs ref Word) Word: return 1\n";
    let operator = format!("#infix('+')\n{plain}");
    let ambiguous =
        format!("{operator}#infix('+')\nother(lhs ref Word, rhs ref Word) Word: return 2\n");
    let consumer = "consumer(lhs ref Word, rhs ref Word) Word: return lhs + rhs\n";
    for order in [[0, 1, 2], [2, 1, 0], [1, 0, 2]] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            for reuse in [BodyReuse::Disabled, BodyReuse::ExactInputs] {
                for workers in [1, 2, 4, 8] {
                    let mut checker = session(workers, reuse);
                    for source in [
                        plain,
                        &operator,
                        &ambiguous,
                        &operator,
                        &operator.replace(": 1", ": 2"),
                        plain,
                        &operator,
                    ] {
                        let input = inputs(&[WORD, source, consumer]);
                        let input: Vec<_> =
                            order.iter().map(|index| input[*index].clone()).collect();
                        let expected = clean(&input, options.clone());
                        let actual = check(&mut checker, &input, options.clone());
                        equivalent(&expected, &actual);
                        let consumer_index = order.iter().position(|index| *index == 2).unwrap();
                        assert_eq!(
                            actual.typed_asts[consumer_index].all_flaws().is_empty(),
                            source.starts_with("#infix") && source != ambiguous,
                            "{source}: {:?}",
                            actual.typed_asts[consumer_index].all_flaws()
                        );
                    }
                }
            }
        }
    }
}

fn inputs(sources: &[&str]) -> Vec<FileAst> {
    sources
        .iter()
        .map(|source| {
            let mut parsed = source.parse_source_full();
            assert!(
                !parsed
                    .symptoms
                    .iter()
                    .any(|diagnostic| diagnostic.category == diagnostic::Category::Flaw),
                "{:?}",
                parsed.symptoms
            );
            crate::prepare_parse_ast(&mut parsed.ast, &flask::CompileTarget::Library);
            parsed.ast
        })
        .collect()
}

fn session(workers: usize, reuse: BodyReuse) -> PackageCheckSession {
    PackageCheckSession::new(workers.try_into().unwrap(), reuse)
        .unwrap()
        .with_effect_queries()
        .with_parallel_references()
        .with_finishing_reuse()
        .with_semantic_reuse()
}

fn package() -> ast::ty::PackageInstanceKey {
    ast::ty::PackageInstanceKey::workspace("dependency-fixture", "0")
}

fn clean(input: &[FileAst], options: PackageTransformOptions) -> PackageTransformArtifacts {
    transform_package_with_shared_context_and_package(input.to_vec(), options, package())
}

fn check(
    session: &mut PackageCheckSession,
    input: &[FileAst],
    options: PackageTransformOptions,
) -> PackageTransformArtifacts {
    transform_package_with_shared_context_and_session(input.to_vec(), options, package(), session)
}

fn equivalent(left: &PackageTransformArtifacts, right: &PackageTransformArtifacts) {
    assert_eq!(left.public_interface, right.public_interface);
    assert_eq!(left.interface_publication, right.interface_publication);
    assert_eq!(
        left.implementation_fingerprint,
        right.implementation_fingerprint
    );
    assert_eq!(left.typed_asts.len(), right.typed_asts.len());
    for (left, right) in left.typed_asts.iter().zip(&right.typed_asts) {
        assert!(
            exact_file_eq(left, right),
            "different file {}",
            left.file_id.0
        );
    }
}

#[test]
fn workers_preserve_typed_results_and_diagnostics() {
    let fixtures = [
        vec![
            WORD,
            "identity(x Word) Word: return x\n",
            "caller(x Word) Word: return identity(x)\n",
        ],
        vec![
            WORD,
            "bad(x Word) Word: return absent(x)\n",
            "mismatch(x Word) Word: return 'text'\n",
        ],
        vec![
            WORD,
            "choose(x Word) Word: return x\nchoose(x Word) Word: return x\n",
            "caller(x Word) Word: return choose(x)\n",
        ],
        vec![
            WORD,
            "forward(x ref Word) ref Word: return x\n",
            "caller(x ref Word) ref Word: return forward(x)\n",
        ],
        vec![
            WORD,
            "Cell has\n    value Word\n    finish(self):\n        return\nreplace(cell mut Cell, value Word):\n    cell.value:: value\n    return\n",
            "call(left mut Cell, right mut Cell, value Word):\n    replace(left, value)\n    return\n",
        ],
        vec![
            WORD,
            "generic(value): return value\n",
            "caller(x Word) Word: return generic(x)\n",
        ],
        vec![
            WORD,
            "first(x Word) Word: return second(x)\n",
            "second(x Word) Word: return first(x)\n",
        ],
        vec!["A is B\nB is A\n", "unresolved: absent\n"],
        vec![
            "Node has\n    next @Node\n    finish(self):\n        return\n",
            "use_node(value own Node) Node: return value\n",
        ],
    ];
    for sources in fixtures {
        let input = inputs(&sources);
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            let expected = clean(&input, options.clone());
            for workers in [1, 2, 4, 8] {
                let mut session = session(workers, BodyReuse::ExactInputs);
                equivalent(&expected, &check(&mut session, &input, options.clone()));
                equivalent(&expected, &check(&mut session, &input, options.clone()));
                assert_eq!(session.stats.lowered_files, 0);
                assert_eq!(session.stats.reused_files, session.stats.declared_files);
            }
        }
    }
}

#[test]
fn private_signature_and_compile_time_edits_match_clean_rebuilds() {
    let before = inputs(&[
        WORD,
        "identity(x Word) Word: return x\nprivate\nhelper(x ref Word) Word: return 1\n",
        "consumer(x ref Word) Word: return helper(x)\n",
    ]);
    for changed in [
        "identity(x Word) Word: return x\nprivate\nhelper(x ref Word) Word: return 2\n",
        "identity(x ref Word) Word: return 1\nprivate\nhelper(x ref Word) Word: return 1\n",
        "identity(x Word) Word: return x\nprivate\nhelper(x ref Word) Word: return 250\n",
    ] {
        let mut session = session(4, BodyReuse::ExactInputs);
        check(&mut session, &before, PackageTransformOptions::FULL);
        let mut after = before.clone();
        after[1] = inputs(&[changed]).remove(0);
        let actual = check(&mut session, &after, PackageTransformOptions::FULL);
        equivalent(&clean(&after, PackageTransformOptions::FULL), &actual);
        if changed.contains("identity(x ref Word)") {
            assert_eq!(session.stats.reused_files, 0);
        } else {
            assert_eq!(session.stats.reused_files, 2);
            assert_eq!(session.stats.lowered_files, 1);
        }
    }
}

#[test]
fn implementation_edit_updates_scalar_interface_and_invalidates_consumer() {
    let before = inputs(&[
        WORD,
        "compute(x is in Word) is in Word: return 1\n",
        "answer: compute(0)\n",
    ]);
    let mut after = before.clone();
    after[1] = inputs(&["compute(x is in Word) is in Word: return 2\n"]).remove(0);
    let mut session = session(4, BodyReuse::ExactInputs);
    let old = check(&mut session, &before, PackageTransformOptions::FULL);
    let actual = check(&mut session, &after, PackageTransformOptions::FULL);
    equivalent(&clean(&after, PackageTransformOptions::FULL), &actual);
    assert_ne!(old.public_interface, actual.public_interface);
    for (artifacts, value) in [(&old, 1), (&actual, 2)] {
        let compute = &artifacts.typed_asts[1].defs
            [&crate::DefId::named(ast::prelude::Intern::from_ref("compute"))];
        assert_eq!(
            compute.effects.scalar_result,
            Some(ast::NormalExpr::from(value))
        );
    }
    assert_eq!(
        old.implementation_fingerprint,
        actual.implementation_fingerprint
    );
    assert_ne!(old.typed_asts[2].exprs, actual.typed_asts[2].exprs);
    assert!(session.stats.reused_files > 0);
}

#[test]
fn changed_overload_candidates_and_missing_names_invalidate() {
    let cases = [
        (
            "choose(x Word) Word: return x\n",
            "choose(x Word) Word: return x\nchoose(x Word) Word: return x\n",
            "caller(x Word) Word: return choose(x)\n",
            "type-ambiguous-overload",
        ),
        (
            "other(x Word) Word: return x\n",
            "other(x Word) Word: return x\nabsent(x Word) Word: return x\n",
            "caller(x Word) Word: return absent(x)\n",
            "",
        ),
    ];
    for (old, new, caller, expected) in cases {
        let before = inputs(&[WORD, old, caller]);
        let after = inputs(&[WORD, new, caller]);
        let mut session = session(4, BodyReuse::ExactInputs);
        check(&mut session, &before, PackageTransformOptions::IDE);
        let actual = check(&mut session, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        let flaws = actual.typed_asts[2].all_flaws();
        if expected.is_empty() {
            assert!(flaws.is_empty(), "{flaws:?}");
        } else {
            assert!(
                flaws.iter().any(|(_, flaw)| flaw.code.slug() == expected),
                "{flaws:?}"
            );
        }
        assert_eq!(session.stats.reused_files, 0);
    }
}

#[test]
fn declaration_traversal_order_is_a_cache_input() {
    let before = inputs(&[
        WORD,
        "first(x ref Word) Word: return 1\nsecond(x ref Word) Word: return 2\n",
    ]);
    let mut after = before.clone();
    let first = *after[1].defs.get_index(0).unwrap().0;
    let bind = after[1].defs.shift_remove(&first).unwrap();
    after[1].defs.insert(first, bind);
    assert_eq!(before, after);
    let mut session = session(4, BodyReuse::ExactInputs);
    check(&mut session, &before, PackageTransformOptions::IDE);
    let actual = check(&mut session, &after, PackageTransformOptions::IDE);
    equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
    assert_eq!(session.stats.reused_files, 0);
}

#[test]
fn ordered_parameter_metadata_is_a_cache_input() {
    let mut before = inputs(&[WORD, "first(x ref Word, y ref Word) Word: return 1\n"]);
    let bind = before[1].defs.values_mut().next().unwrap();
    let names: Vec<_> = bind.params.as_ref().unwrap().keys().copied().collect();
    for name in names {
        bind.param_slots.insert(
            name,
            ast::ParamSlot {
                ty: ast::TyState::Infer,
                default: None,
                convention: ast::ParamConvention::Observe,
            },
        );
        bind.param_conventions
            .insert(name, ast::ParamConvention::Observe);
    }
    for field in 0..3 {
        let mut after = before.clone();
        let bind = after[1].defs.values_mut().next().unwrap();
        match field {
            0 => bind.params.as_mut().unwrap().swap_indices(0, 1),
            1 => bind.param_slots.swap_indices(0, 1),
            _ => bind.param_conventions.swap_indices(0, 1),
        }
        assert_eq!(before, after);
        let mut session = session(4, BodyReuse::ExactInputs);
        check(&mut session, &before, PackageTransformOptions::IDE);
        let actual = check(&mut session, &after, PackageTransformOptions::IDE);
        equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
        assert_eq!(session.stats.reused_files, 0);
    }
}

#[test]
fn local_expression_edit_reuses_other_files() {
    let before = inputs(&[WORD, "identity(x Word) Word: return x\n", "1\n"]);
    let mut after = before.clone();
    after[2] = inputs(&["2\n"]).remove(0);
    let mut session = session(4, BodyReuse::ExactInputs);
    check(&mut session, &before, PackageTransformOptions::IDE);
    let actual = check(&mut session, &after, PackageTransformOptions::IDE);
    equivalent(&clean(&after, PackageTransformOptions::IDE), &actual);
    assert_eq!(session.stats.reused_files, 2);
    assert_eq!(session.stats.lowered_files, 1);
    assert_eq!(session.stats.finished_files, 3);
}

#[test]
fn target_mode_identity_and_locations_are_cache_inputs() {
    let input = inputs(&[WORD, "identity(x Word) Word: return x\n"]);
    let mut session = session(4, BodyReuse::ExactInputs);
    check(&mut session, &input, PackageTransformOptions::IDE);
    for options in [
        PackageTransformOptions::FULL,
        PackageTransformOptions::FULL.with_compile_target(flask::CompileTarget::Concrete(
            flask::TargetTriple::parse("aarch64-apple-darwin").unwrap(),
        )),
    ] {
        equivalent(
            &clean(&input, options.clone()),
            &check(&mut session, &input, options),
        );
        assert_eq!(session.stats.reused_files, 0);
    }
    let moved = inputs(&[WORD, "\n\nidentity(x Word) Word: return x\n"]);
    equivalent(
        &clean(&moved, PackageTransformOptions::IDE),
        &check(&mut session, &moved, PackageTransformOptions::IDE),
    );
    assert_eq!(session.stats.reused_files, 0);
    let other_package = ast::ty::PackageInstanceKey::workspace("other", "0");
    let expected = transform_package_with_shared_context_and_package(
        moved.clone(),
        PackageTransformOptions::IDE,
        other_package.clone(),
    );
    let actual = transform_package_with_shared_context_and_session(
        moved,
        PackageTransformOptions::IDE,
        other_package,
        &mut session,
    );
    equivalent(&expected, &actual);
    assert_eq!(session.stats.reused_files, 0);
}

#[test]
fn supplied_file_ids_survive_body_scheduling_and_input_permutations() {
    let input: Vec<_> = inputs(&[
        "first(x is in 0 to 255) is in 0 to 255: return x\n",
        "second(x is in 0 to 255) is in 0 to 255: return first(x)\n",
        "third(x is in 0 to 255) is in 0 to 255: return second(x)\n",
    ])
    .into_iter()
    .enumerate()
    .map(|(id, file)| (file, FileId(id as u32 + 10)))
    .collect();
    let mut merged = FileAst::default();
    for (file, _) in &input {
        merged.merge_from(file.clone());
    }
    let context = TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(merged));
    let expected = transform_package(&input, &context, PackageTransformOptions::IDE);
    for permutation in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let input: Vec<_> = permutation.map(|index| input[index].clone()).into();
        for workers in [1, 2, 4, 8] {
            let mut session = session(workers, BodyReuse::ExactInputs);
            for _ in 0..2 {
                let actual = session.transform(&input, &context, PackageTransformOptions::IDE);
                for typed in actual {
                    let expected = expected
                        .iter()
                        .find(|expected| expected.file_id == typed.file_id)
                        .unwrap();
                    assert!(exact_file_eq(expected, &typed), "file {}", typed.file_id.0);
                }
            }
        }
    }
}

#[test]
fn declaration_alias_dependencies_are_independent_of_file_order() {
    let expected = assert_declaration_orders(&["A is B\n", "B is C\n", "C is in 0 to 255\n"]);
    let tag = |name| crate::TagId(ast::prelude::Intern::from_ref(name));
    assert_eq!(
        expected[0].tags[&tag("A")].resolved_ty,
        expected[2].tags[&tag("C")].resolved_ty
    );
    assert_eq!(
        expected[1].tags[&tag("B")].resolved_ty,
        expected[2].tags[&tag("C")].resolved_ty
    );
}

#[test]
fn cache_input_equality_covers_diagnostics_and_ownership_metadata() {
    let file = crate::transform::stage_declare(
        &inputs(&[WORD]).remove(0),
        FileId(0),
        &TransformCtx::new(),
    );
    let mut changed = file.clone();
    changed
        .parse_warnings
        .push(diagnostic::Diagnostic::new(TypeCode::Mismatch, "changed"));
    assert_eq!(file, changed);
    assert!(!exact_file_eq(&file, &changed));
    let mut changed = file.clone();
    changed.places.push(crate::typed::TypedPlace {
        binder: ast::BinderId::new(
            0,
            ast::BinderOwner::Definition(ast::DefinitionId::named(ast::prelude::Intern::from_ref(
                "fixture",
            ))),
        ),
        name: ast::prelude::Intern::from_ref("fixture"),
        parent: None,
        projection: None,
        mutable: false,
        explicit_contract: false,
        ty: ast::Ty::Unit,
    });
    assert_eq!(file, changed);
    assert!(!exact_file_eq(&file, &changed));
}

#[test]
fn clearing_and_removing_files_retires_cached_work() {
    let before = inputs(&[WORD, "identity(x Word) Word: return x\n"]);
    let mut session = session(4, BodyReuse::ExactInputs);
    check(&mut session, &before, PackageTransformOptions::IDE);
    session.clear();
    check(&mut session, &before, PackageTransformOptions::IDE);
    assert_eq!(session.stats.reused_files, 0);
    let after = &before[..1];
    equivalent(
        &clean(after, PackageTransformOptions::IDE),
        &check(&mut session, after, PackageTransformOptions::IDE),
    );
    assert_eq!(session.generations[0].files.len(), 1);
    let empty = check(&mut session, &[], PackageTransformOptions::IDE);
    assert!(empty.typed_asts.is_empty());
    assert!(session.generations[0].files.is_empty());
}

fn declaration_fixture(sources: &[&str]) -> (Vec<(FileAst, FileId)>, TransformCtx) {
    let files: Vec<_> = inputs(sources)
        .into_iter()
        .enumerate()
        .map(|(index, file)| (file, FileId(index as u32 + 20)))
        .collect();
    let mut merged = FileAst::default();
    for (file, _) in &files {
        merged.merge_from(file.clone());
    }
    (
        files,
        TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(merged)),
    )
}

fn assert_declaration_orders(sources: &[&str]) -> Vec<TypedFileAst> {
    let (input, context) = declaration_fixture(sources);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        let expected = transform_package(&input, &context, options.clone());
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let files: Vec<_> = order.map(|index| input[index].clone()).into();
            for workers in [1, 2, 4, 8] {
                let mut session = session(workers, BodyReuse::ExactInputs);
                for _ in 0..2 {
                    let actual = session.transform(&files, &context, options.clone());
                    for typed in actual {
                        let expected = expected
                            .iter()
                            .find(|expected| expected.file_id == typed.file_id)
                            .unwrap();
                        assert!(exact_file_eq(expected, &typed), "file {}", typed.file_id.0);
                    }
                }
            }
        }
    }
    transform_package(&input, &context, PackageTransformOptions::IDE)
}

#[test]
fn cross_file_alias_cycles_have_source_located_order_independent_diagnostics() {
    let expected = assert_declaration_orders(&["A is B\n", "B is A\n", "C is A\n"]);
    for file in expected {
        let diagnostics: Vec<_> = file
            .declaration_flaws
            .iter()
            .filter(|(_, flaw)| flaw.code.slug() == "recursive-type-representation")
            .collect();
        assert_eq!(diagnostics.len(), 1, "{:?}", file.all_flaws());
        assert_eq!(diagnostics[0].1.span, file.span_table.get(diagnostics[0].0));
    }
}

#[test]
fn aliases_to_recursive_pointer_identities_are_order_independent() {
    let expected = assert_declaration_orders(&[
        "Alias is Node\n",
        "Node is @Alias\n",
        "consume(value own Alias) Alias: return value\n",
    ]);
    for file in expected {
        assert!(
            file.all_flaws()
                .iter()
                .all(|(_, flaw)| flaw.code.slug() != "recursive-type-representation"),
            "{:?}",
            file.all_flaws()
        );
    }
}

#[test]
fn alias_chains_are_not_limited_by_tag_fixpoint_passes() {
    let mut source = String::new();
    for index in 0..40 {
        source.push_str(&format!("Alias{index} is Alias{}\n", index + 1));
    }
    source.push_str("Alias40 is End\nEnd is in 0 to 255\n");
    let (files, context) = declaration_fixture(&[&source]);
    let typed = transform_package(&files, &context, PackageTransformOptions::IDE).remove(0);
    let first = crate::TagId(ast::prelude::Intern::from_ref("Alias0"));
    let end = crate::TagId(ast::prelude::Intern::from_ref("End"));
    assert_eq!(typed.tags[&first].resolved_ty, typed.tags[&end].resolved_ty);
}

#[test]
fn alias_lookup_and_signature_edits_match_clean_rebuilds() {
    let (before, old_context) = declaration_fixture(&["A is B\n", "B is Missing\n", "1\n"]);
    let mut session = session(4, BodyReuse::ExactInputs);
    session.transform(&before, &old_context, PackageTransformOptions::IDE);
    for source in ["Missing is in 0 to 255\n", "Missing is in 0 to 65535\n"] {
        let (after, context) = declaration_fixture(&["A is B\n", "B is Missing\n", source]);
        let expected = transform_package(&after, &context, PackageTransformOptions::IDE);
        let actual = session.transform(&after, &context, PackageTransformOptions::IDE);
        assert!(
            expected
                .iter()
                .zip(&actual)
                .all(|(expected, actual)| exact_file_eq(expected, actual))
        );
        let a = crate::TagId(ast::prelude::Intern::from_ref("A"));
        let target = crate::TagId(ast::prelude::Intern::from_ref("Missing"));
        assert_eq!(
            actual[0].tags[&a].resolved_ty,
            actual[2].tags[&target].resolved_ty
        );
        assert_eq!(session.stats.reused_files, 0);
    }
}

#[test]
fn alias_to_recursive_record_still_requires_a_finite_representation() {
    let expected =
        assert_declaration_orders(&["Alias is Node\n", "Node has\n    next Alias\n", "1\n"]);
    assert!(
        expected[1]
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "recursive-type-representation")
    );
}

#[test]
fn signature_group_paths_use_completed_cross_file_record_definitions() {
    let expected = assert_declaration_orders(&[
        "write(cell ref{storage} Cell, slot mut{storage.value} Word):\n    return\n",
        "Cell has\n    value Word\n    finish(self):\n        return\n",
        WORD,
    ]);
    assert!(
        expected.iter().all(|file| file.all_flaws().is_empty()),
        "{:?}",
        expected
            .iter()
            .map(TypedFileAst::all_flaws)
            .collect::<Vec<_>>()
    );
}

#[test]
fn generic_signature_group_paths_use_completed_cross_file_record_definitions() {
    let expected = assert_declaration_orders(&[
        "write(cell ref{storage} Cell(Word), slot mut{storage.value} Word):\n    return\n",
        "Cell(item) has\n    value item\n    finish(self):\n        return\n",
        WORD,
    ]);
    assert!(expected.iter().all(|file| file.all_flaws().is_empty()));
}

#[test]
fn completed_registry_edits_match_clean_signature_checks() {
    let signature = "write(cell ref{storage} Cell, slot mut{storage.value} Word):\n    return\n";
    let (before, context) = declaration_fixture(&[
        signature,
        "Cell has\n    other Word\n    finish(self):\n        return\n",
        WORD,
    ]);
    let mut session = session(4, BodyReuse::ExactInputs);
    let old = session.transform(&before, &context, PackageTransformOptions::IDE);
    assert!(
        old[0]
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-unknown-target-group-field")
    );
    for record in [
        "Cell has\n    value Word\n    finish(self):\n        return\n",
        "\n\nCell has\n    value @Word\n    finish(self):\n        return\n",
    ] {
        let (after, context) = declaration_fixture(&[signature, record, WORD]);
        let expected = transform_package(&after, &context, PackageTransformOptions::IDE);
        let actual = session.transform(&after, &context, PackageTransformOptions::IDE);
        assert!(
            expected
                .iter()
                .zip(&actual)
                .all(|(expected, actual)| exact_file_eq(expected, actual))
        );
        assert!(
            actual[0]
                .all_flaws()
                .iter()
                .all(|(_, flaw)| flaw.code.slug() != "type-unknown-target-group-field")
        );
        assert_eq!(session.stats.reused_files, 0);
    }
}

#[test]
fn completed_signature_context_keeps_each_union_candidate_once() {
    let expected = assert_declaration_orders(&[
        "Flag is Yes or No\n",
        "State is Ready or Waiting\n",
        "pass(value Flag) Flag: return value\n",
    ]);
    for file in expected {
        for name in ["Yes", "No", "Ready", "Waiting"] {
            let name = ast::prelude::Intern::from_ref(name);
            assert_eq!(file.variant_map[&name].len(), 1);
        }
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn generic_alias_templates_are_resolved_before_consumers() {
    let expected = assert_declaration_orders(&[
        "A is B(Word)\n",
        "B(element) is ref element\n",
        "Word is in 0 to 255\n",
    ]);
    let ty = &expected[0].tags[&crate::TagId(ast::prelude::Intern::from_ref("A"))].resolved_ty;
    assert!(matches!(ty, ast::Ty::Ref { inner, mutable: false }
        if matches!(&**inner, ast::Ty::Named { instance, .. } if instance.declaration.file == 22)));
}

#[test]
fn generic_alias_application_dependencies_can_chain() {
    let expected = assert_declaration_orders(&[
        "A is B(Word)\nWord is in 0 to 255\n",
        "B(element) is C(element)\n",
        "C(element) is ref element\n",
    ]);
    let ty = &expected[0].tags[&crate::TagId(ast::prelude::Intern::from_ref("A"))].resolved_ty;
    assert!(matches!(ty, ast::Ty::Ref { .. }));
}

#[test]
fn generic_alias_cycles_keep_existing_diagnostics_for_each_order() {
    let (input, context) = declaration_fixture(&[
        "A(element) is B(element)\n",
        "B(element) is A(element)\n",
        "C is A(Missing)\n",
    ]);
    for order in [[0, 1, 2], [1, 0, 2], [2, 1, 0]] {
        let files: Vec<_> = order.map(|index| input[index].clone()).into();
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            let expected = transform_package(&files, &context, options.clone());
            assert!(expected.iter().any(|file| {
                file.declaration_flaws
                    .iter()
                    .any(|(_, flaw)| flaw.code.slug() == "recursive-type-representation")
            }));
            for workers in [1, 2, 4, 8] {
                let mut checker = session(workers, BodyReuse::ExactInputs);
                let actual = checker.transform(&files, &context, options.clone());
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert!(exact_file_eq(actual, expected));
                }
            }
        }
    }
}

#[test]
fn generic_alias_edits_and_failed_lookup_answers_match_clean_rebuilds() {
    let initial = inputs(&["A is B(Missing)\n", "B(element) is ref element\n", "1\n"]);
    let mut checker = session(4, BodyReuse::ExactInputs);
    for sources in [
        initial,
        inputs(&[
            "A is B(Missing)\n",
            "B(element) is ref element\n",
            "Missing is in 0 to 255\n",
        ]),
        inputs(&[
            "A is B(Missing)\n",
            "B(element) is mut element\n",
            "Missing is in 0 to 255\n",
        ]),
    ] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            equivalent(
                &check(&mut checker, &sources, options.clone()),
                &clean(&sources, options),
            );
        }
    }
}

#[test]
fn imported_generic_argument_edits_match_clean_checks() {
    let consumer = "consumer(value ref Word):\n    slot Pair(first: Word): Pair(left: 1 as Word, right: 2 as Word)\n    slot.finish\n    return\n";
    let original = "Pair(first: Word, second: Word) has\n    left first\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n";
    let renamed = "Pair(alpha: Word, second: Word) has\n    left alpha\n    right second\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n";
    let without_parameters = "Pair has\n    left Word\n    right Word\n    finish(self):\n        self.left.finish\n        self.right.finish\n        return\n";
    for order in [[0, 1, 2], [2, 1, 0]] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            for reuse in [BodyReuse::Disabled, BodyReuse::ExactInputs] {
                for workers in [1, 4] {
                    let mut checker = session(workers, reuse);
                    for (step, (provider, expected)) in [
                        (original, None),
                        (renamed, Some("type-unknown-generic-argument")),
                        (without_parameters, Some("type-excess-generic-arguments")),
                        (original, None),
                        (original, None),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let parsed = inputs(&[WORD, provider, consumer]);
                        let input: Vec<_> =
                            order.iter().map(|index| parsed[*index].clone()).collect();
                        let expected_result = clean(&input, options.clone());
                        let actual = check(&mut checker, &input, options.clone());
                        equivalent(&expected_result, &actual);
                        let index = order.iter().position(|index| *index == 2).unwrap();
                        let flaws = actual.typed_asts[index].all_flaws();
                        if let Some(code) = expected {
                            assert!(
                                flaws.iter().any(|(_, flaw)| flaw.code.slug() == code),
                                "{provider}: {flaws:?}"
                            );
                        } else {
                            assert!(flaws.is_empty(), "{provider}: {flaws:?}");
                        }
                        if step == 4 && reuse == BodyReuse::ExactInputs {
                            assert!(checker.stats().reused_files > 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn recursive_reference_aliases_keep_existing_representation_rules() {
    let expected = assert_declaration_orders(&[
        "Loop(element) is ref Loop(element)\n",
        "Word is in 0 to 255\n",
        "1\n",
    ]);
    assert!(expected.iter().all(|file| {
        file.declaration_flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() != "recursive-type-representation")
    }));
}

#[test]
fn lowering_reads_cross_file_constant_implementations_in_ide_mode() {
    let before = inputs(&[
        WORD,
        "private\nseed Word: 1\n",
        "consumer(x Word) Word: return seed\n",
    ]);
    let mut checker = session(4, BodyReuse::ExactInputs);
    let old = check(&mut checker, &before, PackageTransformOptions::IDE);
    let old_signatures = checker.generations[0]
        .context
        .cross_file_callable_signatures
        .clone();
    let old_lowered = checker.generations[0]
        .files
        .values()
        .find(|file| {
            file.source
                .defs
                .keys()
                .any(|name| name.0.as_str() == "consumer")
        })
        .unwrap()
        .result
        .exprs
        .clone();
    let mut after = before.clone();
    after[1] = inputs(&["private\nseed Word: 2\n"]).remove(0);
    let actual = check(&mut checker, &after, PackageTransformOptions::IDE);
    equivalent(&actual, &clean(&after, PackageTransformOptions::IDE));
    assert_ne!(old.public_interface, actual.public_interface);
    for (artifacts, value) in [(&old, 1), (&actual, 2)] {
        let consumer = &artifacts.typed_asts[2].defs
            [&crate::DefId::named(ast::prelude::Intern::from_ref("consumer"))];
        assert_eq!(
            consumer.effects.scalar_result,
            Some(ast::NormalExpr::from(value))
        );
    }
    assert_eq!(
        old_signatures,
        checker.generations[0]
            .context
            .cross_file_callable_signatures
    );
    let new_lowered = &checker.generations[0]
        .files
        .values()
        .find(|file| {
            file.source
                .defs
                .keys()
                .any(|name| name.0.as_str() == "consumer")
        })
        .unwrap()
        .result
        .exprs;
    assert_ne!(&old_lowered, new_lowered);
    assert_eq!(checker.stats().reused_files, 1);
    assert_eq!(checker.stats().lowered_files, 2);
    let consumer = checker.generations[0]
        .files
        .values()
        .find(|file| {
            file.source
                .defs
                .keys()
                .any(|name| name.0.as_str() == "consumer")
        })
        .unwrap();
    assert_eq!(
        consumer
            .reads
            .constants
            .get(&crate::DefId::named(ast::prelude::Intern::from_ref("seed"))),
        Some(&Some(ast::ConstValue::Int(2.into())))
    );
}

#[test]
fn private_body_reuse_matches_clean_rebuilds_for_orders_and_workers() {
    let before = inputs(&[
        WORD,
        "private\nhelper(x ref Word) Word: return 1\n",
        "consumer(x ref Word) Word: return helper(x)\n",
    ]);
    let mut after = before.clone();
    after[1] = inputs(&["private\nhelper(x ref Word) Word: return 2\n"]).remove(0);
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let before: Vec<_> = order.iter().map(|i| before[*i].clone()).collect();
        let after: Vec<_> = order.iter().map(|i| after[*i].clone()).collect();
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            let expected = clean(&after, options.clone());
            for workers in [1, 2, 4, 8] {
                let mut checker = session(workers, BodyReuse::ExactInputs);
                check(&mut checker, &before, options.clone());
                equivalent(&expected, &check(&mut checker, &after, options.clone()));
                assert_eq!(checker.stats().lowered_files, 1);
                assert_eq!(checker.stats().reused_files, 2);
                equivalent(&expected, &check(&mut checker, &after, options.clone()));
                assert_eq!(checker.stats().lowered_files, 0);
            }
        }
    }
}

#[test]
fn constant_answers_stop_invalidation_after_unrelated_and_equal_value_edits() {
    let before = inputs(&[
        WORD,
        "private\nseed Word: compute(0)\n",
        "compute(x is in Word) is in Word: return 1 + 0\nprivate\nhelper(x ref Word) Word: return 1\n",
        "consumer(x Word) Word: return seed\n",
        "other(x Word) Word: return seed\n",
    ]);
    for replacement in [
        "compute(x is in Word) is in Word: return 1 + 0\nprivate\nhelper(x ref Word) Word: return 2\n",
        "compute(x is in Word) is in Word: return 0 + 1\nprivate\nhelper(x ref Word) Word: return 1\n",
        "compute(x is in Word) is in Word: return 2 + 0\nprivate\nhelper(x ref Word) Word: return 1\n",
    ] {
        let mut after = before.clone();
        after[2] = inputs(&[replacement]).remove(0);
        for order in [[0, 1, 2, 3, 4], [2, 1, 0, 4, 3], [1, 2, 0, 3, 4]] {
            let before: Vec<_> = order.iter().map(|i| before[*i].clone()).collect();
            let after: Vec<_> = order.iter().map(|i| after[*i].clone()).collect();
            for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
                for workers in [1, 2, 4, 8] {
                    let mut checker = session(workers, BodyReuse::ExactInputs);
                    check(&mut checker, &before, options.clone());
                    equivalent(
                        &clean(&after, options.clone()),
                        &check(&mut checker, &after, options.clone()),
                    );
                    if replacement.contains("is in Word: return 2") {
                        assert_eq!(checker.stats().lowered_files, 3);
                        assert_eq!(checker.stats().reused_files, 2);
                    } else {
                        assert_eq!(
                            checker.stats().lowered_files,
                            1,
                            "{replacement:?} {options:?} workers={workers}"
                        );
                        assert_eq!(checker.stats().reused_files, 4);
                    }
                    assert_eq!(checker.stats().constant_queries, 1);
                    assert_eq!(checker.stats().constant_environments, 1);
                }
            }
        }
    }
}

#[test]
fn failed_constant_answers_are_confirmed_again_when_implementations_change() {
    let before = inputs(&[
        WORD,
        "seed Word: compute(0)\n",
        "compute(x is in Word) is in Word: return absent(x)\n",
        "consumer(x Word) Word: return seed\n",
    ]);
    let mut after = before.clone();
    after[2] = inputs(&["compute(x is in Word) is in Word: return 2\n"]).remove(0);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        let mut checker = session(4, BodyReuse::ExactInputs);
        check(&mut checker, &before, options.clone());
        equivalent(
            &clean(&after, options.clone()),
            &check(&mut checker, &after, options),
        );
        assert_eq!(checker.stats().lowered_files, 2);
    }
}

#[test]
fn constant_queries_are_shared_without_retaining_body_results() {
    let before = inputs(&[
        WORD,
        "compute(x is in Word) is in Word: return 1\n",
        "seed Word: compute(0)\n",
        "first(x Word) Word: return seed\n",
        "second(x Word) Word: return seed\n",
    ]);
    let mut after = before.clone();
    after[1] = inputs(&["compute(x is in Word) is in Word: return 2\n"]).remove(0);
    for order in [[0, 1, 2, 3, 4], [4, 3, 2, 1, 0], [2, 0, 4, 1, 3]] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            for workers in [1, 2, 4, 8] {
                let mut checker = session(workers, BodyReuse::Disabled);
                for input in [&before, &after, &before] {
                    let input: Vec<_> = order.iter().map(|index| input[*index].clone()).collect();
                    equivalent(
                        &clean(&input, options.clone()),
                        &check(&mut checker, &input, options.clone()),
                    );
                    assert_eq!(checker.stats().constant_queries, 1);
                    assert_eq!(checker.stats().constant_environments, 1);
                    assert_eq!(checker.stats().lowered_files, input.len());
                    assert_eq!(checker.stats().reused_files, 0);
                    assert!(checker.generations.is_empty());
                }
            }
        }
    }
}

#[test]
fn untracked_constant_queries_leave_local_typed_evaluation_in_place() {
    let input = inputs(&[&format!(
        "{WORD}seed Word: 1\nconsumer(x Word) Word: return seed\n"
    )]);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        for workers in [1, 2, 4, 8] {
            let mut checker = session(workers, BodyReuse::Disabled);
            equivalent(
                &clean(&input, options.clone()),
                &check(&mut checker, &input, options.clone()),
            );
            assert_eq!(checker.stats().constant_queries, 0);
            assert_eq!(checker.stats().constant_environments, 0);
            assert!(checker.generations.is_empty());
        }
    }
}

#[test]
fn domain_reads_include_transitive_bindings_and_failed_lookups() {
    use crate::transform::implementation_reads::ImplementationReadRecorder;
    use ast::prelude::Intern;

    let before = inputs(&["root: leaf\nleaf: 1\n"]).remove(0);
    let types = HashMap::new();
    let reads = ImplementationReadRecorder::new();
    let domains = crate::domain::Domains::new(crate::analysis::TypeEnv::new(&types), &before.tags)
        .with_file(&before)
        .with_reads(Some(&reads));
    assert!(
        domains
            .value(&ast::Expr::AnonymousTag(Intern::from_ref("root")))
            .unwrap()
            .is_some()
    );
    assert!(
        domains
            .value(&ast::Expr::AnonymousTag(Intern::from_ref("missing")))
            .unwrap()
            .is_none()
    );
    let reads = reads.into_reads();
    for name in ["root", "leaf", "missing"] {
        assert!(
            reads
                .bindings
                .contains(&crate::DefId::named(Intern::from_ref(name)))
        );
    }
    let changed = inputs(&["root: leaf\nleaf: 2\n"]).remove(0);
    assert!(!reads.unchanged(
        &before,
        &changed,
        &super::super::constant_queries::ConstantQueries::new(std::sync::Arc::new(changed.clone()))
    ));
    let resolved = inputs(&["root: leaf\nleaf: 1\nmissing: 3\n"]).remove(0);
    assert!(!reads.unchanged(
        &before,
        &resolved,
        &super::super::constant_queries::ConstantQueries::new(std::sync::Arc::new(
            resolved.clone()
        ))
    ));
}

#[test]
fn domain_implementation_edits_recheck_consumers() {
    let before = inputs(&[
        WORD,
        "Interval(lo to hi) is lo to hi\nInterval(lo to hi) has\n    lower: lo\n",
        "private\nseed Word: 1\n",
        "consumer(x Word) Word: return Interval(seed to 9).lower\n",
    ]);
    let mut after = before.clone();
    after[2] = inputs(&["private\nseed Word: 2\n"]).remove(0);
    for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
        for workers in [1, 2, 4, 8] {
            let mut checker = session(workers, BodyReuse::ExactInputs);
            let old = check(&mut checker, &before, options.clone());
            let actual = check(&mut checker, &after, options.clone());
            equivalent(&clean(&after, options.clone()), &actual);
            assert_ne!(old.typed_asts[3].exprs, actual.typed_asts[3].exprs);
            assert_eq!(checker.stats().reused_files, 2);
            let consumer = checker.generations[0]
                .files
                .values()
                .find(|file| {
                    file.source
                        .defs
                        .keys()
                        .any(|id| id.0.as_str() == "consumer")
                })
                .unwrap();
            assert!(
                consumer
                    .reads
                    .bindings
                    .contains(&crate::DefId::named(ast::prelude::Intern::from_ref("seed")))
            );
            assert!(consumer.reads.constants.is_empty());
        }
    }
}

#[test]
fn compile_time_transitive_implementation_edits_match_clean_rebuilds() {
    let before = inputs(&[
        WORD,
        "offset(x is in Word) is in Word: return 1\n",
        "compute(x is in Word) is in Word: return offset(x)\n",
        "answer: compute(0)\n",
    ]);
    let mut after = before.clone();
    after[1] = inputs(&["offset(x is in Word) is in Word: return 2\n"]).remove(0);
    for workers in [1, 2, 4, 8] {
        let mut checker = session(workers, BodyReuse::ExactInputs);
        let old = check(&mut checker, &before, PackageTransformOptions::FULL);
        let actual = check(&mut checker, &after, PackageTransformOptions::FULL);
        equivalent(&clean(&after, PackageTransformOptions::FULL), &actual);
        assert_ne!(old.typed_asts[3].exprs, actual.typed_asts[3].exprs);
        assert!(checker.stats().reused_files > 0);
    }
}

#[test]
fn trait_default_edits_and_implementation_availability_match_clean_rebuilds() {
    let before = inputs(&[
        WORD,
        "Provider has\n    get(self ref) Word: return 1\n",
        "Cell has Provider\n    value Word\n    finish(self):\n        return\n",
        "consumer(x ref Cell) Word: return x.get()\n",
    ]);
    let mut changed_default = before.clone();
    changed_default[1] = inputs(&["Provider has\n    get(self ref) Word: return 2\n"]).remove(0);
    let mut added_implementation = before.clone();
    added_implementation.push(inputs(&["Cell.get(self ref) Word: return 2\n"]).remove(0));
    for after in [changed_default, added_implementation] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            for workers in [1, 2, 4, 8] {
                let mut checker = session(workers, BodyReuse::ExactInputs);
                check(&mut checker, &before, options.clone());
                equivalent(
                    &clean(&after, options.clone()),
                    &check(&mut checker, &after, options.clone()),
                );
                assert_eq!(checker.stats().reused_files, 0);
            }
        }
    }
}

#[test]
fn reference_guards_inside_generic_alias_templates_keep_valid_cycles() {
    let expected = assert_declaration_orders(&[
        "A is B(A)\n",
        "B(element) is ref element\n",
        "Word is in 0 to 255\n",
    ]);
    assert!(matches!(
        &expected[0].tags[&crate::TagId(ast::prelude::Intern::from_ref("A"))].resolved_ty,
        ast::Ty::Ref { .. }
    ));
    assert!(expected.iter().all(|file| {
        file.declaration_flaws
            .iter()
            .all(|(_, flaw)| flaw.code.slug() != "recursive-type-representation")
    }));
}

#[test]
fn empty_forks_share_workers_without_inheriting_results() {
    let mut parent = session(4, BodyReuse::ExactInputs);
    let input = inputs(&[WORD, "identity(x Word) Word: return x\n"]);
    check(&mut parent, &input, PackageTransformOptions::IDE);
    let mut child = parent.fork_empty();
    assert!(std::sync::Arc::ptr_eq(
        parent.pool.as_ref().unwrap(),
        child.pool.as_ref().unwrap()
    ));
    assert!(child.generations.is_empty());
    assert_eq!(child.stats().passes, 0);
    check(&mut child, &input, PackageTransformOptions::IDE);
    assert_eq!(child.stats().reused_files, 0);
    check(&mut parent, &input, PackageTransformOptions::IDE);
    assert!(parent.stats().reused_files > 0);
}

#[test]
fn limited_pass_retention_matches_clean_checks_across_edits() {
    let initial = inputs(&[
        WORD,
        "identity(value): return value\ncompute(x is in Word) is in Word: return 1\n",
        "consumer(x Word) Word: return identity(x)\nanswer: compute(0)\nunresolved: absent\n",
    ]);
    let mut implementation = initial.clone();
    implementation[1] =
        inputs(&["identity(value): return value\ncompute(x is in Word) is in Word: return 2\n"])
            .remove(0);
    let mut resolved = implementation.clone();
    resolved.push(inputs(&["absent: 1\n"]).remove(0));
    let mut overload = resolved.clone();
    overload[1] = inputs(&["identity(value): return value\nidentity(value ref Word) Word: return value\ncompute(x is in Word) is in Word: return 2\n"]).remove(0);
    let mut signature = overload.clone();
    signature[0] = inputs(&[&WORD.replace("255", "511")]).remove(0);
    for order in [[0, 1, 2], [1, 2, 0], [2, 1, 0]] {
        for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
            for workers in [1, 2, 4, 8] {
                for limit in [1, 2] {
                    let mut checker = session(workers, BodyReuse::ExactInputs)
                        .with_retained_pass_limit(limit.try_into().unwrap());
                    for (step, sources) in [
                        &initial,
                        &initial,
                        &implementation,
                        &resolved,
                        &overload,
                        &signature,
                        &initial,
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let mut ordered: Vec<_> =
                            order.iter().map(|i| sources[*i].clone()).collect();
                        ordered.extend_from_slice(&sources[3..]);
                        equivalent(
                            &clean(&ordered, options.clone()),
                            &check(&mut checker, &ordered, options.clone()),
                        );
                        assert!(checker.generations.len() <= limit);
                        assert_eq!(checker.stats().retained_passes, checker.generations.len());
                        if step == 1 {
                            assert!(checker.stats().reused_files > 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn retention_limit_is_preserved_when_forking_and_clearing() {
    let input = inputs(&[WORD, "identity(x Word) Word: return x\n"]);
    let mut parent =
        session(4, BodyReuse::ExactInputs).with_retained_pass_limit(1.try_into().unwrap());
    check(&mut parent, &input, PackageTransformOptions::IDE);
    let mut child = parent.fork_empty();
    assert_eq!(child.retained_pass_limit, parent.retained_pass_limit);
    assert!(child.generations.is_empty());
    check(&mut child, &input, PackageTransformOptions::IDE);
    assert_eq!(child.stats().retained_passes, 1);
    parent.clear();
    assert!(parent.generations.is_empty());
    check(&mut parent, &input, PackageTransformOptions::IDE);
    assert_eq!(parent.stats().retained_passes, 1);
    let disabled = session(1, BodyReuse::Disabled).with_retained_pass_limit(1.try_into().unwrap());
    assert!(disabled.generations.is_empty());
}

#[test]
fn lowering_past_the_retention_limit_keeps_no_additional_snapshots() {
    let (files, mut context) = declaration_fixture(&[WORD, "identity(x Word) Word: return x\n"]);
    let sources: Vec<_> = files.iter().map(|(source, _)| source).collect();
    for limit in [1, 2, 3] {
        let mut checker = session(4, BodyReuse::ExactInputs);
        for round in 0..4 {
            checker.stats.passes = round + 1;
            let mut declared = Vec::new();
            for (source, id) in &files {
                declared.push(super::super::stage_declare(source, *id, &context));
            }
            checker.lower(&mut declared, &sources, &context);
        }
        assert_eq!(checker.generations.len(), 4);
        checker = checker.with_retained_pass_limit(limit.try_into().unwrap());
        assert_eq!(checker.generations.len(), limit);
        checker.stats.passes = 5;
        let mut declared: Vec<_> = files
            .iter()
            .map(|(source, id)| super::super::stage_declare(source, *id, &context))
            .collect();
        checker.lower(&mut declared, &sources, &context);
        assert_eq!(checker.generations.len(), limit);
        context.ide_package = !context.ide_package;
    }
}

#[test]
fn effect_query_policy_survives_empty_forks_and_clear() {
    let defaults = PackageCheckSession::new(1.try_into().unwrap(), BodyReuse::Disabled).unwrap();
    assert!(!defaults.effect_queries);
    let mut enabled = defaults.with_effect_queries();
    assert!(enabled.fork_empty().effect_queries);
    check(
        &mut enabled,
        &inputs(&[
            WORD,
            "leaf(x Word) Word: return x\ncaller(x Word) Word: return leaf(x)\n",
        ]),
        PackageTransformOptions::IDE,
    );
    assert!(enabled.stats().effect_checks > 0);
    enabled.clear();
    assert!(enabled.effect_queries);
}

#[test]
fn reference_policy_survives_empty_forks_and_clear() {
    let defaults = PackageCheckSession::new(2.try_into().unwrap(), BodyReuse::Disabled).unwrap();
    assert!(!defaults.parallel_references);
    let mut enabled = defaults.with_parallel_references();
    assert!(enabled.fork_empty().parallel_references);
    check(
        &mut enabled,
        &inputs(&[
            WORD,
            "leaf(x ref Word) ref Word: return x\ncaller(x ref Word) ref Word: return leaf(x)\n",
        ]),
        PackageTransformOptions::IDE,
    );
    assert!(enabled.stats().reference_checks > 0);
    assert!(enabled.stats().reference_rounds > 0);
    enabled.clear();
    assert!(enabled.parallel_references);
}

#[test]
fn conditional_proof_contexts_follow_edited_branches_and_imported_answers() {
    let helper = "choose(value is in 0 to 255) is in 0 to 255: return when value is in 0 to 15 then value else 0\n";
    let changed = helper.replace("else 0", "else 255");
    let consumer = "narrow(value is in 0 to 255) is in 0 to 15: return choose(value)\n";
    for order in [[0, 1], [1, 0]] {
        for workers in [1, 2, 4, 8] {
            for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
                let mut checker = session(workers, BodyReuse::ExactInputs);
                for source in [helper, &changed, helper] {
                    let input = inputs(&[source, consumer]);
                    let input: Vec<_> = order.iter().map(|index| input[*index].clone()).collect();
                    equivalent(
                        &clean(&input, options.clone()),
                        &check(&mut checker, &input, options.clone()),
                    );
                }
            }
        }
    }
}

#[test]
fn finishing_reuse_matches_clean_for_cross_file_constant_and_signature_edits() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "private\nbound Word: 1\n",
            "private\nbound Word: 2\n",
            "narrow(x is in 0 to bound) is in 0 to bound: return x\n",
        ),
        (
            "private\nhelper(x Word) Word: return x\n",
            "private\nhelper(x Word) Word: return x + 0\n",
            "caller(x Word) Word: return helper(x)\n",
        ),
        (
            "private\ncompute(x is in Word) is in Word: return 1\n",
            "private\ncompute(x is in Word) is in Word: return 2\n",
            "consumer(x Word) Word: return compute(0)\n",
        ),
    ];
    for (before, after, consumer) in cases {
        for order in [[0, 1, 2], [2, 1, 0]] {
            for options in [PackageTransformOptions::IDE, PackageTransformOptions::FULL] {
                for workers in [1, 2, 4, 8] {
                    let mut checker = session(workers, BodyReuse::ExactInputs);
                    let initial = inputs(&[WORD, before, consumer]);
                    let initial: Vec<_> = order.iter().map(|i| initial[*i].clone()).collect();
                    check(&mut checker, &initial, options.clone());
                    let edited = inputs(&[WORD, after, consumer]);
                    let edited: Vec<_> = order.iter().map(|i| edited[*i].clone()).collect();
                    equivalent(
                        &clean(&edited, options.clone()),
                        &check(&mut checker, &edited, options.clone()),
                    );
                }
            }
        }
    }
}
