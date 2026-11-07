//! Header `where` claims are proven at exit over parameters and the result.
use typecheck::{Limits, SourceCheckError, check_file};

fn check(text: &str) -> Result<(), SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn a_named_result_claim_is_proven_against_the_returned_value() {
    assert_eq!(
        check("f(v in 8 to 16) out in 0 to 16 where out >= 8, out <= 16:\n  return v\n"),
        Ok(())
    );
    assert_eq!(
        check("f(v in 0 to 16) out in 0 to 16 where out >= 8:\n  return v\n"),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn an_unnamed_result_compares_with_a_leading_comparison() {
    assert_eq!(
        check("f(v in 10 to 16) in 0 to 16 where >= 10:\n  return v\n"),
        Ok(())
    );
    assert_eq!(
        check("f(v in 0 to 16) in 0 to 16 where >= 10:\n  return v\n"),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn a_claim_on_a_mut_place_shares_the_where_clause_with_the_result() {
    assert_eq!(
        check("f(x mut in 0 to 9) out in 0 to 9 where x :: 0, out >= 0:\n  x :: 0\n  return 1\n"),
        Ok(())
    );
    assert!(
        check("f(x mut in 0 to 9) out in 0 to 9 where x :: 0, out >= 2:\n  x :: 0\n  return 1\n")
            .is_err()
    );
    assert!(
        check("f(x mut in 0 to 9) out in 0 to 9 where x :: 0, out >= 0:\n  x :: 1\n  return 1\n")
            .is_err()
    );
}

#[test]
fn a_claim_cannot_call_a_function() {
    // Observed limitation: calls inside claims are Unsupported.
    let text = "id(a in 0 to 1) in 0 to 1: return a\n\
                f(v in 0 to 1) out in 0 to 1 where out = id(v):\n  return v\n";
    assert_eq!(check(text), Err(SourceCheckError::Unsupported));
}

#[test]
fn a_claim_cannot_name_a_local_of_the_body() {
    assert!(
        check("f(v in 0 to 9) out in 0 to 9 where out = local:\n  local: v\n  return v\n").is_err()
    );
}
