//! `=` discriminants in a union's `or` list. The parser
//! reads `Name = literal` as an ordinary equality, so the tree keeps the written spelling; the
//! checker gives it the meaning "is encoded as" inside a union declaration and refuses it
//! anywhere it does not fit.

use ast::{BinaryOperator, Expr, Expression, Item, LiteralKind, TypeBody};

fn union_body(source: &str) -> ast::SourceFile {
    parser::parse_strict(source).unwrap_or_else(|d| panic!("{source:?}: {d:?}"))
}

fn variants(expression: &Expression, out: &mut Vec<(String, Option<LiteralKind>, bool)>, text: &str, file: &ast::SourceFile) {
    match &expression.value {
        Expr::Binary { left, operator, right } if operator.value == BinaryOperator::Or => {
            variants(left, out, text, file);
            variants(right, out, text, file);
        }
        Expr::Binary { left, operator, right } if operator.value == BinaryOperator::Equal => {
            let Expr::Name(name) = &left.value else { panic!("variant name: {left:?}") };
            let (kind, negative) = match &right.value {
                Expr::Literal(kind) => (*kind, false),
                Expr::Unary { operand, .. } => match &operand.value {
                    Expr::Literal(kind) => (*kind, true),
                    other => panic!("literal: {other:?}"),
                },
                other => panic!("literal: {other:?}"),
            };
            let _ = (text, file);
            out.push((name.clone(), Some(kind), negative));
        }
        Expr::Name(name) => out.push((name.clone(), None, false)),
        other => panic!("unexpected {other:?}"),
    }
}

fn written(source: &str) -> Vec<(String, Option<LiteralKind>, bool)> {
    let file = union_body(source);
    let Item::Type(declaration) = &file.items[0].value else { panic!("type") };
    let TypeBody::Is(root) = &declaration.body.value else { panic!("is") };
    let mut out = Vec::new();
    variants(root, &mut out, &file.text, &file);
    out
}

#[test]
fn integer_and_text_discriminants_on_one_line() {
    use LiteralKind::{Integer, String};
    assert_eq!(
        written("ExitCode is Success = 0 or FirstFlaw = 1 or SecFlaw = 2\n"),
        [("Success".into(), Some(Integer), false), ("FirstFlaw".into(), Some(Integer), false), ("SecFlaw".into(), Some(Integer), false)]
    );
    assert_eq!(
        written("LogLevel is Debug = 'debug' or Info = 'info'\n"),
        [("Debug".into(), Some(String), false), ("Info".into(), Some(String), false)]
    );
    assert_eq!(written("Signed is Low = -1 or High = 1\n")[0], ("Low".into(), Some(Integer), true));
}

#[test]
fn vertically_aligned_discriminants() {
    let aligned = "\
ExitCode is Success   = 0
         or FirstFlaw = 1
         or SecFlaw   = 2
";
    assert_eq!(written(aligned).len(), 3);
    let indented = "LogLevel is Debug = 'debug'\n    or Info = 'info'\n    or Warn = 'warn'\n";
    assert_eq!(written(indented).len(), 3);
    // Mixed with plain variants still parses; the checker refuses it (all or none).
    assert_eq!(written("Mixed is A = 1 or B\n").len(), 2);
}

#[test]
fn malformed_discriminants_are_parse_errors() {
    for source in [
        "E is A = or B = 1\n",
        "E is = 1 or B = 2\n",
        "E is A = 1 or\n",
        "E is A == 1 or B = 2\n",
    ] {
        assert!(parser::parse_strict(source).is_err(), "{source:?}");
    }
}
