use std::collections::HashMap;

use parser::query::SourceParseExt;
use resolve::{ParsedFile, resolve_imports};
use test_fixtures::TempPackage;

fn resolve(source: &str, developer_installed: bool) -> Vec<ParsedFile> {
    resolve_package(source, &[], developer_installed)
}

fn resolve_package(
    source: &str,
    extra_files: &[(&str, &str)],
    developer_installed: bool,
) -> Vec<ParsedFile> {
    let package = TempPackage::new("conditional_imports");
    package.write_flask_with_deps(
        "example",
        r#"dev: "./dev"
"#,
    );
    if developer_installed {
        package.write(
            "dev/mod.flask",
            r#"#settings
module: "dev"
version: 0.0.0

#dependencies
"#,
        );
        package.write("dev/tools/tools.gin", "show_wireframe: 42\n");
    }
    let path = package.write("main.gin", source);
    for (path, contents) in extra_files {
        package.write(path, contents);
    }
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    resolve_imports(
        vec![ParsedFile {
            path,
            source: source.to_owned(),
            output: parsed,
        }],
        &HashMap::new(),
    )
}

#[test]
fn release_does_not_resolve_an_uninstalled_developer_dependency() {
    let files = resolve(
        "main: when 0 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse 7\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(
        files[0].output.symptoms.is_empty(),
        "{:?}",
        files[0].output.symptoms
    );
}

#[test]
fn known_function_argument_selects_imports_before_resolution() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else\n        return 7\n\n#entry\nmain: run(0)\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(
        files[0].output.symptoms.is_empty(),
        "{:?}",
        files[0].output.symptoms
    );
}

#[test]
fn developer_selection_loads_the_module() {
    let files = resolve(
        "main: when 1 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse 7\n",
        true,
    );
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
}

#[test]
fn developer_selection_reports_the_missing_dependency() {
    let files = resolve(
        "main: when 1 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse 7\n",
        false,
    );
    assert!(
        files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .any(|diagnostic| diagnostic.code.slug() == "use-dependency-missing-config")
    );
}

#[test]
fn unresolved_runtime_condition_gets_an_import_diagnostic() {
    let files = resolve(
        "#entry\nmain(value):\n    when value = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n",
        false,
    );
    let diagnostics: Vec<_> = files
        .iter()
        .flat_map(|file| &file.output.symptoms)
        .collect();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "use-condition-not-compile-time"),
        "{diagnostics:?}"
    );
    assert!(
        !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.slug() == "use-dependency-missing-config")
    );
}

#[test]
fn both_known_specializations_require_the_developer_dependency_once() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nrelease: run(0)\ndeveloper: run(1)\n",
        true,
    );
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
    let main = files
        .iter()
        .find(|file| file.path.ends_with("main.gin"))
        .unwrap();
    assert_eq!(main.output.ast.uses.len(), 1);
    assert_eq!(
        main.output
            .ast
            .defs
            .values()
            .filter(|bind| bind.name.as_str().starts_with("run$imports$"))
            .count(),
        2
    );
}

#[test]
fn known_inputs_propagate_through_a_wrapper() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nlaunch(mode): return run(mode)\n#entry\nmain: launch(0)\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(
        files[0].output.symptoms.is_empty(),
        "{:?}",
        files[0].output.symptoms
    );
}

#[test]
fn a_dead_call_does_not_activate_its_imports() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nmain: when 0 = 1 then run(1) else 7\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(
        files[0].output.symptoms.is_empty(),
        "{:?}",
        files[0].output.symptoms
    );
}

