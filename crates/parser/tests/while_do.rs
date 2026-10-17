use ast::{BindValue, Condition, Expr, Loop};
use diagnostic::Category;
use parser::query::SourceParseExt;

#[test]
fn do_delimits_while_conditions_and_preserves_following_statements() {
    for condition in [
        "ready",
        "index < limit",
        "value is Some(item)",
        "index in 0 to 9",
        "index < limit and ready or not done",
        "not (index < limit or done)",
    ] {
        let source = format!(
            "run:\n    while {condition} do\n        observe\n    after\n    return 0\nnext: 1\n"
        );
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{source}: {:?}",
            parsed.symptoms
        );
        let BindValue::Body { exprs, .. } =
            &parsed.ast.defs[&internment::Intern::from_ref("run")].value
        else {
            panic!("function body");
        };
        assert_eq!(exprs.len(), 2);
        let Expr::Loop(Loop::While(loop_)) = &exprs[0].value else {
            panic!("while loop");
        };
        assert_eq!(loop_.exprs.len(), 1);
        if condition == "index < limit and ready or not done" {
            assert!(
                matches!(&loop_.condition, Condition::Or(left, _) if matches!(**left, Condition::And(..)))
            );
        }
        assert_eq!(parsed.ast.defs.len(), 2);
    }
}

#[test]
fn while_requires_do_before_an_indented_body() {
    for delimiter in ["", "then", "do do", "do observe"] {
        let source = format!("run: while ready {delimiter}\n    observe\n");
        let parsed = source.parse_source_full();
        assert!(
            parsed
                .symptoms
                .iter()
                .any(|diagnostic| diagnostic.category == Category::Flaw),
            "accepted {source}"
        );
        if delimiter.is_empty() || delimiter == "then" {
            assert!(
                parsed.symptoms.iter().any(|diagnostic| {
                    diagnostic.code.slug() == "parse-expected-token"
                        && diagnostic.message.contains("expected Do")
                }),
                "{source}: {:?}",
                parsed.symptoms
            );
        }
    }
}

#[test]
fn while_header_ending_in_do_is_incomplete() {
    assert!("run: while ready do".parse_source_full().is_incomplete());
    assert!("run: while ready do\n".parse_source_full().is_incomplete());
    assert!(
        !"run: while ready do\n    observe\n"
            .parse_source_full()
            .is_incomplete()
    );
}
