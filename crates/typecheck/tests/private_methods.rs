//! `#private` on a method: callable only inside the module that declares its type.
//!
//! A private method is not exported with the type, so an importer cannot resolve it; the module
//! itself calls it as any method.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module(name, &source, dependencies, Limits::default())
}

const LIBRARY: &str = "\
#unrestricted
Counter has
    value in 0 to 9

    #private
    bumped(self) in 0 to 9: return 5

    public(self) in 0 to 9: return self.bumped
";

fn library() -> CheckedSourceModule {
    check("case.counters", LIBRARY, &[]).expect("the library checks")
}

#[test]
fn a_private_method_is_callable_inside_its_module() {
    let checked = check(
        "case.inside",
        &format!("{LIBRARY}\nuse_it(c Counter) in 0 to 9: return c.bumped\n"),
        &[],
    );
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_public_method_is_callable_from_an_importer() {
    let client = "use case.counters Counter\nread(c Counter) in 0 to 9: return c.public\n";
    let checked = check("case.client", client, &[&library()]);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

#[test]
fn a_private_method_is_not_callable_from_an_importer() {
    for call in ["c.bumped", "c.bumped()", "Counter.bumped(c)"] {
        let client = format!("use case.counters Counter\nread(c Counter) in 0 to 9: return {call}\n");
        assert!(
            check("case.client", &client, &[&library()]).is_err(),
            "{call}"
        );
    }
}

/// The private method is hidden from the signature list an importer sees, too.
#[test]
fn a_private_method_is_marked_in_the_published_signature() {
    let module = library();
    let functions = &module.checked().functions;
    let find = |name: &str| functions.iter().find(|f| f.name == name).expect(name);
    assert!(find("Counter.bumped").private);
    assert!(!find("Counter.public").private);
}

#[test]
fn private_may_sit_with_infix() {
    let source = "\
Numeric has
    #private
    #infix('+')
    add(self, rhs Self): return self
";
    assert!(parser::parse_strict(source).is_ok());
}

#[test]
fn register_still_marks_a_field_only() {
    let source = "One has\n    #register('x0')\n    pick(self): return self\n";
    assert!(parser::parse_strict(source).is_err());
}