fn check(files: &[ParsedFile]) -> Vec<typecheck::TypedFileAst> {
    let target = flask::CompileTarget::Library;
    let mut files: Vec<_> = files.iter().map(|file| file.output.ast.clone()).collect();
    for file in &mut files {
        let diagnostics = typecheck::prepare_parse_ast(file, &target);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
    let mut evaluation = ast::FileAst::default();
    for file in &files {
        evaluation.merge_from(file.clone());
    }
    let context = typecheck::transform::TransformCtx::with_package_compile_time_arc(
        std::sync::Arc::new(evaluation),
    );
    let files: Vec<_> = files
        .into_iter()
        .enumerate()
        .map(|(index, file)| (file, typecheck::FileId(index as u32)))
        .collect();
    typecheck::transform::transform_package(
        &files,
        &context,
        typecheck::transform::PackageTransformOptions::FULL,
    )
}

#[test]
fn selected_branch_is_typechecked_after_import_resolution() {
    let files = resolve(
        "main: when 1 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse absent\n",
        true,
    );
    let typed = check(&files);
    for file in &typed {
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn import_alias_does_not_escape_its_branch() {
    let files = resolve(
        "main: when 1 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse 7\n\noutside: tools.show_wireframe\n",
        true,
    );
    let typed = check(&files);
    assert!(
        typed
            .iter()
            .flat_map(|file| file.all_flaws())
            .any(|(_, diagnostic)| diagnostic.code.slug() == "type-unknown-symbol")
    );
}

#[test]
fn static_inner_import_can_have_a_runtime_outer_condition() {
    let files = resolve(
        "#entry\nmain(value):\n    when value = 1 then\n        when 1 = 1 then\n            use dev.tools as tools\n            return tools.show_wireframe\n        else return 7\n    else return 7\n",
        true,
    );
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
}

#[test]
fn selected_pattern_bindings_survive_branch_selection() {
    let files = resolve(
        "main: when 3 is\n    count then\n        use dev.tools as tools\n        count\n    else absent\n",
        true,
    );
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
    for file in check(&files) {
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn ordered_selection_does_not_require_later_conditions_to_be_known() {
    let files = resolve(
        "#entry\nmain(value):\n    when\n        1 = 1 then return 7\n        value = 1 then\n            use dev.tools as tools\n            return tools.show_wireframe\n        else return 9\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(
        files[0].output.symptoms.is_empty(),
        "{:?}",
        files[0].output.symptoms
    );
}

#[test]
fn unknown_parameter_does_not_fall_back_to_a_global_with_the_same_name() {
    let files = resolve(
        "value := 1\n#entry\nmain(value):\n    when value = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n",
        false,
    );
    assert!(
        files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .any(|diagnostic| diagnostic.code.slug() == "use-condition-not-compile-time")
    );
}

#[test]
fn repeated_file_imports_share_a_namespace_without_leaking_local_aliases() {
    let files = resolve(
        "left: when 1 = 1 then\n    use './dev/tools/tools.gin' as first\n    first.show_wireframe\nelse 0\n\nright: when 1 = 1 then\n    use './dev/tools/tools.gin' as second\n    second.show_wireframe\nelse 0\n",
        true,
    );
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
    for file in check(&files) {
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn selected_member_imports_are_scoped_to_their_branch() {
    let files = resolve(
        "main: when 1 = 1 then\n    use dev.tools show_wireframe as draw\n    draw\nelse absent\n",
        true,
    );
    assert_eq!(files.len(), 2);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
    for file in check(&files) {
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn specialization_does_not_hide_invalid_call_arity() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nmain: run(0, 1)\n",
        false,
    );
    assert!(
        files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .any(|diagnostic| diagnostic.code.slug() == "use-specialization-arity")
    );
}

#[test]
fn mutable_parameter_aliases_do_not_leave_a_stale_compile_time_condition() {
    let files = resolve(
        "run(flag):\n    mutate(flag)\n    when flag = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nmutate(value mut): return\nmain: run(0)\n",
        false,
    );
    assert!(
        files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .any(|diagnostic| diagnostic.code.slug() == "use-condition-not-compile-time")
    );
}

#[test]
fn named_specialization_arguments_select_the_release_branch() {
    let files = resolve(
        "run(mode):\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\n\nmain: run(mode: 0)\n",
        false,
    );
    assert_eq!(files.len(), 1);
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
}

#[test]
fn specialization_preserves_parameter_type_checks() {
    let files = resolve(
        "Mode is in 1 to 2\nrun(mode Mode) Mode:\n    when mode = 1 then\n        use dev.tools as tools\n        return 1\n    else return 2\n\nmain: run(0)\n",
        false,
    );
    assert!(files.iter().all(|file| file.output.symptoms.is_empty()));
    let typed = check(&files);
    let flaws: Vec<_> = typed.iter().flat_map(|file| file.all_flaws()).collect();
    assert!(
        flaws.iter().any(|(_, diagnostic)| matches!(
            diagnostic.code.slug().as_str(),
            "type-out-of-range" | "type-integer-literal-out-of-range"
        )),
        "{flaws:?}"
    );
}

const RUNNER: &str = "Code is in 0 to 255\nCode has\n    finish(self):\n        return\nrun(mode, result Code) Code:\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return result\n";

fn accepted(files: &[ParsedFile]) {
    for file in files {
        assert!(
            file.output.symptoms.is_empty(),
            "{}: {:?}",
            file.path.display(),
            file.output.symptoms
        );
    }
    for file in check(files) {
        assert!(file.all_flaws().is_empty(), "{:?}", file.all_flaws());
    }
}

#[test]
fn named_arguments_are_matched_by_name_instead_of_source_order() {
    let files = resolve(&format!("{RUNNER}\nmain: run(result: 7, mode: 0)\n"), false);
    assert_eq!(files.len(), 1);
    accepted(&files);
}

#[test]
fn named_and_positional_arguments_fill_distinct_parameter_slots() {
    let files = resolve(&format!("{RUNNER}\nmain: run(result: 7, 0)\n"), false);
    assert_eq!(files.len(), 1);
    accepted(&files);
}

#[test]
fn named_arguments_propagate_through_wrappers_without_shadowing_locals() {
    let files = resolve(
        &format!(
            "{RUNNER}\nlaunch(mode, result): return run(result: result, mode: mode)\nmain: launch(result: 7, mode: 0)\n"
        ),
        false,
    );
    assert_eq!(files.len(), 1);
    accepted(&files);
}

#[test]
fn invalid_named_arguments_are_diagnosed_before_dependency_resolution() {
    for (arguments, expected) in [
        ("other: 0, result: 7", "use-specialization-argument-name"),
        ("mode: 0, mode: 1", "use-specialization-argument-duplicate"),
        ("mode: 0", "use-specialization-arity"),
    ] {
        let files = resolve(&format!("{RUNNER}\nmain: run({arguments})\n"), false);
        let codes: Vec<_> = files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .map(|diagnostic| diagnostic.code.slug())
            .collect();
        assert!(
            codes.iter().any(|code| code == expected),
            "{arguments}: {codes:?}"
        );
        assert!(
            !codes
                .iter()
                .any(|code| code == "use-dependency-missing-config")
        );
    }
}

#[test]
fn cross_file_named_call_selects_release_without_developer_tools() {
    let files = resolve_package(
        "use './workers/runner.gin' as runner\nmain: runner.run(result: 7, mode: 0)\n",
        &[("workers/runner.gin", RUNNER)],
        false,
    );
    assert_eq!(files.len(), 2);
    accepted(&files);
}

#[test]
fn cross_file_wrappers_propagate_named_arguments_across_multiple_hops() {
    let files = resolve_package(
        "use './launcher.gin' as launcher\nmain: launcher.launch(result: 7, mode: 0)\n",
        &[
            (
                "launcher.gin",
                "use './workers/runner.gin' as runner\nlaunch(mode, result): return runner.run(result: result, mode: mode)\n",
            ),
            ("workers/runner.gin", RUNNER),
        ],
        false,
    );
    assert_eq!(files.len(), 3);
    accepted(&files);
}

#[test]
fn cross_file_developer_call_loads_tools_in_the_function_source_file() {
    let files = resolve_package(
        "use './workers/runner.gin' as runner\nmain: runner.run(result: 7, mode: 1)\n",
        &[("workers/runner.gin", RUNNER)],
        true,
    );
    assert_eq!(files.len(), 3);
    accepted(&files);
    assert!(
        files
            .iter()
            .find(|file| file.path.ends_with("workers/runner.gin"))
            .unwrap()
            .output
            .ast
            .uses
            .len()
            == 1
    );
}

#[test]
fn cross_file_missing_developer_import_reports_the_source_file() {
    let files = resolve_package(
        "use './workers/runner.gin' as runner\nmain: runner.run(result: 7, mode: 1)\n",
        &[("workers/runner.gin", RUNNER)],
        false,
    );
    let failed = files
        .iter()
        .find(|file| {
            file.output
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.code.slug() == "use-dependency-missing-config")
        })
        .unwrap();
    assert!(failed.path.ends_with("workers/runner.gin"));
}

#[test]
fn cross_file_selected_local_import_is_relative_to_the_function_source() {
    let files = resolve_package(
        "use './workers/runner.gin' as runner\nmain: runner.run(mode: 0)\n",
        &[
            (
                "workers/runner.gin",
                "Code is in 0 to 255\nCode has\n    finish(self):\n        return\nrun(mode) Code:\n    when mode = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else\n        use './support.gin' as support\n        return support.result\n",
            ),
            ("workers/support.gin", "result: 7\n"),
        ],
        false,
    );
    assert_eq!(files.len(), 3);
    accepted(&files);
}

#[test]
fn cross_file_runtime_import_condition_is_diagnosed_without_loading_tools() {
    let files = resolve_package(
        "use './workers/runner.gin' as runner\n#entry\nmain(mode): return runner.run(result: 7, mode: mode)\n",
        &[("workers/runner.gin", RUNNER)],
        false,
    );
    let codes: Vec<_> = files
        .iter()
        .flat_map(|file| &file.output.symptoms)
        .map(|diagnostic| diagnostic.code.slug())
        .collect();
    assert!(
        codes
            .iter()
            .any(|code| code == "use-condition-not-compile-time"),
        "{codes:?}"
    );
    assert!(
        !codes
            .iter()
            .any(|code| code == "use-dependency-missing-config")
    );
}

#[test]
fn conditional_member_alias_can_call_an_already_loaded_import_template() {
    let files = resolve_package(
        "use './workers' run\nmain: when 1 = 1 then\n    use './workers' run as launch\n    launch(result: 7, mode: 0)\nelse 9\n",
        &[("workers/runner.gin", RUNNER)],
        false,
    );
    assert_eq!(files.len(), 2);
    accepted(&files);
}

#[test]
fn selected_import_uses_the_callee_packages_dependency_aliases() {
    let files = resolve_package(
        "use library.runner as runner\nmain: runner.run(result: 7, mode: 1)\n",
        &[
            (
                "mod.flask",
                r#"#settings
module: "example"
version: 0.0.0

#dependencies
library: "./lib"
"#,
            ),
            (
                "lib/mod.flask",
                r#"#settings
module: "library"
version: 0.0.0

#dependencies
dev: "../developer"
"#,
            ),
            ("lib/runner/runner.gin", RUNNER),
            (
                "developer/mod.flask",
                r#"#settings
module: "dev"
version: 0.0.0

#dependencies
"#,
            ),
            ("developer/tools/tools.gin", "show_wireframe: 42\n"),
        ],
        false,
    );
    assert_eq!(files.len(), 3);
    accepted(&files);
}

#[test]
fn mixed_named_mutable_arguments_do_not_leave_a_stale_condition() {
    let files = resolve(
        "run(flag):\n    mutate(other: 7, flag)\n    when flag = 1 then\n        use dev.tools as tools\n        return tools.show_wireframe\n    else return 7\nmutate(value mut, other): return\nmain: run(0)\n",
        false,
    );
    assert!(
        files
            .iter()
            .flat_map(|file| &file.output.symptoms)
            .any(|diagnostic| diagnostic.code.slug() == "use-condition-not-compile-time")
    );
}
