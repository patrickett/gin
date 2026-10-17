//! `when` exhaustiveness: optional `else` when all cases are covered.

mod support;

use std::collections::HashMap;
use std::path::PathBuf;

use diagnostic::Diagnostic;
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile, resolve_imports};
use support::has_flaw_matching as has_flaw;
use support::transform_source;
use test_fixtures::TempPackage;
use typecheck::FileId;
use typecheck::transform::{PackageTransformOptions, TransformCtx, transform_package};

fn has_parse_warning(typed: &typecheck::TypedFileAst) -> bool {
    !typed.parse_warnings.is_empty()
}

fn is_missing_else(f: &Diagnostic) -> bool {
    f.code.slug() == "type-missing-else-arm"
}

fn is_unreachable_else(f: &Diagnostic) -> bool {
    f.code.slug() == "type-unreachable-else-arm"
}

fn is_unreachable_arm(f: &Diagnostic) -> bool {
    f.code.slug() == "type-unreachable-when-arm"
}

fn is_wildcard_pattern(f: &Diagnostic) -> bool {
    f.code.slug() == "type-wildcard-when-pattern"
}

fn transform_resolved_package(files: Vec<(PathBuf, String)>) -> Vec<typecheck::TypedFileAst> {
    let mut deps = HashMap::new();
    let mut parsed = Vec::new();
    for (path, source) in files {
        if deps.is_empty() {
            deps = path.flask_path_dependencies();
        }
        parsed.push(ParsedFile {
            path,
            source: source.clone(),
            output: source.parse_source_full(),
        });
    }
    let mut resolved = resolve_imports(parsed, &deps);
    let entry = flask::CompileTarget::Library;
    let mut asts: Vec<_> = resolved
        .iter_mut()
        .map(|file| file.output.ast.clone())
        .collect();
    let diags = typecheck::prepare_package_asts(&mut asts, &entry);
    for (file, (ast, diags)) in resolved.iter_mut().zip(asts.into_iter().zip(diags)) {
        file.output.ast = ast;
        file.output.symptoms.extend(diags);
    }
    let mut compile_time_eval_ast = ast::FileAst::default();
    for file in &resolved {
        compile_time_eval_ast.merge_from(file.output.ast.clone());
    }
    let ctx =
        TransformCtx::with_package_compile_time_arc(std::sync::Arc::new(compile_time_eval_ast));
    let file_asts: Vec<_> = resolved
        .into_iter()
        .enumerate()
        .map(|(i, file)| (file.output.ast, FileId(i as u32)))
        .collect();
    transform_package(&file_asts, &ctx, PackageTransformOptions::FULL)
}

#[test]
fn maybe_exhaustive_without_else() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: return when m is
                              Some(x) then x
                              None then 0
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "expected no MissingElseArm when all Maybe variants are covered: {:?}",
        typed.all_flaws()
    );
    assert!(!has_flaw(&typed, is_unreachable_else));
}

#[test]
fn maybe_non_exhaustive_without_else() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: return when m is Some(x) then x
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_missing_else),
        "expected MissingElseArm when None arm is missing"
    );
}

#[test]
fn maybe_unreachable_else_hint() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: return when m is
                              Some(x) then x
                              None then 0
                              else 99
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_else),
        "expected UnreachableElseArm when all cases are already covered"
    );
    assert!(!has_flaw(&typed, is_missing_else));
}

#[test]
fn nonexhaustive_subject_when_still_requires_else() {
    let source = "Bool is True or False\nInt is in 0 to 1000\nInt has\n    finish(self):\n        return\nfoo(x Int) Int: return when x is 10 then x else 0";
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));

    let source = "Bool is True or False\nInt is in 0 to 1000\nInt has\n    finish(self):\n        return\nfoo(x Int) Int: return when x is 10 then x";
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn bool_union_exhaustive_without_else() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         True then 1
                         False then 0
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "expected True/False to cover Bool: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn bool_union_partial_requires_else() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is True then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn lowercase_binding_pattern_is_exhaustive() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is value then 1
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "a lowercase binding should cover Bool: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn lowercase_binding_makes_else_unreachable() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0 to 1000
Int has
    finish(self):
        return
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(has_flaw(&typed, is_unreachable_else));
}

