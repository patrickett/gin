//! A compile-time query whose arguments are known is recorded as the number (or text) it is, so
//! every consumer of a body sees a constant; one that depends on a type parameter is not.
use typecheck::{CheckedSourceBodyKind as Kind, CheckedSourceFile, Limits, SourceCheckError, Target, check_file};

const POINT: &str = "#unrestricted\nPoint has\n    x in 0 to 9\n    y in 0 to 300\n\n#unrestricted\nSmall is in 3 to 200\n\n";

fn check(text: &str) -> Result<CheckedSourceFile, SourceCheckError> {
    check_file(&parser::parse_strict(text).expect("parses"), Limits::default())
}

fn root_integer(file: &CheckedSourceFile, name: &str) -> Option<i128> {
    let at = file.functions.iter().position(|f| f.name == name)?;
    let body = file.bodies[at].as_ref()?;
    let Kind::Block { result, .. } = &body.expressions[body.root as usize].kind else { return None };
    let Kind::Return { value } = body.expressions[*result as usize].kind else { return None };
    match &body.expressions[value as usize].kind {
        Kind::Integer(n) => Some(n.as_i128()),
        _ => None,
    }
}

#[test]
fn size_alignment_lower_and_upper_with_known_arguments_are_constants() {
    let file = check(&format!("{POINT}size() in 0 to 255: return #size(Point)\nalign() in 0 to 255: return #alignment(Point)\nlow() in 0 to 255: return #lower(Small)\nhigh() in 0 to 255: return #upper(Small)\nbytes() in 0 to 255: return #size(Small)\n")).unwrap();
    assert_eq!(root_integer(&file, "size"), Some(4));
    assert_eq!(root_integer(&file, "align"), Some(2));
    assert_eq!(root_integer(&file, "low"), Some(3));
    assert_eq!(root_integer(&file, "high"), Some(200));
    assert_eq!(root_integer(&file, "bytes"), Some(1));
}

#[test]
fn a_query_composes_with_arithmetic_and_folds() {
    let file = check(&format!("{POINT}twice() in 0 to 255: return #size(Point) * 2 + #alignment(Point)\n")).unwrap();
    assert_eq!(root_integer(&file, "twice"), Some(10));
}

#[test]
fn the_target_facts_are_recorded() {
    let limits = Limits { target: Target { triple: "test-triple", register_bits: 32 }, ..Limits::default() };
    let file = check_file(&parser::parse_strict("bits() in 0 to 255: return #target.register_bits\n").unwrap(), limits).unwrap();
    assert_eq!(root_integer(&file, "bits"), Some(32));
    // The triple is compile-time text.
    let file = check_file(&parser::parse_strict("name() in 0 to 255:\n    t: #target.triple\n    return 1\n").unwrap(), limits).unwrap();
    let body = file.bodies[0].as_ref().expect("recorded");
    assert!(body.expressions.iter().any(|e| matches!(&e.kind, Kind::Text(t) if t == "test-triple")));
}

#[test]
fn a_query_that_depends_on_a_type_parameter_is_recorded_as_a_layout_node() {
    // Not a number yet: a node naming the type, filled in when an instance is made.
    let file = check("width(element) in 0 to 4611686018427387904: return #size(element)\n").unwrap();
    let body = file.bodies[0].as_ref().expect("recorded");
    assert!(body.expressions.iter().any(|e| matches!(&e.kind, Kind::Layout { alignment: false, .. })), "{body:?}");
    // Control: a type with no parameter is still the number itself.
    let file = check(&format!("{POINT}size() in 0 to 255: return #size(Point)\n")).unwrap();
    assert_eq!(root_integer(&file, "size"), Some(4));
}

#[test]
fn a_query_the_checker_does_not_define_is_still_refused() {
    let text = format!("{POINT}Shape is Single or Pair\nSingle has value in 0 to 100\nPair has first in 0 to 100\nsize() in 0 to 255: return #size(Shape)\n");
    assert!(check(&text).is_err());
}
