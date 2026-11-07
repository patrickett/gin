//! `when condition then ...` as a statement: branches are blocks, `else` is optional, and a
//! branch may `return`. The value form `when c then a else b` stays an expression.
use ast::{Block, Expr, Item, SourceFile, Statement, WhenStatement};

fn body(file: &SourceFile) -> &Block {
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
        panic!("block body")
    };
    block
}

fn when(file: &SourceFile, index: usize) -> &WhenStatement {
    let Statement::When(when) = &body(file).statements[index].value else {
        panic!("when statement at {index}: {:?}", body(file).statements[index].value)
    };
    when
}

#[test]
fn a_guard_that_returns_on_one_line_is_a_statement() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then return 0\n    return n\n";
    let file = parser::parse_strict(source).unwrap();
    let guard = when(&file, 0);
    assert_eq!(guard.then_block.value.statements.len(), 1);
    assert!(matches!(
        guard.then_block.value.statements[0].value,
        Statement::Return(_)
    ));
    assert!(guard.else_branch.is_none());
    assert!(matches!(body(&file).statements[1].value, Statement::Return(_)));
}

#[test]
fn a_block_after_then_is_a_statement_without_else() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n        m: n + 1\n        return m\n    return n\n";
    let file = parser::parse_strict(source).unwrap();
    let guard = when(&file, 0);
    assert_eq!(guard.then_block.value.statements.len(), 2);
    assert!(guard.else_branch.is_none());
    assert_eq!(body(&file).statements.len(), 2);
}

#[test]
fn else_on_its_own_line_takes_a_block() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n        return 0\n    else\n        return 1\n";
    let file = parser::parse_strict(source).unwrap();
    let guard = when(&file, 0);
    let otherwise = guard.else_branch.as_ref().expect("an else branch");
    assert_eq!(otherwise.block.value.statements.len(), 1);
    assert_eq!(body(&file).statements.len(), 1);
}

#[test]
fn a_fall_through_block_may_reassign() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    m: n\n    when n < 2 then m :: 5\n    return m\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(
        when(&file, 1).then_block.value.statements[0].value,
        Statement::Assign(_)
    ));
}

#[test]
fn conditionals_nest_inside_blocks() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 5 then\n        when n < 2 then return 0\n        return 1\n    return 2\n";
    let file = parser::parse_strict(source).unwrap();
    let outer = when(&file, 0);
    assert!(matches!(
        outer.then_block.value.statements[0].value,
        Statement::When(_)
    ));
    assert_eq!(outer.then_block.value.statements.len(), 2);
    assert_eq!(body(&file).statements.len(), 2);
}

#[test]
fn the_value_form_is_still_an_expression() {
    let source = "f(n in 0 to 9) in 0 to 1:\n    return when n < 2 then 1 else 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Return(ret) = &body(&file).statements[0].value else {
        panic!("return")
    };
    assert!(matches!(ret.value.as_ref().unwrap().value, Expr::Conditional(_)));
}

#[test]
fn a_case_statement_is_still_a_case() {
    let source = "Mode is A or B\n\nf(m Mode) in 0 to 1: when m is\n    A then 0\n    B then 1\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(file.items.len(), 2));
}

#[test]
fn a_block_needs_a_statement() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then\n    return n\n";
    assert!(parser::parse_strict(source).is_err());
}

#[test]
fn a_call_after_then_is_a_statement_when_no_else_follows() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    when n < 2 then g(n)\n    return n\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(
        when(&file, 0).then_block.value.statements[0].value,
        Statement::Expression(_)
    ));
}

#[test]
fn a_bare_return_ends_a_one_line_guard() {
    let source = "f(n in 0 to 9):\n    when n < 2 then return\n    g(n)\n";
    let file = parser::parse_strict(source).unwrap();
    assert!(matches!(
        when(&file, 0).then_block.value.statements[0].value,
        Statement::Return(_)
    ));
}

