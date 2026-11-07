//! Parameter conventions: an unmarked parameter owns its argument, `end` owns it and must
//! also end it, and `ref`/`mut` borrow it without ever being able to move, end or return it.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict borrowing fixture");
    check_file(&file, Limits::default())
}

const P: &str = "P has x in 0 to 9\n";
const U: &str = "#unrestricted\nP has x in 0 to 9\n";

fn with(types: &str, tail: &str) -> String {
    format!("{types}{tail}")
}

#[test]
fn an_unmarked_parameter_is_an_owner_that_must_be_ended_or_passed_on() {
    assert!(check(&with(P, "forward(p P) P: return p\n")).is_ok());
    assert!(check(&with(P, "eat(p P):\n  P(x) := p\n  return\n")).is_ok());
    assert_eq!(
        check(&with(P, "leak(p P): return\n")),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn a_borrowed_parameter_leaves_responsibility_with_its_owner() {
    for word in ["ref", "mut"] {
        let source = with(P, &format!("look(p {word} P): return\n"));
        assert!(check(&source).is_ok(), "{word}: {:?}", check(&source));
    }
}

#[test]
fn a_borrow_cannot_be_moved_ended_or_returned() {
    for word in ["ref", "mut"] {
        for body in [
            "give(p WORD P) P: return p\n",
            "alias(p WORD P):\n  q: p\n  return\n",
            "eat(p WORD P):\n  P(x) := p\n  return\n",
        ] {
            let source = with(P, &body.replace("WORD", word));
            assert_eq!(
                check(&source),
                Err(SourceCheckError::MoveOutOfBorrow),
                "{word}: {body}"
            );
        }
    }
}

#[test]
fn reading_an_unrestricted_borrow_copies_it() {
    let source = with(
        U,
        "copy(p ref P) P: return p\ntwice(p mut P):\n  a: p\n  b: p\n  return\n",
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn an_end_parameter_must_be_ended_here_and_cannot_come_back_out() {
    assert!(check(&with(P, "finish(p end P):\n  P(x) := p\n  return\n")).is_ok());
    assert_eq!(
        check(&with(P, "forward(p end P) P: return p\n")),
        Err(SourceCheckError::EndEscape)
    );
    assert_eq!(
        check(&with(P, "alias(p end P) P:\n  q: p\n  return q\n")),
        Err(SourceCheckError::EndEscape)
    );
    assert_eq!(
        check(&with(P, "leak(p end P): return\n")),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn only_ref_mut_and_end_are_parameter_conventions() {
    for header in ["f(p own P): return", "f(e ref Type): return"] {
        let source = with(P, &format!("{header}\n"));
        assert_eq!(
            check(&source),
            Err(SourceCheckError::Unsupported),
            "{header}"
        );
    }
}

#[test]
fn a_borrowing_receiver_does_not_make_a_method_terminal() {
    let borrowing = "FileId is in 0 to 255\nFileId has\n  peek(self ref): return\n";
    assert_eq!(check(borrowing), Err(SourceCheckError::MissingTerminal));
    let both =
        "FileId is in 0 to 255\nFileId has\n  peek(self ref): return\n  close(self): return\n";
    assert!(check(both).is_ok(), "{:?}", check(both));
}

const LEND: &str = "\
P has x in 0 to 9
look(p ref P): return
poke(p mut P): return
eat(p P):
  P(x) := p
  return
";

fn go(body: &str) -> String {
    format!("{LEND}go():\n  p: P(x: 3)\n{body}")
}

#[test]
fn lending_a_value_leaves_it_owned_and_available() {
    let once = go("  look(p)\n  eat(p)\n  return\n");
    assert!(check(&once).is_ok(), "{:?}", check(&once));
    let repeatedly = go("  look(p)\n  poke(p)\n  look(p)\n  eat(p)\n  return\n");
    assert!(check(&repeatedly).is_ok(), "{:?}", check(&repeatedly));
}

#[test]
fn the_owner_still_has_to_end_a_value_it_only_lent() {
    assert_eq!(
        check(&go("  look(p)\n  return\n")),
        Err(SourceCheckError::Unconsumed)
    );
}

#[test]
fn a_moved_value_cannot_be_lent_and_a_lent_value_can_still_be_moved() {
    let after_move = go("  eat(p)\n  look(p)\n  return\n");
    assert_eq!(check(&after_move), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn borrows_within_one_call_must_not_overlap() {
    let source = |header: &str, call: &str| {
        format!(
            "P has x in 0 to 9\n{header}go():\n  p: P(x: 3)\n  {call}\n  eat(p)\n  return\neat(p P):\n  P(x) := p\n  return\n"
        )
    };
    let shared = source("two(a ref P, b ref P): return\n", "two(p, p)");
    assert!(check(&shared).is_ok(), "{:?}", check(&shared));
    for (header, call) in [
        ("two(a mut P, b mut P): return\n", "two(p, p)"),
        ("two(a mut P, b ref P): return\n", "two(p, p)"),
        ("two(a ref P, b mut P): return\n", "two(p, p)"),
        ("two(a ref P, b P):\n  P(x) := b\n  return\n", "two(p, p)"),
    ] {
        assert_eq!(
            check(&source(header, call)),
            Err(SourceCheckError::BorrowConflict),
            "{header}"
        );
    }
}

#[test]
fn shared_access_cannot_be_lent_out_exclusively() {
    let lend = |outer: &str, inner: &str| {
        format!(
            "P has x in 0 to 9\npoke(p {inner} P): return\nlook(p {outer} P):\n  poke(p)\n  return\n"
        )
    };
    assert_eq!(
        check(&lend("ref", "mut")),
        Err(SourceCheckError::BorrowConflict)
    );
    for (outer, inner) in [("ref", "ref"), ("mut", "ref"), ("mut", "mut")] {
        let source = lend(outer, inner);
        assert!(
            check(&source).is_ok(),
            "{outer}->{inner}: {:?}",
            check(&source)
        );
    }
}

#[test]
fn a_borrowed_parameter_can_be_passed_to_another_borrow_but_not_an_owner() {
    let to_owner =
        "P has x in 0 to 9\neat(p P):\n  P(x) := p\n  return\nlook(p ref P):\n  eat(p)\n  return\n";
    assert_eq!(check(to_owner), Err(SourceCheckError::MoveOutOfBorrow));
}

const SELF: &str = "\
P has
  x in 0 to 9
  peek(self ref): return
  poke(self mut): return
  eat(self):
    P(x) := self
    return
";

#[test]
fn a_ref_receiver_borrows_the_value_a_method_is_called_on() {
    let source =
        format!("{SELF}go():\n  p: P(x: 1)\n  p.peek\n  p.poke\n  p.peek()\n  p.eat\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let leaked = format!("{SELF}go():\n  p: P(x: 1)\n  p.peek\n  return\n");
    assert_eq!(check(&leaked), Err(SourceCheckError::Unconsumed));
    let after = format!("{SELF}go():\n  p: P(x: 1)\n  p.eat\n  p.peek\n  return\n");
    assert_eq!(check(&after), Err(SourceCheckError::UseAfterMove));
}

#[test]
fn a_borrowing_method_cannot_consume_its_receiver() {
    let source = "P has\n  x in 0 to 9\n  bad(self ref):\n    P(x) := self\n    return\n";
    assert_eq!(check(source), Err(SourceCheckError::MoveOutOfBorrow));
}

#[test]
fn a_linear_temporary_lent_to_a_borrow_is_never_ended() {
    let source = format!("{LEND}make() P: return P(x: 1)\ngo():\n  look(make())\n  return\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unconsumed));
}

#[test]
fn an_unrestricted_value_lent_stays_copyable() {
    let source =
        format!("{U}look(p ref P): return\ngo():\n  p: P(x: 1)\n  look(p)\n  look(p)\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn a_linear_domain_can_be_lent_and_then_closed() {
    let source = "\
FileId is in 0 to 255
FileId has
  close(self): return
  peek(self ref): return
go():
  id FileId: 7
  id.peek
  id.close
  return
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

const MONEY: &str = "\
Money is in 0 to 9
Money has
  close(self): return
  to_int(self ref) in 0 to 9: self
  #infix('+')
  add(self ref, rhs ref Self) in 0 to 18:
    a in Self: self
    b in Self: rhs
    return a + b
";

fn money(tail: &str) -> String {
    format!("{MONEY}{tail}")
}

#[test]
fn a_borrowing_operator_lends_its_operands_instead_of_consuming_them() {
    let source = money(
        "total(a Money, b Money) in 0 to 18:\n  s: a + b\n  a.close\n  b.close\n  return s\n",
    );
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn the_operands_of_a_borrowing_operator_still_have_to_be_ended() {
    let source = money("total(a Money, b Money) in 0 to 18:\n  s: a + b\n  a.close\n  return s\n");
    assert_eq!(check(&source), Err(SourceCheckError::Unconsumed));
}

#[test]
fn one_value_may_be_lent_to_both_sides_of_a_shared_operator() {
    let source = money("twice(a Money) in 0 to 18:\n  s: a + a\n  a.close\n  return s\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
}

#[test]
fn an_operator_taking_one_operand_by_value_and_lending_the_other_cannot_use_one_value_twice() {
    let source = MONEY.replace("add(self ref, rhs ref Self)", "add(self, rhs ref Self)");
    let source = source.replace("  to_int(self ref) in 0 to 9: self\n", "");
    // `self` is owned here, so reading it as an integer ends it; the other is lent.
    let mixed =
        format!("{source}total(a Money, b Money) in 0 to 18:\n  s: a + b\n  b.close\n  return s\n");
    assert!(check(&mixed).is_ok(), "{:?}", check(&mixed));
    let aliased = format!("{source}total(a Money) in 0 to 18: return a + a\n");
    assert_eq!(check(&aliased), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn exclusive_access_cannot_overlap_the_other_operand() {
    let source = MONEY.replace("rhs ref Self", "rhs mut Self");
    let used = format!("{source}total(a Money) in 0 to 18:\n  s: a + a\n  a.close\n  return s\n");
    assert_eq!(check(&used), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn a_computed_linear_operand_cannot_be_lent() {
    let source = money(
        "make() Money: return 3\ntotal(a Money) in 0 to 18:\n  s: a + make()\n  a.close\n  return s\n",
    );
    assert_eq!(check(&source), Err(SourceCheckError::Unsupported));
}

#[test]
fn a_borrowed_domain_receiver_reads_its_integer_but_cannot_be_moved() {
    let source = money("go():\n  id Money: 3\n  n: id.to_int\n  id.close\n  return\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let leak = money("");
    let leak = leak.replace(
        "to_int(self ref) in 0 to 9: self",
        "leak(self ref) Self: self",
    );
    assert_eq!(check(&leak), Err(SourceCheckError::MoveOutOfBorrow));
}

struct Bodies<'a>(&'a [Option<CheckedSourceBody>]);

impl BodyResolver for Bodies<'_> {
    fn body(&self, callee: CheckedSourceCallee) -> Option<&CheckedSourceBody> {
        match callee {
            CheckedSourceCallee::Local { function } => self.0[function as usize].as_ref(),
            CheckedSourceCallee::Dependency { .. } => None,
        }
    }
}

fn domain(value: i64) -> Evaluated {
    Evaluated::Record {
        schema: 0,
        fields: vec![Evaluated::Integer(I256::from(value))],
    }
}

#[test]
fn a_borrowing_operator_records_and_evaluates() {
    let source = money("total(a ref Money, b ref Money) in 0 to 18: return a + b\n");
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "total")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    let result = evaluate_value(
        body,
        &[domain(4), domain(5)],
        &Bodies(&checked.bodies),
        &mut 1_000,
    );
    assert_eq!(result, Some(Evaluated::Integer(I256::from(9))));
}
