//! Return parentheses affect source punctuation, not result typing or ownership.
use ast::{Block, Expr, Item, ReturnStatement, SequenceDelimiters, SourceFile, Statement};
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_file, check_module};

fn check(
    text: &str,
    dependencies: &[&CheckedSourceModule],
    limits: Limits,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid return-sequence source");
    let before = source.clone();
    let result = check_module("app.returns", &source, dependencies, limits);
    assert_eq!(source, before);
    result
}

fn library(spelling: &str) -> CheckedSourceModule {
    let text = format!(
        "Choice is A or B
choose(element Type, first element, second element, choice Choice) (element, element):
    result: when choice is
        A then
            {spelling}
        B then
            {spelling}
    return result
"
    );
    let source = parser::parse_strict(&text).unwrap();
    check_module("library.returns", &source, &[], Limits::default()).unwrap()
}

const CLIENT: &str = "
use library.returns Choice, choose
Scalar has number in 0 to 16
observe(first in 0 to 16, second in 0 to 16) in 0 to 16 and = first:
    pair: choose(Scalar, Scalar(number: first), Scalar(number: second), A)
    left: pair.0
    right: pair.[1]
    Scalar(left_number) := left
    Scalar(right_number) := right
    return left_number
";

#[test]
fn both_forms_infer_the_same_local_sequence_without_an_expected_type() {
    for spelling in ["return first, second", "return (first, second)"] {
        let library = library(spelling);
        check(CLIENT, &[&library], Limits::default()).unwrap();
    }
}

#[test]
fn both_forms_reject_the_same_wrong_origin_after_inference() {
    for spelling in ["return second, first", "return (second, first)"] {
        let library = library(spelling);
        assert_eq!(
            check(CLIENT, &[&library], Limits::default()).unwrap_err(),
            SourceCheckError::Unproved
        );
    }
}

fn minimum_client_work(library: &CheckedSourceModule) -> u32 {
    check(CLIENT, &[library], Limits::default()).unwrap();
    let mut lower = 0;
    let mut upper = Limits::default().work_max;
    for _ in 0..17 {
        if lower == upper {
            break;
        }
        let work_max = lower + (upper - lower) / 2;
        let limits = Limits {
            work_max,
            ..Limits::default()
        };
        match check(CLIENT, &[library], limits) {
            Ok(_) => upper = work_max,
            Err(SourceCheckError::Budget) => lower = work_max + 1,
            Err(other) => panic!("unexpected client result: {other:?}"),
        }
    }
    assert_eq!(lower, upper);
    upper
}

#[test]
fn imported_clients_do_not_pay_for_the_written_return_parentheses() {
    let bare = library("return first, second");
    let parenthesized = library("return (first, second)");
    assert_eq!(
        minimum_client_work(&bare),
        minimum_client_work(&parenthesized)
    );
}

#[test]
fn both_source_forms_have_one_sequence_expression_with_delimiter_metadata() {
    for (text, parenthesized) in [("f: return 1, 2\n", false), ("f: return (1, 2)\n", true)] {
        let source = parser::parse_strict(text).unwrap();
        let Item::Binding(binding) = &source.items[0].value else {
            panic!("binding");
        };
        let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
            panic!("body");
        };
        let Statement::Return(returned) = &block.statements[0].value else {
            panic!("return");
        };
        let Expr::Sequence(sequence) = &returned.value.as_ref().unwrap().value else {
            panic!("one sequence expression");
        };
        assert_eq!(sequence.elements.len(), 2);
        assert_eq!(sequence.separators.len(), 1);
        assert_eq!(
            matches!(
                sequence.delimiters,
                SequenceDelimiters::Parenthesized { .. }
            ),
            parenthesized
        );
    }
}

fn returned(source: &mut SourceFile) -> &mut ReturnStatement {
    let Item::Binding(binding) = &mut source.items[0].value else {
        panic!("binding");
    };
    let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
        panic!("body");
    };
    let Statement::Return(returned) = &mut block.statements[0].value else {
        panic!("return");
    };
    returned
}

#[test]
fn a_written_value_cannot_be_erased_or_replaced_with_an_empty_block() {
    for erase in [false, true] {
        let mut source = parser::parse_strict("f: return 1, 2\n").unwrap();
        let returned = returned(&mut source);
        if erase {
            returned.value = None;
        } else {
            returned.value.as_mut().unwrap().value = Expr::Block(Block {
                statements: Vec::new(),
            });
        }
        assert_eq!(
            check_file(&source, Limits::default()).unwrap_err(),
            SourceCheckError::Malformed
        );
    }
}
