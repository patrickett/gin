//! Folder modules: several source files checked as one module with exact file attribution.
use ast::SourceFile;
use typecheck::{Limits, SourceCheckError, check_module_files, check_module_files_diagnostic};

const CALLEE: &str = "five() in 0 to 16: return 5\n";
const CALLER: &str = "again() in 0 to 16: return five()\n";

fn files(texts: &[&str]) -> Vec<SourceFile> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            parser::parse_strict_file(text, index as u32).expect("strict multi-file fixture")
        })
        .collect()
}

#[test]
fn files_share_one_declaration_namespace_in_any_order() {
    for order in [[CALLEE, CALLER], [CALLER, CALLEE]] {
        let module = check_module_files("app", &files(&order), &[], Limits::default())
            .expect("a call and its callee may live in different files");
        let names: Vec<_> = module
            .checked()
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect();
        assert_eq!(names.len(), 2, "{order:?}");
        assert!(
            names.contains(&"five") && names.contains(&"again"),
            "{names:?}"
        );
    }
}

#[test]
fn bodies_stay_aligned_with_functions_across_files() {
    let module = check_module_files("app", &files(&[CALLEE, CALLER]), &[], Limits::default())
        .expect("two-file module checks");
    let checked = module.checked();
    assert_eq!(checked.functions.len(), checked.bodies.len());
}

#[test]
fn duplicate_names_across_files_are_rejected_and_attributed_to_the_later_file() {
    let failure = check_module_files_diagnostic(
        "app",
        &files(&[CALLEE, "five() in 0 to 16: return 6\n"]),
        &[],
        Limits::default(),
    )
    .expect_err("one namespace cannot declare five twice");
    assert_eq!(failure.error, SourceCheckError::Duplicate);
    assert_eq!(failure.file, Some(1));
    assert!(failure.span.is_some());
}

#[test]
fn unknown_names_are_attributed_to_the_file_that_wrote_them() {
    let failure = check_module_files_diagnostic(
        "app",
        &files(&[CALLEE, CALLER, "broken() in 0 to 16: return missing()\n"]),
        &[],
        Limits::default(),
    )
    .expect_err("missing is declared nowhere");
    assert_eq!(failure.file, Some(2));
}

#[test]
fn a_single_file_module_matches_the_one_file_entry_point() {
    let single =
        parser::parse_strict("five() in 0 to 16: return 5\n").expect("strict fixture");
    let one = typecheck::check_module("app", &single, &[], Limits::default())
        .expect("one-file module");
    let many = check_module_files("app", &[single], &[], Limits::default())
        .expect("same module through the multi-file entry");
    assert_eq!(one.checked(), many.checked());
}

#[test]
fn an_empty_file_list_is_malformed() {
    assert_eq!(
        check_module_files("app", &[], &[], Limits::default()).unwrap_err(),
        SourceCheckError::Malformed
    );
}

#[test]
fn a_file_parsed_for_another_slot_is_rejected_rather_than_misread() {
    // Both files carry file-0 ids, so the second would resolve against the first.
    let mixed = vec![
        parser::parse_strict(CALLEE).unwrap(),
        parser::parse_strict(CALLER).unwrap(),
    ];
    assert_eq!(
        check_module_files("app", &mixed, &[], Limits::default()).unwrap_err(),
        SourceCheckError::Malformed
    );
}