#[test]
fn arm_after_else_is_rejected() {
    let source = "Bool is True or False\nInt is in 0 to 1000\nInt has\n    finish(self):\n        return\npick(b Bool) Int: return when b is\n    True then 1\n    else 0\n    False then 2\n";
    let typed = transform_source(source);
    assert!(
        has_parse_warning(&typed),
        "arm after else should be rejected: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn range_pattern_matches_compile_time() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
classify(n Int) String: return when n is in 0 to 9 then 'small'
                               else 'large'
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
}

#[test]
fn range_pattern_exhaustive_for_literal_union() {
    let source = r#"
Digit is 0 or 1 or 2 or 3
Int is in 0 to 1000
Int has
    finish(self):
        return
parse(d Digit) Int: return when d is
                          in 0 to 2 then 1
                          3 then 0
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "range 0 to 2 plus literal 3 should cover Digit: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_fixed_length_plus_cons_catch_all_exhaustive() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
List(x) is [] or [x, ...List(x)]
len(xs List(Int)) Int: return when xs is
    [] then 0
    [h, ...tail] then 1
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "[] + [h, ...tail] should be exhaustive: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn range_pattern_partial_requires_else() {
    let source = r#"
Digit is 0 or 1 or 2
Int is in 0 to 1000
Int has
    finish(self):
        return
parse(d Digit) Int: return when d is in 0 to 1 then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn semantic_duplicate_value_pattern_is_unreachable() {
    // 'x' and 'y' are different names but resolve to the same const value (False).
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
x Bool: False
y Bool: False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         x then 1
                         y then 2
                         true then 3
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_arm),
        "second `is true` should be unreachable: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn known_subject_value_selects_matching_is_arm_and_else_is_dropped() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0 to 1000
Int has
    finish(self):
        return
something() Int:
    b: false
    x: when b is false then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    // The subject b:false matches `is false`, making the else arm unreachable.
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(
        has_flaw(&typed, is_unreachable_else),
        "else should be unreachable when subject matches is false: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn imported_lowercase_name_still_binds_in_pattern_position() {
    let pkg = TempPackage::new("when_imported_bool_values");
    pkg.write_flask_with_deps(
        "app",
        r#"core: "./."
"#,
    );
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive Bool, true, false

Int is in 0 to 1000
Int has
    finish(self):
        return
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let main_path = pkg.write("app/main.gin", main_src);
    let typed = transform_resolved_package(vec![
        (
            bool_path,
            std::fs::read_to_string(pkg.join("primitive/bool.gin")).unwrap(),
        ),
        (main_path, main_src.to_string()),
    ]);
    let main_typed = typed
        .iter()
        .find(|file| {
            file.defs
                .keys()
                .any(|def| def.0.as_str().ends_with("something"))
        })
        .expect("main file typed output");
    assert!(
        !has_flaw(main_typed, is_missing_else),
        "a lowercase pattern should be exhaustive even when a value has the same name: {:?}",
        main_typed.all_flaws()
    );
    assert!(has_flaw(main_typed, is_unreachable_else));
}

#[test]
fn imported_qualified_variant_pattern_is_detected_as_duplicate() {
    let pkg = TempPackage::new("when_imported_qualified_variant");
    pkg.write_flask_with_deps(
        "app",
        r#"core: "./."
"#,
    );
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive Bool, false

Int is in 0 to 1000
Int has
    finish(self):
        return
something() Int:
    b: false
    x: when b is Bool.True then 1
                    else 2
    return x
"#;
    let main_path = pkg.write("app/main.gin", main_src);
    let typed = transform_resolved_package(vec![
        (
            bool_path,
            std::fs::read_to_string(pkg.join("primitive/bool.gin")).unwrap(),
        ),
        (main_path, main_src.to_string()),
    ]);
    let main_typed = typed
        .iter()
        .find(|file| {
            file.defs
                .keys()
                .any(|def| def.0.as_str().ends_with("something"))
        })
        .expect("main file typed output");
    // Bool.True should resolve as Tag { name: "True" }, and b:false means it doesn't match
    assert!(
        has_flaw(main_typed, is_unreachable_arm),
        "imported `Bool.True` should be unreachable when subject b is false: {:?}",
        main_typed.all_flaws()
    );
}

#[test]
fn imported_qualified_variant_duplicate_of_nominal() {
    // Bool.True and True should be detected as duplicates via semantic resolution
    let pkg = TempPackage::new("when_imported_qualified_dup");
    pkg.write_flask_with_deps(
        "app",
        r#"core: "./."
"#,
    );
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive Bool

Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         True then 1
                         Bool.True then 2
                         else 0
"#;
    let main_path = pkg.write("app/main.gin", main_src);
    let typed = transform_resolved_package(vec![
        (
            bool_path,
            std::fs::read_to_string(pkg.join("primitive/bool.gin")).unwrap(),
        ),
        (main_path, main_src.to_string()),
    ]);
    let main_typed = typed
        .iter()
        .find(|file| file.defs.keys().any(|def| def.0.as_str().ends_with("pick")))
        .expect("main file typed output");
    assert!(
        has_flaw(main_typed, is_unreachable_arm),
        "Bool.True should be unreachable when True already covers it: {:?}",
        main_typed.all_flaws()
    );
}

#[test]
fn literal_union_exhaustive_without_else() {
    let source = r#"
Arch is 'x86_64' or 'aarch64' or 'wasm32'
Int is in 0 to 1000
Int has
    finish(self):
        return
ptr(a Arch) Int: return when a is
                       'x86_64' or 'aarch64' then 8
                       'wasm32' then 4
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "expected all Arch literals to be covered: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn literal_union_partial_requires_else() {
    let source = r#"
Arch is 'x86_64' or 'aarch64' or 'wasm32'
Int is in 0 to 1000
Int has
    finish(self):
        return
ptr(a Arch) Int: return when a is 'x86_64' or 'aarch64' then 8
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn wildcard_pattern_is_prohibited_use_else_instead() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is _ then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_wildcard_pattern));
}

