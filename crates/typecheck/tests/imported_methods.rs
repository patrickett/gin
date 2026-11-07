//! A type's methods come with the type when a module imports it.
//!
//! Methods are the functions `Type.name`. Importing `Type` by name makes its methods callable
//! on its values, as `b.get`, `b.get()`, `Type.Variant.get` and, for a domain, as operators.
//! Importing a function does not import the methods of the types it mentions.
use typecheck::{
    CheckedSourceBodyKind, CheckedSourceModule, Limits, SourceCheckError, check_module,
};

fn check(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid syntax");
    check_module(name, &source, dependencies, Limits::default())
}

const BOX: &str = "\
#unrestricted
Box has
    value in 0 to 9
    get(self) in 0 to 9:
        Box(value) := self
        return value
";

fn library() -> CheckedSourceModule {
    check("case.boxes", BOX, &[]).unwrap()
}

#[test]
fn an_imported_record_brings_its_methods() {
    let library = library();
    for call in ["held.get", "held.get()"] {
        let client = format!(
            "use case.boxes Box\nanswer() in 0 to 9:\n    held := Box(value: 4)\n    return {call}\n"
        );
        let checked = check("case.client", &client, &[&library]);
        assert!(checked.is_ok(), "{call}: {:?}", checked.err());
    }
}

#[test]
fn a_method_is_called_on_a_parameter_of_the_imported_type() {
    let library = library();
    let client = "use case.boxes Box\nread(box Box) in 0 to 9: return box.get\n";
    assert!(check("case.client", client, &[&library]).is_ok());
}

#[test]
fn an_imported_method_folds_at_compile_time() {
    let library = library();
    let client =
        "use case.boxes Box\nanswer() in 0 to 9:\n    held := Box(value: 4)\n    return held.get\n";
    let module = check("case.client", client, &[&library]).unwrap();
    let file = module.checked();
    let index = file
        .functions
        .iter()
        .position(|f| f.name == "answer")
        .unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    assert!(
        body.expressions
            .iter()
            .all(|expr| !matches!(expr.kind, CheckedSourceBodyKind::Call { .. })),
        "{:?}",
        body.expressions
    );
    assert!(body.expressions.iter().any(
        |expr| matches!(&expr.kind, CheckedSourceBodyKind::Integer(value) if *value == 4.into())
    ));
}

#[test]
fn an_imported_union_brings_its_methods() {
    let library = check(
        "case.symbols",
        "#unrestricted\nSymbol is Putchar\n       or Labs\n\nSymbol has\n    code(self) in 0 to 1: when self is\n        Putchar then 0\n        Labs then 1\n",
        &[],
    )
    .unwrap();
    for receiver in ["Symbol.Labs.code", "Symbol.Labs.code()"] {
        let client = format!("use case.symbols Symbol\nanswer() in 0 to 1: return {receiver}\n");
        let checked = check("case.client", &client, &[&library]);
        assert!(checked.is_ok(), "{receiver}: {:?}", checked.err());
    }
    let client = "use case.symbols Symbol\nread(symbol Symbol) in 0 to 1: return symbol.code\n";
    assert!(check("case.client", client, &[&library]).is_ok());
}

/// Compile-time text from a method of an imported union reaches an instruction operand.
#[test]
fn an_imported_to_string_names_the_symbol_of_an_instruction() {
    let library = check(
        "case.symbols",
        "#unrestricted\nSymbol is Putchar or Labs\n\nSymbol has\n    to_string(self): when self is\n        Putchar then 'putchar'\n        Labs then 'labs'\n",
        &[],
    )
    .unwrap();
    let client = "\
use case.symbols Symbol
#unrestricted
Word is in 0 to 255
#unrestricted
One has
    #register('x0')
    value Word
#instruction('bl $target')
call(target, input Type, output Type, inputs input) output
put(character Word) Word:
    after := call(Symbol.Putchar.to_string, One, One, One(value: character))
    return after.value
";
    let module = check("case.client", client, &[&library]).unwrap();
    let file = module.checked();
    let index = file.functions.iter().position(|f| f.name == "put").unwrap();
    let body = file.bodies[index].as_ref().expect("a recorded body");
    let symbols = body.expressions.iter().find_map(|expr| match &expr.kind {
        CheckedSourceBodyKind::Instruction { symbols, .. } => Some(symbols.clone()),
        _ => None,
    });
    assert_eq!(
        symbols,
        Some(vec![("target".to_owned(), "putchar".to_owned())])
    );
}

