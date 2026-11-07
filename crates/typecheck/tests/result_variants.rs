//! A result written as variants, `Reserved(Code) where (..) or Failed(Code) where (..)`, declares
//! those variants as the function's own, and the claims of the variant a path returns are the
//! ones proven. (A header no longer looks for a declared sum with those members: a bare name
//! in the list is a new variant, never an existing type.)
use typecheck::{Limits, SourceCheckError, check_file};

const TYPES: &str = "#unrestricted\nCode is in 0 to 9\n\n";

fn check(function: &str) -> Result<(), SourceCheckError> {
    let text = format!("{TYPES}{function}");
    let source = parser::parse_strict(&text).expect("strict source syntax");
    check_file(&source, Limits::default()).map(|_| ())
}

#[test]
fn the_returned_variants_claims_hold() {
    check(
        "ok(n in 0 to 9) Reserved(Code) where (n >= 0) or\n  Failed(Code) where (n <= 9):\n    return Reserved(n as Code)\n",
    )
    .unwrap();
}

#[test]
fn a_false_claim_of_the_returned_variant_is_rejected() {
    assert_eq!(
        check("bad(n in 0 to 9) Reserved(Code) where (n >= 1) or\n  Failed(Code) where (n <= 9):\n    return Reserved(n as Code)\n"),
        Err(SourceCheckError::Unproved)
    );
}

#[test]
fn only_the_claims_of_the_variant_returned_are_proven() {
    // The failure variant's claim cannot be proven for n = 0, but this path never returns it.
    check(
        "ok(n in 0 to 9) Reserved(Code) or\n  Failed(Code) where (n >= 5):\n    return Reserved(n as Code)\n",
    )
    .unwrap();
    assert!(check(
        "other(n in 0 to 9) Reserved(Code) or\n  Failed(Code) where (n >= 5):\n    return Failed(n as Code)\n",
    )
    .is_err());
}

#[test]
fn variants_without_claims_are_the_headers_own() {
    check("plain(n in 0 to 9) Reserved(Code) or Failed(Code):\n    return Reserved(n as Code)\n").unwrap();
    // Returning a name the header did not declare is refused.
    assert_eq!(
        check("none(n in 0 to 9) Reserved(Code) or Failed(Code):\n    return Missing\n"),
        Err(SourceCheckError::UnknownName)
    );
}

#[test]
fn claims_joined_by_and_are_all_proven() {
    check(
        "both(n in 2 to 9) Reserved(Code) where (n >= 1 and n <= 9) or Failed(Code):\n    return Reserved(n as Code)\n",
    )
    .unwrap();
    assert!(check(
        "one(n in 0 to 9) Reserved(Code) where (n >= 1 and n <= 9) or Failed(Code):\n    return Reserved(n as Code)\n",
    )
    .is_err());
}

mod generic_and_nested {
    use typecheck::{Limits, SourceCheckError, check_file};

    fn check(text: &str) -> Result<(), SourceCheckError> {
        let file = parser::parse_strict(text).expect("strict fixture");
        check_file(&file, Limits::default()).map(|_| ())
    }

    const TYPES: &str = "#unrestricted\nCode is in 0 to 9\n\n";

    #[test]
    fn variants_of_a_generic_result_carry_their_own_claims() {
        let ok = format!(
            "{TYPES}make(x Type, v x, n in 3 to 9) Made(x) where (n >= 3) or\n  Late:\n    return Made(v)\n"
        );
        assert_eq!(check(&ok), Ok(()));
        let wrong = format!(
            "{TYPES}make(x Type, v x, n in 0 to 9) Made(x) where (n >= 3) or\n  Late:\n    return Made(v)\n"
        );
        assert_eq!(check(&wrong), Err(SourceCheckError::Unproved));
    }

    #[test]
    fn a_variant_without_a_payload_takes_no_claim_of_another() {
        // Returning `Late` is the `Late` variant, which claims nothing, so the first variant's
        // claim is not asked of it.
        let plain = format!(
            "{TYPES}make(n in 0 to 9, late in 0 to 1) Made(Code) where (n >= 3) or\n  Late:\n    when late = 1 then\n        return Late\n    when n >= 3 then\n        return Made(n as Code)\n    return Late\n"
        );
        assert_eq!(check(&plain), Ok(()));
        // The first variant must still keep its claim.
        let wrong = format!(
            "{TYPES}make(n in 0 to 9, late in 0 to 1) Made(Code) where (n >= 3) or\n  Late:\n    when late = 1 then\n        return Late\n    return Made(n as Code)\n"
        );
        assert_eq!(check(&wrong), Err(SourceCheckError::Unproved));
    }
}
