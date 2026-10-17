use parser::parse_from_str;
use typecheck::FileId;
use typecheck::transform::transform_file;

fn flaws(branch: &str) -> Vec<diagnostic::Diagnostic> {
    let source = format!(
        "{}Word is in 0 to 65535\nWord has\n    finish(self):\n        return\nBool is True or False\npair() first Byte, second Byte: return 1, 2\ntriple() first Byte, second Byte, third Byte: return 1, 2, 3\nwide() first Byte, second Word: return 1, 2\nvalue(flag Bool) first Byte, second Byte:\n    when flag is True then\n        return {branch}()\n    return pair()\n",
        test_fixtures::preludes::BYTE_PRELUDE
    );
    let parsed = parse_from_str(&source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    transform_file(parsed, FileId(0))
        .all_flaws()
        .into_iter()
        .map(|(_, diagnostic)| diagnostic.clone())
        .collect()
}

#[test]
fn conditional_return_forwards_a_complete_result_row() {
    assert!(flaws("pair").is_empty());
}

#[test]
fn conditional_return_reports_the_forwarded_row_arity() {
    let flaws = flaws("triple");
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-return-arity"
                && flaw.message == "returned 3 values, but the callable declares 2 results"),
        "{flaws:?}"
    );
}

#[test]
fn conditional_return_checks_the_forwarded_field_types() {
    let flaws = flaws("wide");
    assert!(
        flaws
            .iter()
            .any(|flaw| flaw.code.slug() == "type-return-mismatch"),
        "{flaws:?}"
    );
}