#[test]
fn an_imported_domain_brings_its_operators() {
    let library = check(
        "case.numbers",
        "\
Numeric has
    #infix('+')
    add(self, rhs Self):
        a in Self: self
        b in Self: rhs
        return a + b

#unrestricted
#wrap
Word is in 0 to 255
Word has Numeric
",
        &[],
    )
    .unwrap();
    let client = "use case.numbers Word\nsum(a Word, b Word) Word: return a + b\n";
    let checked = check("case.client", client, &[&library]);
    assert!(checked.is_ok(), "{:?}", checked.err());
}

/// Importing a function does not import the methods of the types it mentions: the type is
/// not named in the client, so there is no `Box` to call a method on.
#[test]
fn importing_a_function_alone_does_not_import_a_types_methods() {
    let library = check(
        "case.boxes",
        &format!("{BOX}make() Box: return Box(value: 4)\n"),
        &[],
    )
    .unwrap();
    let client =
        "use case.boxes make\nanswer() in 0 to 9:\n    held := make()\n    return held.get\n";
    assert!(check("case.client", client, &[&library]).is_err());
}

#[test]
fn an_imported_method_does_not_clash_with_a_local_function_of_another_name() {
    let library = library();
    let client =
        "use case.boxes Box\nget() in 0 to 9: return 1\nanswer() in 0 to 9: return get()\n";
    assert!(check("case.client", client, &[&library]).is_ok());
}

/// A type declared here and the imported one of the same name cannot both exist.
#[test]
fn a_local_type_of_an_imported_types_name_is_refused() {
    let library = library();
    let client = "use case.boxes Box\nBox has\n    other in 0 to 9\n";
    assert!(check("case.client", client, &[&library]).is_err());
}

/// A body folded in an importer names variants by the importer's numbering, not the
/// library's. The importer here declares types first, so the two numberings differ; a fold
/// that compared the library's numbers would pick the wrong arm and say so confidently.
#[test]
fn a_folded_match_uses_the_importers_numbering_of_the_variants() {
    let library = check(
        "case.modes",
        "\
#unrestricted
Mode is Loud or Quiet or Plain

letter(mode Mode) in 0 to 255: when mode is
    Loud then 76
    Quiet then 81
    Plain then 80
",
        &[],
    )
    .unwrap();
    let client = "\
Padding1 has
Padding2 has
Padding3 has
use case.modes Mode, letter

loud() in 0 to 255: return letter(Mode.Loud)
quiet() in 0 to 255: return letter(Mode.Quiet)
plain() in 0 to 255: return letter(Mode.Plain)
";
    let module = check("case.client", client, &[&library]).unwrap();
    let file = module.checked();
    for (function, expected) in [("loud", 76), ("quiet", 81), ("plain", 80)] {
        let index = file
            .functions
            .iter()
            .position(|f| f.name == function)
            .unwrap();
        let body = file.bodies[index].as_ref().expect("a recorded body");
        let folded: Vec<_> = body
            .expressions
            .iter()
            .filter_map(|expr| match &expr.kind {
                CheckedSourceBodyKind::Integer(value) => Some(*value),
                _ => None,
            })
            .collect();
        assert!(
            folded.contains(&expected.into()),
            "{function}: expected {expected}, found {folded:?}"
        );
    }
}

/// With one type ahead of the import the numberings overlap: the importer's `Loud` carries
/// the number the library gave `Quiet`. Unremapped, the fold would answer 81 for `Loud`.
#[test]
fn overlapping_numberings_do_not_fold_to_the_wrong_arm() {
    let library = check(
        "case.modes",
        "#unrestricted\nMode is Loud or Quiet\n\nletter(mode Mode) in 0 to 255: when mode is\n    Loud then 76\n    Quiet then 81\n",
        &[],
    )
    .unwrap();
    let client = "Padding has\nuse case.modes Mode, letter\n\nloud() in 0 to 255: return letter(Mode.Loud)\nquiet() in 0 to 255: return letter(Mode.Quiet)\n";
    let module = check("case.client", client, &[&library]).unwrap();
    let file = module.checked();
    let folded = |function: &str| -> Vec<_> {
        let index = file
            .functions
            .iter()
            .position(|f| f.name == function)
            .unwrap();
        let body = file.bodies[index].as_ref().expect("a recorded body");
        body.expressions
            .iter()
            .filter_map(|expr| match &expr.kind {
                CheckedSourceBodyKind::Integer(value) => Some(*value),
                _ => None,
            })
            .collect()
    };
    assert!(
        !folded("loud").contains(&81.into()),
        "Loud folded to Quiet's arm: {:?}",
        folded("loud")
    );
    assert!(
        !folded("quiet").contains(&76.into()),
        "Quiet folded to Loud's arm: {:?}",
        folded("quiet")
    );
    assert!(folded("loud").contains(&76.into()), "{:?}", folded("loud"));
    assert!(
        folded("quiet").contains(&81.into()),
        "{:?}",
        folded("quiet")
    );
}
