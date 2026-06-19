//! `when` exhaustiveness: optional `else` when all cases are covered.

mod support;

use std::collections::HashMap;
use std::path::PathBuf;

use diagnostic::Diagnostic;
use parser::query::SourceParseExt;
use resolve::{GinPackageExt, ParsedFile, resolve_imports};
use support::transform_source;
use test_fixtures::TempPackage;
use typecheck::FileId;
use typecheck::transform::{PackageTransformOptions, TransformCtx, transform_package};

fn has_flaw(typed: &typecheck::TypedFileAst, pred: fn(&Diagnostic) -> bool) -> bool {
    typed.all_flaws().iter().any(|(_, f)| pred(f))
}

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

fn is_mixed_when_forms(f: &Diagnostic) -> bool {
    f.code.slug() == "type-mixed-when-forms"
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
    let ctx = TransformCtx::with_package_compile_time(Vec::new(), compile_time_eval_ast);
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
Int is in 0...1000
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: when m is Some(x) then x
                              is None then 0
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
Int is in 0...1000
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: when m is Some(x) then x
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
Int is in 0...1000
Maybe(x) is Some(x) or None
pick(m Maybe(Int)) Int: when m is Some(x) then x
                              is None then 0
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
fn cond_when_still_requires_else() {
    let source = "Bool is True or False\nInt is in 0...1000\nlt(x Int, y Int) Bool: True\nfoo(x Int) Int: when lt(x, 10) then x else 0";
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));

    let source = "Bool is True or False\nInt is in 0...1000\nlt(x Int, y Int) Bool: True\nfoo(x Int) Int: when lt(x, 10) then x";
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn bool_union_exhaustive_without_else() {
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         is False then 0
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
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn lowercase_bool_value_alias_patterns_are_exhaustive() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0...1000
pick(b Bool) Int: when b is true then 1
                         is false then 2
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "true/false value patterns should cover Bool: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn lowercase_bool_value_alias_pattern_with_else_is_valid() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0...1000
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(!has_flaw(&typed, is_unreachable_else));
}

#[test]
fn known_subject_value_selects_matching_else_branch_when_is_arm_does_not_match() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0...1000
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    // The when is folded to just the else body at compile time.
    // No MissingElseArm because the folded body is valid.
    assert!(!has_flaw(&typed, is_missing_else));
    assert!(!has_flaw(&typed, is_unreachable_else));
}

#[test]
fn else_must_be_last_in_boolean_when() {
    let source = "Bool is True or False\nInt is in 0...1000\nx(_ Bool) Bool: True\npick(b Bool) Int: when x(b) then 1\n                         else 0\n                         x(b) then 2\n";
    let typed = transform_source(source);
    // The boolean form parser supports arms after else (via continue).
    // The second `x(b) then 2` arm should be flagged unreachable.
    // (Exact check: at least one unreachable arm flaw should exist.)
    assert!(
        has_flaw(&typed, is_unreachable_arm),
        "arm after else should be unreachable: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn range_pattern_matches_compile_time() {
    let source = r#"
Int is in 0...1000
classify(n Int) String: when n is in 0...9 then 'small'
                               else 'large'
"#;
    let typed = transform_source(source);
    assert!(!has_flaw(&typed, is_missing_else));
}

#[test]
fn range_pattern_exhaustive_for_literal_union() {
    let source = r#"
Digit is 0 or 1 or 2 or 3
Int is in 0...1000
parse(d Digit) Int: when d is in 0...2 then 1
                          is 3 then 0
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "range 0...2 plus literal 3 should cover Digit: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn list_fixed_length_plus_cons_catch_all_exhaustive() {
    let source = r#"
Int is in 0...1000
List(x) is [] or [x, ..List(x)]
len(xs List(Int)) Int: when xs is [] then 0
                            is [h, ..tail] then 1
"#;
    let typed = transform_source(source);
    assert!(
        !has_flaw(&typed, is_missing_else),
        "[] + [h, ..tail] should be exhaustive: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn range_pattern_partial_requires_else() {
    let source = r#"
Digit is 0 or 1 or 2
Int is in 0...1000
parse(d Digit) Int: when d is in 0...1 then 1
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
Int is in 0...1000
pick(b Bool) Int: when b is x then 1
                         is y then 2
                         is true then 3
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
Int is in 0...1000
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
fn imported_lowercase_bool_value_pattern_with_else_is_valid() {
    let pkg = TempPackage::new("when_imported_bool_values");
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"."}}"#);
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive.(Bool, true, false)

Int is in 0...1000
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
        "imported `true` should resolve as a value pattern: {:?}",
        main_typed.all_flaws()
    );
    assert!(!has_flaw(main_typed, is_unreachable_else));
}

