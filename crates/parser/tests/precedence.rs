use ast::{BinaryOperator, Expr, Expression, Item, SourceFile};

fn expression(file: &SourceFile) -> &Expression {
    assert_eq!(file.items.len(), 1);
    let Item::Expression(expression) = &file.items[0].value else {
        panic!("expected one expression");
    };
    expression
}

fn binary(expression: &Expression, expected: BinaryOperator) -> (&Expression, &Expression) {
    let Expr::Binary {
        left,
        operator,
        right,
    } = &expression.value
    else {
        panic!("expected {expected:?}, found {:?}", expression.value);
    };
    assert_eq!(operator.value, expected);
    (left, right)
}

fn name(expression: &Expression, expected: &str) {
    let Expr::Name(actual) = &expression.value else {
        panic!("expected name {expected}");
    };
    assert_eq!(actual, expected);
}

#[test]
fn adjacent_precedence_levels_bind_in_both_orders() {
    use BinaryOperator::*;
    let levels = [
        ("*", Multiply, "+", Add),
        ("+", Add, "<<", ShiftLeft),
        (">>", ShiftRight, "&", BitAnd),
        ("&", BitAnd, "^", BitXor),
        ("^", BitXor, "|", BitOr),
        ("|", BitOr, "=", Equal),
        ("<", Less, "and", And),
        ("and", And, "or", Or),
    ];
    for (strong_text, strong, weak_text, weak) in levels {
        let source = format!("a {weak_text} b {strong_text} c\n");
        let file = parser::parse_strict(&source).expect(&source);
        let (left, right) = binary(expression(&file), weak);
        name(left, "a");
        let (left, right) = binary(right, strong);
        name(left, "b");
        name(right, "c");

        let source = format!("a {strong_text} b {weak_text} c\n");
        let file = parser::parse_strict(&source).expect(&source);
        let (left, right) = binary(expression(&file), weak);
        name(right, "c");
        let (left, right) = binary(left, strong);
        name(left, "a");
        name(right, "b");
    }
}

#[test]
fn equal_precedence_arithmetic_bitwise_and_logic_associate_left() {
    use BinaryOperator::*;
    let cases = [
        ("a / b * c\n", Divide, Multiply),
        ("a * b / c\n", Multiply, Divide),
        ("a % b / c\n", Remainder, Divide),
        ("a - b - c\n", Subtract, Subtract),
        ("a << b >> c\n", ShiftLeft, ShiftRight),
        ("a & b & c\n", BitAnd, BitAnd),
        ("a ^ b ^ c\n", BitXor, BitXor),
        ("a | b | c\n", BitOr, BitOr),
        ("a and b and c\n", And, And),
        ("a or b or c\n", Or, Or),
    ];
    for (source, inner, outer) in cases {
        let file = parser::parse_strict(source).expect(source);
        let (left, right) = binary(expression(&file), outer);
        name(right, "c");
        let (left, right) = binary(left, inner);
        name(left, "a");
        name(right, "b");
    }
}

#[test]
fn grouping_overrides_precedence() {
    let file = parser::parse_strict("(a + b) * c\n").expect("explicit group");
    let (left, right) = binary(expression(&file), BinaryOperator::Multiply);
    name(right, "c");
    let Expr::Group(inner) = &left.value else {
        panic!("expected preserved group");
    };
    let (left, right) = binary(inner, BinaryOperator::Add);
    name(left, "a");
    name(right, "b");
}

#[test]
fn every_comparison_binds_below_bitwise_and_above_logic() {
    use BinaryOperator::*;
    for (spelling, operator) in [
        ("=", Equal),
        ("<", Less),
        ("<=", LessOrEqual),
        (">", Greater),
        (">=", GreaterOrEqual),
    ] {
        let source = format!("a | b {spelling} c | d and e\n");
        let file = parser::parse_strict(&source).expect(&source);
        let (left, right) = binary(expression(&file), And);
        name(right, "e");
        let (left, right) = binary(left, operator);
        let (a, b) = binary(left, BitOr);
        name(a, "a");
        name(b, "b");
        let (c, d) = binary(right, BitOr);
        name(c, "c");
        name(d, "d");
    }
}

#[test]
fn comparison_chains_require_explicit_grouping() {
    for source in ["a < b < c", "a = b > c", "a <= b >= c"] {
        let output = parser::parse(source).unwrap();
        assert!(output.has_errors());
        assert!(
            output
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.message.contains("comparison chains") })
        );
        assert!(parser::parse_strict(source).is_err());
    }
    assert!(parser::parse_strict("(a < b) = c").is_ok());
    assert!(parser::parse_strict("a < b and b < c").is_ok());
}