#[test]
fn lowercase_pattern_is_a_catch_all() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is value then 1
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
}

#[test]
fn list_empty_and_cons_catch_all_are_exhaustive_without_else() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
List(x) is [] or [x, ...List(x)]
len(xs List(Int)) Int: return when xs is
    [] then 0
    [head, ...tail] then 1
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "[] plus cons with catch-all tail should cover List: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_only_empty_requires_else() {
    let source = r#"
Int is in 0 to 1000
Int has
    finish(self):
        return
List(x) is [] or [x, ...List(x)]
len(xs List(Int)) Int: return when xs is [] then 0
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn else_is_allowed_when_union_is_partial() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is True then 1
                         else 0
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(!has_flaw(&typed, is_unreachable_else));
}

#[test]
fn duplicate_pattern_arm_is_unreachable() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         True then 1
                         True then 2
                         False then 0
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_unreachable_arm));
}

#[test]
fn boolean_condition_arm_requires_an_explicit_pattern() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when
    b then 2
    else 0
"#;
    let typed = transform_source(source);
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| { flaw.code.slug() == "type-condition-requires-proposition" })
    );
}

#[test]
fn bounded_int_fully_covered_by_ranges_no_else() {
    let source = r#"
Int is in 0 to 5
Int has
    finish(self):
        return
parse(n Int) Int: return when n is
    in 0 to 2 then 1
    in 3 to 5 then 2
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "expected 0..2 + 3..5 to cover 0..5: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn bounded_int_partially_covered_by_range_requires_else() {
    let source = r#"
Int is in 0 to 5
Int has
    finish(self):
        return
parse(n Int) Int: return when n is in 0 to 2 then 1
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_missing_else),
        "expected MissingElseArm when only 0..2 is covered"
    );
}

#[test]
fn qualified_variant_pattern_duplicate_of_lowercase_value() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         true then 1
                         Bool.True then 2
                         else 0
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_arm),
        "`is Bool.True` should be unreachable because `true` already matches True: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn qualified_variant_pattern_duplicate_of_nominal_variant() {
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
                         True then 1
                         Bool.True then 2
                         else 0
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_arm),
        "`is Bool.True` should be unreachable because `True` is already covered: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn lowercase_binding_matches_regardless_of_existing_value() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0 to 1000
Int has
    finish(self):
        return
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_unreachable_arm));
    assert!(has_flaw(&typed, is_unreachable_else));
}

#[test]
fn else_not_last_in_pattern_when_parses_safely() {
    // Arms after `else` are not parsed (parser breaks after else).
    // The `else` effectively becomes the last arm.
    let source = r#"
Bool is True or False
Int is in 0 to 1000
Int has
    finish(self):
        return
pick(b Bool) Int: return when b is
    True then 1
    else 0
    False then 2
"#;
    // Should produce a parse warning that `else` must be last.
    let typed = transform_source(source);
    assert!(
        has_parse_warning(&typed),
        "expected parse warning for `else` not last, got: {:?}",
        typed.parse_warnings
    );
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(!has_flaw(&typed, is_unreachable_else));
}
