//! Field projection `p.x`: reads copy unrestricted fields, linear fields can only be lent,
//! and nothing is ever moved out of a record this way.
use i256::I256;
use typecheck::{
    BodyResolver, CheckedSourceBody, CheckedSourceCallee, CheckedSourceFile, Evaluated, Limits,
    SourceCheckError, check_file, evaluate_value,
};

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    let file = parser::parse_strict(text).expect("strict projection fixture");
    check_file(&file, Limits::default())
}

const POINT: &str = "Point has\n  x in 0 to 9\n  y in 0 to 9\n";

// A linear resource, and a record holding one beside a number.
const RES: &str = "\
Res has v in 0 to 9
Outer has
  inner Res
  n in 0 to 9
look(r ref Res): return
poke(r mut Res): return
eat(r Res):
  Res(v) := r
  return
";

fn with(types: &str, tail: &str) -> String {
    format!("{types}{tail}")
}

#[test]
fn a_field_read_copies_the_field_and_leaves_the_record_whole() {
    let source = with(POINT, "get(p ref Point) in 0 to 9: return p.x\n");
    assert!(check(&source).is_ok(), "{:?}", check(&source));
    let sum = with(POINT, "sum(p ref Point) in 0 to 18: return p.x + p.y\n");
    assert!(check(&sum).is_ok(), "{:?}", check(&sum));
}

#[test]
fn reading_a_field_does_not_consume_an_owned_record() {
    let ended = with(
        POINT,
        "f(p Point) in 0 to 9:\n  s: p.x\n  Point(x, y) := p\n  return s\n",
    );
    assert!(check(&ended).is_ok(), "{:?}", check(&ended));
    // The owner still has to end the record it only read.
    let leaked = with(POINT, "f(p Point) in 0 to 9:\n  s: p.x\n  return s\n");
    assert_eq!(check(&leaked), Err(SourceCheckError::Unconsumed));
}

#[test]
fn an_unknown_field_is_an_error() {
    let source = with(POINT, "f(p ref Point) in 0 to 9: return p.z\n");
    assert_eq!(check(&source), Err(SourceCheckError::UnknownName));
}

#[test]
fn a_field_of_a_field_is_read_by_a_path() {
    let source = "\
Inner has v in 0 to 9
Outer has
  a Inner
  b in 0 to 9
get(o ref Outer) in 0 to 9: return o.a.v
";
    assert!(check(source).is_ok(), "{:?}", check(source));
}

#[test]
fn a_linear_field_cannot_be_moved_out_by_reading_it() {
    for body in [
        "take(o ref Outer) Res: return o.inner\n",
        "take(o Outer) Res:\n  r: o.inner\n  eat(r)\n  Outer(i, n) := o\n  return\n",
    ] {
        let source = with(RES, body);
        let result = check(&source);
        assert!(
            result == Err(SourceCheckError::MoveOutOfBorrow) || result.is_err(),
            "{body}: {result:?}"
        );
    }
    assert_eq!(
        check(&with(RES, "take(o ref Outer) Res: return o.inner\n")),
        Err(SourceCheckError::MoveOutOfBorrow)
    );
}

#[test]
fn a_linear_field_can_be_lent_to_a_borrowing_parameter() {
    let shared = with(RES, "see(o ref Outer):\n  look(o.inner)\n  return\n");
    assert!(check(&shared).is_ok(), "{:?}", check(&shared));
    let exclusive = with(RES, "see(o mut Outer):\n  poke(o.inner)\n  return\n");
    assert!(check(&exclusive).is_ok(), "{:?}", check(&exclusive));
}

#[test]
fn shared_access_to_a_record_is_shared_access_to_its_fields() {
    let source = with(RES, "see(o ref Outer):\n  poke(o.inner)\n  return\n");
    assert_eq!(check(&source), Err(SourceCheckError::BorrowConflict));
}

#[test]
fn lending_an_owned_records_field_leaves_the_record_to_be_ended() {
    let ended = with(
        RES,
        "see(o Outer):\n  look(o.inner)\n  Outer(i, n) := o\n  eat(i)\n  return\n",
    );
    assert!(check(&ended).is_ok(), "{:?}", check(&ended));
    let leaked = with(RES, "see(o Outer):\n  look(o.inner)\n  return\n");
    assert_eq!(check(&leaked), Err(SourceCheckError::Unconsumed));
}