fn loop_body(file: &SourceFile) -> &Block {
    let Statement::Expression(expression) = &body(file).statements[0].value else {
        panic!("loop")
    };
    let Expr::Loop(looped) = &expression.value else {
        panic!("loop expression")
    };
    let ast::LoopExpression::While { body, .. } = &**looped else {
        panic!("while")
    };
    let Expr::Block(block) = &body.value else {
        panic!("block body")
    };
    block
}

#[test]
fn a_loop_body_holds_when_statements() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    while n > 0 do\n        when n < 2 then\n            return 0\n        else\n            return 1\n        when n > 5 then g(n)\n    return n\n";
    let file = parser::parse_strict(source).unwrap();
    let statements = &loop_body(&file).statements;
    assert_eq!(statements.len(), 2);
    let Statement::When(first) = &statements[0].value else {
        panic!("when")
    };
    assert!(first.else_branch.is_some());
    assert!(matches!(statements[1].value, Statement::When(_)));
}

#[test]
fn when_statements_nest_inside_loop_blocks() {
    let source = "f(n in 0 to 9) in 0 to 9:\n    while n > 0 do\n        when n < 5 then\n            when n < 2 then return 0\n            g(n)\n    return n\n";
    let file = parser::parse_strict(source).unwrap();
    let statements = &loop_body(&file).statements;
    let Statement::When(outer) = &statements[0].value else {
        panic!("when")
    };
    assert_eq!(outer.then_block.value.statements.len(), 2);
    assert!(matches!(
        outer.then_block.value.statements[0].value,
        Statement::When(_)
    ));
}

fn when_is(file: &SourceFile, index: usize) -> &ast::WhenIsStatement {
    let Statement::WhenIs(when) = &body(file).statements[index].value else {
        panic!("one-armed case at {index}: {:?}", body(file).statements[index].value)
    };
    when
}

/// `when subject is Pattern then ...` with one pattern is a statement; error propagation.
#[test]
fn a_one_armed_case_with_a_returning_block_is_a_statement() {
    let source = "f(r Result) in 0 to 1:\n    when r is Failed(code) then return 1\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let case = when_is(&file, 0);
    assert!(matches!(case.subject.value, Expr::Name(_)));
    let ast::MatchPattern::Variant(pattern) = &case.pattern.value else {
        panic!("variant pattern")
    };
    assert_eq!(pattern.constructor.value, "Failed");
    assert!(matches!(
        case.then_block.value.statements[0].value,
        Statement::Return(_)
    ));
    assert!(case.else_branch.is_none());
}

#[test]
fn the_subject_may_be_a_call_and_the_block_may_be_indented() {
    let source = "f() in 0 to 1:\n    when step(3) is Failed(_) then\n        cleanup()\n        return 1\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let case = when_is(&file, 0);
    assert!(matches!(case.subject.value, Expr::Call { .. }));
    assert_eq!(case.then_block.value.statements.len(), 2);
}

#[test]
fn a_one_armed_case_may_sit_in_a_loop_body() {
    let source = "f() in 0 to 1:\n    while go() do\n        when step(3) is Failed(error) then return 1\n        count()\n    return 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Statement::Expression(expression) = &body(&file).statements[0].value else {
        panic!("loop")
    };
    let Expr::Loop(looped) = &expression.value else {
        panic!("loop expression")
    };
    let ast::LoopExpression::While { body: inner, .. } = &**looped else {
        panic!("while")
    };
    let Expr::Block(block) = &inner.value else {
        panic!("block")
    };
    assert!(matches!(block.statements[0].value, Statement::WhenIs(_)));
}

/// A case whose arms follow on their own lines is still a full case, an expression.
#[test]
fn a_full_case_with_arms_is_not_a_one_armed_case() {
    let source = "f(r Result) in 0 to 1: when r is\n    Failed(_) then 1\n    Done then 0\n";
    let file = parser::parse_strict(source).unwrap();
    let Item::Binding(binding) = &file.items[0].value else {
        panic!("function")
    };
    assert!(matches!(
        binding.value.as_ref().unwrap().expression.value,
        Expr::Match(_)
    ));
}