#[test]
fn imported_qualified_variant_pattern_is_detected_as_duplicate() {
    let pkg = TempPackage::new("when_imported_qualified_variant");
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"."}}"#);
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive.Bool

Int is in 0...1000
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
    pkg.write_flask_with_deps("app", r#"{"core":{"path":"."}}"#);
    let bool_path = pkg.write(
        "primitive/bool.gin",
        "Bool is True or False\nfalse Bool: False\ntrue Bool: True\n",
    );
    let main_src = r#"
use core.primitive.Bool

Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         is Bool.True then 2
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
Arch is 'x86_64' or 'arm64' or 'wasm32'
Int is in 0...1000
ptr(a Arch) Int: when a is 'x86_64' or 'arm64' then 8
                       is 'wasm32' then 4
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
Arch is 'x86_64' or 'arm64' or 'wasm32'
Int is in 0...1000
ptr(a Arch) Int: when a is 'x86_64' or 'arm64' then 8
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn wildcard_pattern_is_prohibited_use_else_instead() {
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is _ then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_wildcard_pattern));
}

#[test]
fn lowercase_pattern_is_not_a_catch_all() {
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is value then 1
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn list_empty_and_cons_catch_all_are_exhaustive_without_else() {
    let source = r#"
Int is in 0...1000
List(x) is [] or [x, ..List(x)]
len(xs List(Int)) Int: when xs is [] then 0
                            is [head, ..tail] then 1
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
Int is in 0...1000
List(x) is [] or [x, ..List(x)]
len(xs List(Int)) Int: when xs is [] then 0
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_missing_else));
}

#[test]
fn else_is_allowed_when_union_is_partial() {
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
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
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         is True then 2
                         is False then 0
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_unreachable_arm));
}

#[test]
fn condition_and_pattern_arms_cannot_be_mixed() {
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         b then 2
                         is False then 0
"#;
    let typed = transform_source(source);
    assert!(has_flaw(&typed, is_mixed_when_forms));
}

#[test]
fn bounded_int_fully_covered_by_ranges_no_else() {
    let source = r#"
Int is in 0...5
parse(n Int) Int: when n is in 0...2 then 1
                         is in 3...5 then 2
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
Int is in 0...5
parse(n Int) Int: when n is in 0...2 then 1
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
Int is in 0...1000
pick(b Bool) Int: when b is true then 1
                         is Bool.True then 2
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
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         is Bool.True then 2
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
fn known_subject_non_matching_value_pattern_is_unreachable() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0...1000
something() Int:
    b: false
    x: when b is true then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_arm),
        "`is true` should be unreachable when subject b is false: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn known_subject_matching_is_arm_makes_else_unreachable() {
    let source = r#"
Bool is True or False
false Bool: False
true Bool: True
Int is in 0...1000
something() Int:
    b: false
    x: when b is false then 1
                    else 2
    return x
"#;
    let typed = transform_source(source);
    assert!(
        has_flaw(&typed, is_unreachable_else),
        "else should be unreachable when `is false` matches subject b:false: {:?}",
        typed.all_flaws()
    );
}

#[test]
fn else_not_last_in_pattern_when_parses_safely() {
    // Arms after `else` are not parsed (parser breaks after else).
    // The `else` effectively becomes the last arm.
    let source = r#"
Bool is True or False
Int is in 0...1000
pick(b Bool) Int: when b is True then 1
                         else 0
                         is False then 2
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