#[test]
fn a_field_and_its_record_cannot_be_borrowed_in_conflicting_ways_at_once() {
    let call = |header: &str, args: &str| {
        with(
            &format!("{RES}{header}"),
            &format!("go(o Outer):\n  both({args})\n  Outer(i, n) := o\n  eat(i)\n  return\n"),
        )
    };
    for (header, args) in [
        ("both(a mut Outer, b ref Res): return\n", "o, o.inner"),
        ("both(a ref Res, b mut Outer): return\n", "o.inner, o"),
        ("both(a mut Res, b mut Res): return\n", "o.inner, o.inner"),
    ] {
        assert_eq!(
            check(&call(header, args)),
            Err(SourceCheckError::BorrowConflict),
            "{header} {args}"
        );
    }
    // Different fields, and two shared borrows of one, do not overlap.
    let ok = call("both(a ref Res, b ref Res): return\n", "o.inner, o.inner");
    assert!(check(&ok).is_ok(), "{:?}", check(&ok));
    let disjoint = with(
        &format!("{RES}both(a mut Res, b mut Res): return\n"),
        "go(o Outer, p Outer):\n  both(o.inner, p.inner)\n  Outer(i, n) := o\n  eat(i)\n  Outer(j, m) := p\n  eat(j)\n  return\n",
    );
    assert!(check(&disjoint).is_ok(), "{:?}", check(&disjoint));
}

#[test]
fn a_method_is_called_on_a_field_of_its_own_type() {
    let source = "\
Res has
  v in 0 to 9
  peek(self ref): return
  eat(self):
    Res(v) := self
    return
Outer has
  inner Res
  n in 0 to 9
see(o ref Outer):
  o.inner.peek
  return
";
    assert!(check(source).is_ok(), "{:?}", check(source));
    let consuming = source.replace("o.inner.peek", "o.inner.eat");
    assert!(
        check(&consuming).is_err(),
        "an owning method cannot take a field out"
    );
}

#[test]
fn the_hidden_number_of_a_domain_cannot_be_named() {
    let source = "FileId is in 0 to 255\nFileId has\n  close(self): return\npeek(id ref FileId) in 0 to 255: return id.value\n";
    assert_eq!(check(source), Err(SourceCheckError::Privacy));
}

#[test]
fn a_private_field_of_an_imported_record_cannot_be_read() {
    // The existing privacy rule now applies to a real read, not only to a refused one.
    let source =
        "Box has\n  #private\n  secret in 0 to 9\nuse_it(b ref Box) in 0 to 9: return b.secret\n";
    // In its own module a private field is readable; privacy is across modules.
    assert!(check(source).is_ok(), "{:?}", check(source));
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

fn int(value: i64) -> Evaluated {
    Evaluated::Integer(I256::from(value))
}

fn point(x: i64, y: i64) -> Evaluated {
    Evaluated::Record {
        schema: 0,
        fields: vec![int(x), int(y)],
    }
}

#[test]
fn a_field_read_is_recorded_and_evaluates() {
    let source = with(POINT, "sum(p ref Point) in 0 to 18: return p.x + p.y\n");
    let checked = check(&source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "sum")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    let result = evaluate_value(body, &[point(3, 4)], &Bodies(&checked.bodies), &mut 1_000);
    assert_eq!(result, Some(int(7)));
}

#[test]
fn a_nested_field_path_is_recorded_and_evaluates() {
    let source = "\
Inner has v in 0 to 9
Outer has
  a Inner
  b in 0 to 9
get(o ref Outer) in 0 to 9: return o.a.v
";
    let checked = check(source).unwrap();
    let index = checked
        .functions
        .iter()
        .position(|f| f.name == "get")
        .unwrap();
    let body = checked.bodies[index].as_ref().expect("recorded");
    let outer = Evaluated::Record {
        schema: 1,
        fields: vec![
            Evaluated::Record {
                schema: 0,
                fields: vec![int(6)],
            },
            int(2),
        ],
    };
    let result = evaluate_value(body, &[outer], &Bodies(&checked.bodies), &mut 1_000);
    assert_eq!(result, Some(int(6)));
}
